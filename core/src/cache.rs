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
/// Sub-directory of [`CACHE_DIR`] holding the content-addressed output store (Phase 50):
/// `<project>/.mainstage/cache/<digest>` is the blob for the file whose contents hash to
/// `<digest>`. Co-located with the change-detection cache so `mainstage clean` clears both.
pub const CAS_DIR: &str = "cache";

/// Absolute path to the content-addressed store for the project rooted at `project_dir`.
fn cas_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(CACHE_DIR).join(CAS_DIR)
}

/// The on-disk change-detection cache: one entry per stage, keyed by stage name.
///
/// A `BTreeMap` keeps the serialized form stable (sorted keys) so the cache file
/// produces minimal diffs between runs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    stages: BTreeMap<String, StageEntry>,
    /// Cumulative output-cache (CAS) restore counters (Phase 50), surfaced by
    /// `mainstage cache stats`. `cas_hits` counts stages whose missing outputs were
    /// restored from the store instead of rebuilt; `cas_misses` counts restores that could
    /// not complete because a referenced blob was absent (and so fell back to a rebuild).
    /// Defaulted, so a cache written before Phase 50 still loads.
    #[serde(default)]
    cas_hits: u64,
    #[serde(default)]
    cas_misses: u64,
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
    /// Content-addressed snapshot of this stage's declared outputs at its last successful
    /// run (Phase 50): one [`OutputRecord`] per declared output path, each mapping its
    /// constituent files to their CAS blob digests. Lets a later run with matching inputs
    /// *restore* missing outputs from the store rather than re-running the stage. Defaulted,
    /// so a cache written before Phase 50 (no `output_records`) still loads — such a stage
    /// simply has nothing to restore from and falls back to a rebuild when its outputs go
    /// missing.
    #[serde(default)]
    output_records: Vec<OutputRecord>,
}

/// The content-addressed snapshot of a single declared output path (Phase 50). A file
/// output has exactly one [`OutputFile`] with an empty `rel`; a directory output has one
/// per regular file beneath it, each `rel` being the file's path relative to the output
/// root. Restoring re-creates every file from its blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputRecord {
    /// The declared output path, exactly as recorded (e.g. `"dist"` or `"bin/app"`).
    path: String,
    files: Vec<OutputFile>,
}

/// One regular file within an [`OutputRecord`]: where it sits under the output root and
/// the CAS blob digest of its contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputFile {
    /// Path relative to the output root, using `/` separators. Empty when the output is
    /// itself a single file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    rel: String,
    /// SHA-256 (hex) of the file's contents — the blob's name in the CAS.
    digest: String,
    /// Unix permission bits, so an executable output survives a restore. `0` (the default,
    /// omitted from the serialized form) means "use the umask default", e.g. on Windows.
    #[serde(default, skip_serializing_if = "is_zero")]
    mode: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
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

    /// Classify `stage` against `input_digest` for the Phase 50 restore path: a matching
    /// digest with all outputs present is [`Freshness::Fresh`] (skip); a matching digest
    /// whose outputs are missing but were snapshotted into the CAS is
    /// [`Freshness::Restorable`] (restore instead of rebuild); anything else is
    /// [`Freshness::Stale`] (rebuild). Subsumes [`is_fresh`](Self::is_fresh) — the runner
    /// uses this richer form, while the planner still uses `is_fresh`.
    pub fn freshness(&self, stage: &str, input_digest: &str, project_dir: &Path) -> Freshness {
        match self.stages.get(stage) {
            Some(entry) if entry.input_digest == input_digest => {
                if entry.outputs.iter().all(|o| output_exists(o, project_dir)) {
                    Freshness::Fresh
                } else if !entry.output_records.is_empty() {
                    Freshness::Restorable(OutputSnapshot(entry.output_records.clone()))
                } else {
                    Freshness::Stale
                }
            }
            _ => Freshness::Stale,
        }
    }

    /// Attach a content-addressed output `snapshot` to `stage`'s entry after its outputs
    /// have been stored in the CAS (Phase 50). A no-op when the stage has no entry yet (it
    /// must be recorded via [`update_fingerprint`](Self::update_fingerprint) first).
    pub fn set_output_records(&mut self, stage: &str, snapshot: OutputSnapshot) {
        if let Some(entry) = self.stages.get_mut(stage) {
            entry.output_records = snapshot.0;
        }
    }

    /// Note that a stage's outputs were restored from the CAS (a cache hit).
    pub fn note_cas_hit(&mut self) {
        self.cas_hits += 1;
    }

    /// Note that a restore was attempted but a referenced blob was absent (a cache miss).
    pub fn note_cas_miss(&mut self) {
        self.cas_misses += 1;
    }

    /// Record (or replace) the cache entry for `stage` after a successful run, without
    /// per-file fast-path metadata. Retained for callers that hash via [`input_digest`].
    pub fn update(&mut self, stage: &str, input_digest: String, outputs: Vec<String>) {
        self.stages.insert(
            stage.to_string(),
            StageEntry {
                input_digest,
                outputs,
                files: BTreeMap::new(),
                output_records: Vec::new(),
            },
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
            StageEntry {
                input_digest: fp.digest,
                outputs,
                files: fp.files,
                // The caller attaches the content-addressed snapshot separately, via
                // `set_output_records`, after the outputs are stored in the CAS.
                output_records: Vec::new(),
            },
        );
    }
}

