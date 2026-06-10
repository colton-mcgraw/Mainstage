//! Phase 7 — Change Detection.
//!
//! Computes a content digest over a stage's resolved `inputs` and persists it,
//! together with the stage's declared output paths, to a per-project cache at
//! `.mainstage/cache.json`. On a later run, a stage is skipped when its input
//! digest is unchanged *and* all of its declared outputs still exist.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let dir = project_dir.join(CACHE_DIR);
        std::fs::create_dir_all(&dir)
            .map_err(|e| cache_err(format!("create cache dir '{}': {}", dir.display(), e)))?;
        let path = dir.join(CACHE_FILE);
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| cache_err(format!("serialize cache: {}", e)))?;
        std::fs::write(&path, text)
            .map_err(|e| cache_err(format!("write cache '{}': {}", path.display(), e)))
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

    /// Record (or replace) the cache entry for `stage` after a successful run.
    pub fn update(&mut self, stage: &str, input_digest: String, outputs: Vec<String>) {
        self.stages.insert(stage.to_string(), StageEntry { input_digest, outputs });
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
        Value::Bool(_) => {}
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
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
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
        let before = input_digest(&fileset(&[a.clone()]));
        std::fs::write(&a, "two").unwrap();
        let after = input_digest(&fileset(&[a.clone()]));
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
        let value = Value::List(vec![
            Value::String("a/x".to_string()),
            Value::String("b/y".to_string()),
        ]);
        assert_eq!(output_paths(&value), vec!["a/x".to_string(), "b/y".to_string()]);
    }
}
