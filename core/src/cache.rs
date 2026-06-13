//! Phase 7 — Change Detection.
//!
//! Computes a content digest over a stage's resolved `inputs` and persists it,
//! together with the stage's declared output paths, to a per-project cache at
//! `.mainstage/cache.json`. On a later run, a stage is skipped when its input
//! digest is unchanged *and* all of its declared outputs still exist.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Diagnostic, Error, Result};
use crate::eval::Value;

/// Directory (relative to the project root) that holds Mainstage's local state.
pub const CACHE_DIR: &str = ".mainstage";
/// Cache file name within [`CACHE_DIR`].
pub const CACHE_FILE: &str = "cache.json";

/// The on-disk change-detection cache: one entry per stage, keyed by stage name.
///
/// A `BTreeMap` keeps the serialized form stable (sorted keys) so the cache file
/// produces minimal diffs between runs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    stages: BTreeMap<String, StageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StageEntry {
    /// Combined SHA-256 digest (hex) of all input files at the last successful run.
    input_digest: String,
    /// Declared output paths recorded at the last successful run.
    outputs: Vec<String>,
    /// Per-input-file fingerprints (size + mtime + content hash), keyed by path, used to
    /// skip re-hashing a file whose size and mtime are unchanged. Optional and
    /// defaulted, so a cache written before Phase 25 (no `files` field) still loads —
    /// such an entry simply re-hashes every file once, then records the metadata.
    #[serde(default)]
    files: BTreeMap<String, FileMeta>,
}

/// A single input file's fast-path fingerprint. When a file's `size` and modification
/// time match a recorded entry, its `hash` is reused instead of re-reading the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileMeta {
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    /// SHA-256 (hex) of the file contents at the time it was recorded.
    hash: String,
}

impl Cache {
    /// Load the cache for the project rooted at `project_dir`. A missing, unreadable,
    /// or corrupt cache yields an empty cache rather than an error — change detection
    /// degrades to "always run".
    pub fn load(project_dir: &Path) -> Self {
        let path = project_dir.join(CACHE_DIR).join(CACHE_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Cache::default(),
        }
    }

    /// Persist the cache to `<project_dir>/.mainstage/cache.json`, creating the
    /// directory if needed.
    ///
    /// The write is atomic: the JSON is written to a uniquely-named temporary file in
    /// the same directory and then renamed over the target, so an interrupted run
    /// (Ctrl-C / crash mid-write) never leaves a truncated or corrupt `cache.json` —
    /// either the old file or the fully-written new one is present.
    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let dir = project_dir.join(CACHE_DIR);
        std::fs::create_dir_all(&dir)
            .map_err(|e| cache_err(format!("create cache dir '{}': {}", dir.display(), e)))?;
        let path = dir.join(CACHE_FILE);
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| cache_err(format!("serialize cache: {}", e)))?;

        // Unique temp name so concurrent saves in the same directory never clash.
        let tmp = dir.join(format!("{}.{}.tmp", CACHE_FILE, uuid::Uuid::new_v4()));
        std::fs::write(&tmp, text)
            .map_err(|e| cache_err(format!("write cache '{}': {}", tmp.display(), e)))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            // Best-effort cleanup of the temp file if the atomic replace failed.
            let _ = std::fs::remove_file(&tmp);
            cache_err(format!("replace cache '{}': {}", path.display(), e))
        })
    }

    /// Return `true` when `stage` can be skipped: its recorded digest equals
    /// `input_digest` and every recorded output path still exists on disk.
    pub fn is_fresh(&self, stage: &str, input_digest: &str, project_dir: &Path) -> bool {
        match self.stages.get(stage) {
            Some(entry) if entry.input_digest == input_digest => {
                entry.outputs.iter().all(|o| output_exists(o, project_dir))
            }
            _ => false,
        }
    }

    /// Record (or replace) the cache entry for `stage` after a successful run, without
    /// per-file fast-path metadata. Retained for callers that hash via [`input_digest`].
    pub fn update(&mut self, stage: &str, input_digest: String, outputs: Vec<String>) {
        self.stages.insert(
            stage.to_string(),
            StageEntry { input_digest, outputs, files: BTreeMap::new() },
        );
    }

    /// Snapshot the per-file fast-path metadata recorded for `stage` on its last run.
    /// Empty when the stage has no entry or the entry predates Phase 25.
    pub fn input_meta(&self, stage: &str) -> InputMeta {
        InputMeta(self.stages.get(stage).map(|e| e.files.clone()).unwrap_or_default())
    }

    /// Record (or replace) the cache entry for `stage` from a computed
    /// [`InputFingerprint`], persisting its per-file metadata for the next run.
    pub fn update_fingerprint(&mut self, stage: &str, fp: InputFingerprint, outputs: Vec<String>) {
        self.stages.insert(
            stage.to_string(),
            StageEntry { input_digest: fp.digest, outputs, files: fp.files },
        );
    }
}