/// The result of classifying a stage against the cache for the restore path (Phase 50).
#[derive(Debug)]
pub enum Freshness {
    /// Inputs changed (or the stage is uncached): the stage must run.
    Stale,
    /// Inputs unchanged and outputs all present: the stage can be skipped.
    Fresh,
    /// Inputs unchanged but some outputs are missing, and a content-addressed snapshot of
    /// them exists: restore from the CAS instead of rebuilding. Carries the snapshot to hand
    /// to [`restore_outputs`].
    Restorable(OutputSnapshot),
}

/// An opaque, content-addressed snapshot of a stage's declared outputs (Phase 50). Produced
/// by [`store_outputs`], carried by [`Freshness::Restorable`], consumed by
/// [`restore_outputs`] and [`Cache::set_output_records`]. The inner records are private so
/// the on-disk schema can evolve without breaking callers.
#[derive(Debug)]
pub struct OutputSnapshot(Vec<OutputRecord>);

impl OutputSnapshot {
    /// Whether the snapshot recorded no files — nothing to persist or restore.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
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

    /// The input paths whose content is unchanged versus `prior` — same recorded content
    /// hash. Drives per-file incremental change detection (Phase 38): an unchanged input
    /// file's `for`-loop iteration can be skipped because its output is already present
    /// and current. A path absent from `prior` (newly added) is treated as changed.
    pub fn unchanged_since(&self, prior: &InputMeta) -> std::collections::HashSet<PathBuf> {
        self.files
            .iter()
            .filter(|(path, meta)| prior.0.get(*path).is_some_and(|p| p.hash == meta.hash))
            .map(|(path, _)| PathBuf::from(path))
            .collect()
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

/// Collect the resolved file paths referenced by an evaluated `inputs` value, in the
/// same way change detection gathers them. Used by `--dry-run` and `watch` to learn
/// which files a stage reads.
pub fn input_paths(value: &Value) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_paths(value, &mut paths);
    paths
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

/// Whether every declared output path in `outputs` currently exists on disk. Used to gate
/// per-file incremental change detection: an unchanged input's output is only safe to
/// reuse when the stage's declared outputs are all still present.
pub fn all_outputs_exist(outputs: &[String], project_dir: &Path) -> bool {
    outputs.iter().all(|o| output_exists(o, project_dir))
}

/// An output is present if its path exists as written, or relative to the project
/// root — covering both absolute paths and project-relative declarations.
fn output_exists(output: &str, project_dir: &Path) -> bool {
    let p = Path::new(output);
    p.exists() || project_dir.join(p).exists()
}

// ── Content-addressed output store (Phase 50) ──────────────────────────────────────

/// Resolve a declared output path to the on-disk location that currently exists — the path
/// as written, or relative to the project root — or `None` when neither is present.
fn existing_output_path(output: &str, project_dir: &Path) -> Option<PathBuf> {
    let p = Path::new(output);
    if p.exists() {
        Some(p.to_path_buf())
    } else {
        let joined = project_dir.join(p);
        joined.exists().then_some(joined)
    }
}

/// Resolve a declared output path to the location a restore should write it to: the path
/// itself when absolute, otherwise relative to the project root (mirroring how the runner
/// produces outputs).
fn output_dest(output: &str, project_dir: &Path) -> PathBuf {
    let p = Path::new(output);
    if p.is_absolute() { p.to_path_buf() } else { project_dir.join(p) }
}

/// The unix permission bits of `md` (`0` on platforms without them, so the field is omitted).
fn file_mode(md: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        md.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        0
    }
}

/// Recursively list every regular file beneath `base`, each as `(rel, abs, mode)` where
/// `rel` is the `/`-joined path relative to `base` (empty when `base` is itself a file).
fn walk_output_files(base: &Path) -> Vec<(String, PathBuf, u32)> {
    fn recurse(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf, u32)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            match std::fs::symlink_metadata(&path) {
                Ok(md) if md.is_dir() => recurse(&path, &rel, out),
                Ok(md) if md.is_file() => out.push((rel, path, file_mode(&md))),
                // Symlinks and other special files are skipped — the output cache stores
                // plain file contents only.
                _ => {}
            }
        }
    }

    match std::fs::symlink_metadata(base) {
        Ok(md) if md.is_file() => vec![(String::new(), base.to_path_buf(), file_mode(&md))],
        Ok(md) if md.is_dir() => {
            let mut out = Vec::new();
            recurse(base, "", &mut out);
            out
        }
        _ => Vec::new(),
    }
}