/// Delete the change-detection cache for the project rooted at `project_dir`.
/// Succeeds as a no-op when no cache directory is present.
pub fn clean(project_dir: &Path) -> Result<()> {
    let dir = project_dir.join(CACHE_DIR);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| cache_err(format!("remove cache dir '{}': {}", dir.display(), e)))?;
    }
    Ok(())
}

/// Compute the combined SHA-256 digest of every file referenced by `value`.
///
/// Paths are gathered from `FileSet` entries and from any plain string values
/// (treated as paths), then sorted so the digest is independent of iteration
/// order. Each file contributes its path plus the SHA-256 of its contents; a
/// file that cannot be read contributes a sentinel, so a file appearing or
/// disappearing changes the digest.
pub fn input_digest(value: &Value) -> String {
    let mut paths = Vec::new();
    collect_paths(value, &mut paths);
    paths.sort();
    paths.dedup();

    let mut outer = Sha256::new();
    for p in &paths {
        outer.update(p.to_string_lossy().as_bytes());
        outer.update([0u8]);
        match std::fs::read(p) {
            Ok(bytes) => outer.update(Sha256::digest(&bytes)),
            Err(_) => outer.update(b"<missing>"),
        }
        outer.update([0u8]);
    }
    hex(&outer.finalize())
}

/// SHA-256 of `bytes` as a lowercase hex string.
///
/// Shared with the `hash` standard-library module so script-level hashing uses the
/// same algorithm and hex encoding as change detection.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

// ── Fast-path fingerprinting (Phase 25) ───────────────────────────────────────────

/// A within-run cache of file fingerprints so a file referenced by several stages'
/// inputs is read and hashed at most once per run. Synchronized so concurrently
/// executing stages (Phase 24) can share it.
#[derive(Default)]
pub struct RunFileCache(Mutex<HashMap<PathBuf, FileMeta>>);

impl RunFileCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A snapshot of a stage's per-file fast-path metadata from its previous run.
#[derive(Default)]
pub struct InputMeta(BTreeMap<String, FileMeta>);

/// The fingerprint of a stage's resolved inputs: the combined digest used for the
/// skip-check, plus the per-file metadata to persist for the next run's fast path.
pub struct InputFingerprint {
    digest: String,
    files: BTreeMap<String, FileMeta>,
}