/// Snapshot `output_paths` into the content-addressed store under `<project>/.mainstage/cache/`
/// (Phase 50), returning the records to attach to the stage's cache entry. Every regular file
/// of every existing output is hashed (in parallel) and its contents written to the store
/// keyed by digest; an already-present blob is left untouched (content-addressed dedup).
///
/// Best-effort and all-or-nothing: if any blob cannot be written, an **empty** snapshot is
/// returned so the stage is simply not restorable (and always rebuilds) rather than being
/// recorded as restorable from an incomplete set. A stage with no existing outputs likewise
/// yields an empty snapshot.
pub fn store_outputs(project_dir: &Path, output_paths: &[String]) -> OutputSnapshot {
    let store = cas_dir(project_dir);
    if std::fs::create_dir_all(&store).is_err() {
        return OutputSnapshot(Vec::new());
    }

    // Gather (output index, rel, abs path, mode) for every file across all outputs.
    let mut records: Vec<OutputRecord> = Vec::new();
    let mut jobs: Vec<(usize, String, PathBuf, u32)> = Vec::new();
    for output in output_paths {
        let Some(base) = existing_output_path(output, project_dir) else {
            continue; // a declared-but-unproduced output simply isn't stored
        };
        let idx = records.len();
        records.push(OutputRecord { path: output.clone(), files: Vec::new() });
        for (rel, abs, mode) in walk_output_files(&base) {
            jobs.push((idx, rel, abs, mode));
        }
    }
    if records.is_empty() {
        return OutputSnapshot(Vec::new());
    }

    // Hash every file's contents in parallel, then write each blob to the store.
    let hashed = parallel_read_hash(&jobs);
    for ((idx, rel, _abs, mode), bytes) in jobs.iter().zip(hashed) {
        let Some(bytes) = bytes else {
            return OutputSnapshot(Vec::new()); // unreadable file → nothing is restorable
        };
        let digest = sha256_hex(&bytes);
        if write_blob(&store, &digest, &bytes).is_err() {
            return OutputSnapshot(Vec::new());
        }
        records[*idx].files.push(OutputFile { rel: rel.clone(), digest, mode: *mode });
    }
    OutputSnapshot(records)
}

/// Read and SHA-256 each file in `jobs`, in parallel for larger sets. A file that cannot be
/// read yields `None`. Returns the raw bytes (not just the hash) so the caller can both key
/// and write the blob without a second read.
fn parallel_read_hash(jobs: &[(usize, String, PathBuf, u32)]) -> Vec<Option<Vec<u8>>> {
    fn read_one(job: &(usize, String, PathBuf, u32)) -> Option<Vec<u8>> {
        std::fs::read(&job.2).ok()
    }
    let workers =
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(jobs.len().max(1));
    if workers <= 1 || jobs.len() <= 1 {
        return jobs.iter().map(read_one).collect();
    }
    let chunk = jobs.len().div_ceil(workers);
    let mut out = Vec::with_capacity(jobs.len());
    std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(read_one).collect::<Vec<_>>()))
            .collect();
        for h in handles {
            out.extend(h.join().unwrap());
        }
    });
    out
}

/// Write `bytes` to the store as the blob named `digest`, atomically and only if absent.
/// A present blob (same digest ⇒ same contents) is left as-is and its mtime touched so the
/// LRU eviction in [`gc`] treats it as recently used.
fn write_blob(store: &Path, digest: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = store.join(digest);
    if path.exists() {
        touch(&path);
        return Ok(());
    }
    let tmp = store.join(format!("{digest}.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Restore every file in `snapshot` from the content-addressed store (Phase 50). Returns
/// `true` once all files are written; returns `false` without touching the tree when any
/// referenced blob is absent, so the caller can fall back to a full rebuild. Restoring is
/// idempotent and creates parent directories as needed.
pub fn restore_outputs(project_dir: &Path, snapshot: &OutputSnapshot) -> bool {
    let store = cas_dir(project_dir);

    // Verify every referenced blob is present before writing anything, so a missing blob
    // never leaves the tree half-restored.
    for record in &snapshot.0 {
        for file in &record.files {
            if !store.join(&file.digest).exists() {
                return false;
            }
        }
    }

    for record in &snapshot.0 {
        let base = output_dest(&record.path, project_dir);
        for file in &record.files {
            let dest =
                if file.rel.is_empty() { base.clone() } else { base.join(rel_to_path(&file.rel)) };
            if let Some(parent) = dest.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                return false;
            }
            let blob = store.join(&file.digest);
            if std::fs::copy(&blob, &dest).is_err() {
                return false;
            }
            apply_mode(&dest, file.mode);
            touch(&blob); // restored ⇒ recently used, for LRU
        }
    }
    true
}

/// Convert a `/`-separated stored relative path into a platform path.
fn rel_to_path(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

/// Apply stored unix permission bits to `path` (no-op when `mode` is `0` or off-unix).
fn apply_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    if mode != 0 {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

/// Best-effort bump of `path`'s modification time to now, so [`gc`]'s LRU eviction orders
/// it as recently used. Failures (e.g. a read-only store) are ignored.
fn touch(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

// ── Cache maintenance: stats & gc (Phase 50) ───────────────────────────────────────

/// Aggregate statistics about the content-addressed store, for `mainstage cache stats`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    /// Number of blobs currently in the store.
    pub blob_count: usize,
    /// Total size of all blobs, in bytes.
    pub total_bytes: u64,
    /// How many of those blobs are referenced by a recorded stage output.
    pub referenced: usize,
    /// Cumulative restores served from the store.
    pub hits: u64,
    /// Cumulative restores that failed because a referenced blob was absent.
    pub misses: u64,
}

/// Compute [`CacheStats`] for the project rooted at `project_dir`: walks the store for blob
/// count and size, and reads the cumulative hit/miss counters from the cache file.
pub fn stats(project_dir: &Path) -> CacheStats {
    let cache = Cache::load(project_dir);
    let referenced = cache.referenced_digests();
    let mut stats =
        CacheStats { hits: cache.cas_hits, misses: cache.cas_misses, ..Default::default() };
    for (name, size) in blob_sizes(&cas_dir(project_dir)) {
        stats.blob_count += 1;
        stats.total_bytes += size;
        if referenced.contains(&name) {
            stats.referenced += 1;
        }
    }
    stats
}

/// What a [`gc`] pass removed.
#[derive(Debug, Default, Clone, Copy)]
pub struct GcReport {
    /// Blobs deleted because no recorded output referenced them.
    pub pruned_count: usize,
    pub pruned_bytes: u64,
    /// Blobs deleted by LRU eviction to honor the size ceiling.
    pub evicted_count: usize,
    pub evicted_bytes: u64,
    /// Total store size after the pass.
    pub remaining_bytes: u64,
}

/// Garbage-collect the content-addressed store (Phase 50): first prune every blob no
/// recorded stage output references, then — when `max_bytes` is set and the store is still
/// over the ceiling — evict the least-recently-used remaining blobs until it fits. Returns a
/// [`GcReport`] of what was removed.
pub fn gc(project_dir: &Path, max_bytes: Option<u64>) -> Result<GcReport> {
    let store = cas_dir(project_dir);
    let mut report = GcReport::default();
    if !store.exists() {
        return Ok(report);
    }
    let referenced = Cache::load(project_dir).referenced_digests();

    // Pass 1: prune unreferenced blobs.
    let mut survivors: Vec<(String, PathBuf, u64, std::time::SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(&store)
        .map_err(|e| cache_err(format!("read cache store '{}': {}", store.display(), e)))?
        .flatten()
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip stray non-blob files (e.g. an interrupted `.tmp`); only hex digests are blobs.
        let Ok(md) = entry.metadata() else { continue };
        if !md.is_file() || name.contains('.') {
            continue;
        }
        let size = md.len();
        if referenced.contains(&name) {
            let mtime = md.modified().unwrap_or(std::time::UNIX_EPOCH);
            survivors.push((name, path, size, mtime));
        } else if std::fs::remove_file(&path).is_ok() {
            report.pruned_count += 1;
            report.pruned_bytes += size;
        }
    }

    // Pass 2: LRU eviction down to the ceiling, if one was given.
    let mut remaining: u64 = survivors.iter().map(|(_, _, size, _)| *size).sum();
    if let Some(max) = max_bytes
        && remaining > max
    {
        // Oldest modification time first — least recently stored or restored.
        survivors.sort_by_key(|(_, _, _, mtime)| *mtime);
        for (_, path, size, _) in &survivors {
            if remaining <= max {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                report.evicted_count += 1;
                report.evicted_bytes += size;
                remaining -= size;
            }
        }
    }
    report.remaining_bytes = remaining;
    Ok(report)
}

/// The `(blob-name, size)` of every blob in `store`, ignoring stray temp files.
fn blob_sizes(store: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = std::fs::read_dir(store) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let md = e.metadata().ok()?;
            (md.is_file() && !name.contains('.')).then_some((name, md.len()))
        })
        .collect()
}

impl Cache {
    /// Every CAS blob digest referenced by a recorded stage output across the whole cache.
    fn referenced_digests(&self) -> std::collections::HashSet<String> {
        self.stages
            .values()
            .flat_map(|s| &s.output_records)
            .flat_map(|r| &r.files)
            .map(|f| f.digest.clone())
            .collect()
    }
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
    fn unchanged_since_reports_only_byte_identical_inputs() {
        // Phase 38: after one file of a two-file input set changes, only the untouched
        // file is reported unchanged, so only its loop iteration can be skipped.
        let dir = unique_dir("incr_unchanged");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "two").unwrap();
        let value = fileset(&[a.clone(), b.clone()]);

        let mut cache = Cache::default();
        let fp1 = fingerprint_inputs(&value, &cache.input_meta("c"), &RunFileCache::new());
        cache.update_fingerprint("c", fp1, vec![]);

        // Change only `a`; `b` is byte-for-byte identical.
        std::fs::write(&a, "one-changed").unwrap();
        let prior = cache.input_meta("c");
        let fp2 = fingerprint_inputs(&value, &prior, &RunFileCache::new());
        let unchanged = fp2.unchanged_since(&prior);
        assert!(unchanged.contains(&b), "the untouched file is unchanged");
        assert!(!unchanged.contains(&a), "the edited file is changed");

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

    // ── Phase 50: content-addressed output cache ────────────────────────────────

    /// Record a stage with the given input digest and stored outputs, returning the cache.
    fn cache_with_stored(dir: &Path, stage: &str, digest: &str, outputs: &[String]) -> Cache {
        let mut cache = Cache::default();
        cache.update(stage, digest.to_string(), outputs.to_vec());
        let snapshot = store_outputs(dir, outputs);
        cache.set_output_records(stage, snapshot);
        cache
    }

    #[test]
    fn store_then_restore_a_file_output() {
        let dir = unique_dir("cas_file");
        let out = dir.join("dist").join("app.txt");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(&out, "artifact contents").unwrap();

        let outputs = vec!["dist/app.txt".to_string()];
        let cache = cache_with_stored(&dir, "build", "digestA", &outputs);

        // Outputs present ⇒ Fresh.
        assert!(matches!(cache.freshness("build", "digestA", &dir), Freshness::Fresh));

        // Delete the output, then a matching digest is Restorable from the CAS.
        std::fs::remove_dir_all(dir.join("dist")).unwrap();
        let snapshot = match cache.freshness("build", "digestA", &dir) {
            Freshness::Restorable(s) => s,
            other => panic!("expected Restorable, got {other:?}"),
        };
        assert!(restore_outputs(&dir, &snapshot));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "artifact contents");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_then_restore_a_directory_output() {
        let dir = unique_dir("cas_dir");
        let base = dir.join("dist");
        std::fs::create_dir_all(base.join("nested")).unwrap();
        std::fs::write(base.join("a.txt"), "one").unwrap();
        std::fs::write(base.join("nested").join("b.txt"), "two").unwrap();

        let outputs = vec!["dist".to_string()];
        let cache = cache_with_stored(&dir, "bundle", "d", &outputs);

        std::fs::remove_dir_all(&base).unwrap();
        let snapshot = match cache.freshness("bundle", "d", &dir) {
            Freshness::Restorable(s) => s,
            other => panic!("expected Restorable, got {other:?}"),
        };
        assert!(restore_outputs(&dir, &snapshot));
        assert_eq!(std::fs::read_to_string(base.join("a.txt")).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(base.join("nested").join("b.txt")).unwrap(), "two");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_fails_when_a_blob_is_missing() {
        let dir = unique_dir("cas_missing");
        let out = dir.join("out.bin");
        std::fs::write(&out, "data").unwrap();
        let outputs = vec!["out.bin".to_string()];
        let cache = cache_with_stored(&dir, "s", "d", &outputs);

        // Wipe the CAS, then a restore must fail (and leave the tree untouched).
        std::fs::remove_dir_all(cas_dir(&dir)).unwrap();
        std::fs::remove_file(&out).unwrap();
        let snapshot = match cache.freshness("s", "d", &dir) {
            Freshness::Restorable(s) => s,
            other => panic!("expected Restorable, got {other:?}"),
        };
        assert!(!restore_outputs(&dir, &snapshot), "a missing blob must make restore fail");
        assert!(!out.exists(), "a failed restore writes nothing");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn freshness_is_stale_without_stored_outputs() {
        // A pre-Phase-50 entry (no output_records) whose output is gone is Stale, not
        // Restorable — there is nothing to restore from.
        let dir = unique_dir("cas_legacy_stale");
        let mut cache = Cache::default();
        cache.update("s", "d".to_string(), vec!["gone.txt".to_string()]);
        assert!(matches!(cache.freshness("s", "d", &dir), Freshness::Stale));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gc_prunes_unreferenced_blobs_only() {
        let dir = unique_dir("cas_gc");
        std::fs::write(dir.join("out.bin"), "kept").unwrap();
        let cache = cache_with_stored(&dir, "s", "d", &["out.bin".to_string()]);
        cache.save(&dir).unwrap();

        // Drop a stray blob the cache does not reference.
        let store = cas_dir(&dir);
        std::fs::write(store.join(sha256_hex(b"orphan")), "orphan").unwrap();
        assert_eq!(stats(&dir).blob_count, 2);

        let report = gc(&dir, None).unwrap();
        assert_eq!(report.pruned_count, 1, "only the unreferenced blob is pruned");
        let after = stats(&dir);
        assert_eq!(after.blob_count, 1);
        assert_eq!(after.referenced, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gc_evicts_lru_to_honor_ceiling() {
        let dir = unique_dir("cas_lru");
        let store = cas_dir(&dir);
        std::fs::create_dir_all(&store).unwrap();
        // Two unreferenced-but-large blobs; an empty cache references neither, so a no-ceiling
        // gc would prune both. Use a ceiling instead to exercise eviction on survivors: make
        // them referenced by recording a stage that "produced" them.
        std::fs::write(dir.join("a.bin"), vec![b'a'; 100]).unwrap();
        std::fs::write(dir.join("b.bin"), vec![b'b'; 100]).unwrap();
        let mut cache = Cache::default();
        cache.update("s", "d".to_string(), vec!["a.bin".to_string(), "b.bin".to_string()]);
        let snapshot = store_outputs(&dir, &["a.bin".to_string(), "b.bin".to_string()]);
        cache.set_output_records("s", snapshot);
        cache.save(&dir).unwrap();

        // Make `a` the least-recently-used by back-dating its mtime.
        let a_blob = store.join(sha256_hex(&[b'a'; 100]));
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
        std::fs::OpenOptions::new().write(true).open(&a_blob).unwrap().set_modified(old).unwrap();

        // Ceiling of 150 bytes forces eviction of one 100-byte blob — the older `a`.
        let report = gc(&dir, Some(150)).unwrap();
        assert_eq!(report.evicted_count, 1);
        assert!(!a_blob.exists(), "the least-recently-used blob is evicted first");
        assert!(report.remaining_bytes <= 150);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stats_report_hit_and_miss_counters() {
        let dir = unique_dir("cas_stats");
        let mut cache = Cache::default();
        cache.note_cas_hit();
        cache.note_cas_hit();
        cache.note_cas_miss();
        cache.save(&dir).unwrap();

        let stats = stats(&dir);
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