impl InputFingerprint {
    /// The combined input digest, comparable against a cached entry via
    /// [`Cache::is_fresh`].
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Fingerprint a stage's resolved `inputs`, producing the same combined digest as
/// [`input_digest`] but reusing each file's content hash whenever its size and mtime are
/// unchanged — first from `run` (this run), then from `prior` (the last run) — so an
/// unchanged file is never re-read. Files that still require hashing are read in
/// parallel.
pub fn fingerprint_inputs(
    value: &Value,
    prior: &InputMeta,
    run: &RunFileCache,
) -> InputFingerprint {
    let mut paths = Vec::new();
    collect_paths(value, &mut paths);
    paths.sort();
    paths.dedup();

    // Pass 1: stat each path and decide whether its hash can be reused (fast path) or
    // the file must be read. Stat is cheap; reading + hashing is what we avoid.
    let mut resolved: Vec<Option<FileMeta>> = vec![None; paths.len()];
    let mut to_hash: Vec<(usize, u64, u64, u32)> = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        let Ok(md) = std::fs::metadata(p) else {
            continue; // missing/unreadable → contributes the `<missing>` sentinel
        };
        if md.is_dir() {
            continue; // directories carry no content hash
        }
        let size = md.len();
        let (secs, nanos) = mtime_parts(&md);

        // Reuse from the within-run cache, then from the prior run's record.
        let run_hit = run.0.lock().unwrap().get(p).cloned();
        if let Some(m) = run_hit.filter(|m| m.matches(size, secs, nanos)) {
            resolved[i] = Some(m);
            continue;
        }
        if let Some(m) =
            prior.0.get(p.to_string_lossy().as_ref()).filter(|m| m.matches(size, secs, nanos))
        {
            let m = m.clone();
            run.0.lock().unwrap().insert(p.clone(), m.clone());
            resolved[i] = Some(m);
            continue;
        }
        to_hash.push((i, size, secs, nanos));
    }

    // Pass 2: read + hash the cache-miss files, in parallel for larger sets.
    for (i, meta) in parallel_hash(&paths, &to_hash) {
        if let Some(m) = &meta {
            run.0.lock().unwrap().insert(paths[i].clone(), m.clone());
        }
        resolved[i] = meta;
    }

    // Pass 3: build the combined digest (byte-for-byte compatible with `input_digest`)
    // and collect the metadata to persist.
    let mut files = BTreeMap::new();
    let mut outer = Sha256::new();
    for (i, p) in paths.iter().enumerate() {
        outer.update(p.to_string_lossy().as_bytes());
        outer.update([0u8]);
        match &resolved[i] {
            Some(meta) => {
                outer.update(hex_to_bytes(&meta.hash));
                files.insert(p.to_string_lossy().into_owned(), meta.clone());
            }
            None => outer.update(b"<missing>"),
        }
        outer.update([0u8]);
    }
    InputFingerprint { digest: hex(&outer.finalize()), files }
}

impl FileMeta {
    /// Whether a freshly-stat'd file matches this recorded fingerprint's size and mtime.
    fn matches(&self, size: u64, secs: u64, nanos: u32) -> bool {
        self.size == size && self.mtime_secs == secs && self.mtime_nanos == nanos
    }
}

/// Read and hash the cache-miss files. Uses scoped worker threads for sets large
/// enough to benefit; small sets hash inline to avoid thread overhead.
fn parallel_hash(
    paths: &[PathBuf],
    to_hash: &[(usize, u64, u64, u32)],
) -> Vec<(usize, Option<FileMeta>)> {
    fn hash_one(
        paths: &[PathBuf],
        &(i, size, secs, nanos): &(usize, u64, u64, u32),
    ) -> (usize, Option<FileMeta>) {
        match std::fs::read(&paths[i]) {
            Ok(bytes) => (
                i,
                Some(FileMeta {
                    size,
                    mtime_secs: secs,
                    mtime_nanos: nanos,
                    hash: sha256_hex(&bytes),
                }),
            ),
            Err(_) => (i, None),
        }
    }

    let workers =
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(to_hash.len());
    if workers <= 1 {
        return to_hash.iter().map(|t| hash_one(paths, t)).collect();
    }

    let chunk = to_hash.len().div_ceil(workers);
    let mut out = Vec::with_capacity(to_hash.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = to_hash
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(|t| hash_one(paths, t)).collect::<Vec<_>>()))
            .collect();
        for h in handles {
            out.extend(h.join().unwrap());
        }
    });
    out
}

/// A file's modification time split into whole seconds and sub-second nanoseconds since
/// the Unix epoch. Falls back to `(0, 0)` when the platform cannot report it, in which
/// case the fast path simply never matches and the file is hashed.
fn mtime_parts(md: &std::fs::Metadata) -> (u64, u32) {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0))
}

/// Decode a lowercase hex string back to its bytes (inverse of [`hex`]).
fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

/// Collect the declared output paths from an evaluated `outputs` value.
pub fn output_paths(value: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_paths(value, &mut paths);
    paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

// ── Helpers ──────────────────────────────────────────────────────────────────────

fn collect_paths(value: &Value, out: &mut Vec<PathBuf>) {
    match value {
        Value::FileSet(entries) => out.extend(entries.iter().map(|e| e.path.clone())),
        Value::List(items) => items.iter().for_each(|v| collect_paths(v, out)),
        Value::String(s) => out.push(PathBuf::from(s)),
        // Ints and bools are not paths — they never appear in an `outputs` position.
        Value::Int(_) | Value::Bool(_) => {}
    }
}

/// An output is present if its path exists as written, or relative to the project
/// root — covering both absolute paths and project-relative declarations.
fn output_exists(output: &str, project_dir: &Path) -> bool {
    let p = Path::new(output);
    p.exists() || project_dir.join(p).exists()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn cache_err(msg: impl Into<String>) -> Error {
    Error::Eval(vec![Diagnostic::new(msg)])
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::FileEntry;

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_cache_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fileset(paths: &[PathBuf]) -> Value {
        Value::FileSet(paths.iter().cloned().map(FileEntry::from_path).collect())
    }

    #[test]
    fn digest_is_stable_and_order_independent() {
        let dir = unique_dir("digest");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::write(&a, "alpha").unwrap();
        std::fs::write(&b, "beta").unwrap();

        let d1 = input_digest(&fileset(&[a.clone(), b.clone()]));
        let d2 = input_digest(&fileset(&[b.clone(), a.clone()]));
        assert_eq!(d1, d2, "digest must not depend on ordering");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digest_changes_with_content() {
        let dir = unique_dir("content");
        let a = dir.join("a");
        std::fs::write(&a, "one").unwrap();
        let before = input_digest(&fileset(std::slice::from_ref(&a)));
        std::fs::write(&a, "two").unwrap();
        let after = input_digest(&fileset(std::slice::from_ref(&a)));
        assert_ne!(before, after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip_and_freshness() {
        let dir = unique_dir("roundtrip");
        let out = dir.join("out.bin");
        std::fs::write(&out, "x").unwrap();

        let mut cache = Cache::default();
        cache.update("build", "abc123".to_string(), vec![out.display().to_string()]);
        cache.save(&dir).unwrap();

        let loaded = Cache::load(&dir);
        assert!(loaded.is_fresh("build", "abc123", &dir));
        assert!(!loaded.is_fresh("build", "different", &dir), "digest mismatch is not fresh");

        // Missing output invalidates freshness.
        std::fs::remove_file(&out).unwrap();
        assert!(!loaded.is_fresh("build", "abc123", &dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_files() {
        // The save writes to a temp file then renames, so the directory ends with exactly
        // the cache file and no `.tmp` residue — guaranteeing an interrupted run never
        // leaves a half-written cache behind.
        let dir = unique_dir("atomic");

        let mut cache = Cache::default();
        cache.update("a", "d1".to_string(), vec![]);
        cache.save(&dir).unwrap();
        // Overwrite to ensure rename-over-existing works.
        cache.update("b", "d2".to_string(), vec![]);
        cache.save(&dir).unwrap();

        let cache_dir = dir.join(CACHE_DIR);
        let entries: Vec<String> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec![CACHE_FILE.to_string()], "only the cache file should remain");

        let reloaded = Cache::load(&dir);
        assert!(reloaded.is_fresh("a", "d1", &dir));
        assert!(reloaded.is_fresh("b", "d2", &dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_removes_cache() {
        let dir = unique_dir("clean");
        Cache::default().save(&dir).unwrap();
        assert!(dir.join(CACHE_DIR).exists());
        clean(&dir).unwrap();
        assert!(!dir.join(CACHE_DIR).exists());
        // Cleaning again is a no-op.
        clean(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_paths_flatten_list() {
        let value =
            Value::List(vec![Value::String("a/x".to_string()), Value::String("b/y".to_string())]);
        assert_eq!(output_paths(&value), vec!["a/x".to_string(), "b/y".to_string()]);
    }

    // ── Phase 25: fast-path fingerprinting ──────────────────────────────────────

    #[test]
    fn fingerprint_digest_matches_legacy_input_digest() {
        // The fast-path digest must be byte-for-byte identical to `input_digest` so an
        // existing cache (written by the legacy path) stays valid after the upgrade.
        let dir = unique_dir("fp_compat");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::write(&a, "alpha").unwrap();
        std::fs::write(&b, "beta").unwrap();
        let value = fileset(&[a.clone(), b.clone()]);

        let legacy = input_digest(&value);
        let fp = fingerprint_inputs(&value, &InputMeta::default(), &RunFileCache::new());
        assert_eq!(fp.digest(), legacy);

        // A missing file is handled identically by both paths.
        std::fs::remove_file(&b).unwrap();
        let legacy_missing = input_digest(&value);
        let fp_missing = fingerprint_inputs(&value, &InputMeta::default(), &RunFileCache::new());
        assert_eq!(fp_missing.digest(), legacy_missing);
        assert_ne!(fp_missing.digest(), fp.digest());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fast_path_reuses_unchanged_and_detects_changes() {
        let dir = unique_dir("fp_fast");
        let a = dir.join("a");
        std::fs::write(&a, "one").unwrap();
        let value = fileset(std::slice::from_ref(&a));

        // First run records per-file metadata in the cache.
        let mut cache = Cache::default();
        let fp1 = fingerprint_inputs(&value, &cache.input_meta("gen"), &RunFileCache::new());
        cache.update_fingerprint("gen", fp1, vec![]);

        // A later run with the same content takes the fast path; the digest is unchanged.
        let prior = cache.input_meta("gen");
        let fp2 = fingerprint_inputs(&value, &prior, &RunFileCache::new());
        assert!(cache.is_fresh("gen", fp2.digest(), &dir), "unchanged inputs must be fresh");

        // Changing the content to a different size forces a re-hash and a new digest,
        // regardless of mtime resolution.
        std::fs::write(&a, "changed-content").unwrap();
        let fp3 = fingerprint_inputs(&value, &cache.input_meta("gen"), &RunFileCache::new());
        assert!(!cache.is_fresh("gen", fp3.digest(), &dir), "changed inputs must not be fresh");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_cache_shares_fingerprints_across_stages() {
        // Two fingerprints in the same run, sharing a file, must agree — the second
        // resolves the shared file from the within-run cache.
        let dir = unique_dir("fp_run");
        let a = dir.join("a");
        std::fs::write(&a, "shared").unwrap();
        let value = fileset(std::slice::from_ref(&a));

        let run = RunFileCache::new();
        let d1 = fingerprint_inputs(&value, &InputMeta::default(), &run).digest().to_string();
        let d2 = fingerprint_inputs(&value, &InputMeta::default(), &run).digest().to_string();
        assert_eq!(d1, d2);
        assert_eq!(d1, input_digest(&value));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_cache_without_files_field_loads_and_is_fresh() {
        // A cache.json written before Phase 25 has no `files` field; it must still load
        // and remain usable (an empty `input_meta`, freshness by digest as before).
        let dir = unique_dir("fp_legacy");
        let cache_dir = dir.join(CACHE_DIR);
        std::fs::create_dir_all(&cache_dir).unwrap();
        let out = dir.join("out.bin");
        std::fs::write(&out, "x").unwrap();
        let json = format!(
            r#"{{ "stages": {{ "build": {{ "input_digest": "abc123", "outputs": ["{}"] }} }} }}"#,
            out.display().to_string().replace('\\', "/")
        );
        std::fs::write(cache_dir.join(CACHE_FILE), json).unwrap();

        let loaded = Cache::load(&dir);
        assert!(loaded.is_fresh("build", "abc123", &dir), "legacy entry stays fresh by digest");
        assert!(loaded.input_meta("build").0.is_empty(), "no per-file metadata yet");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
