//! Phase 53 — input-completeness audit.
//!
//! Best-effort detection of files a stage *reads* that it did not declare in its `inputs`
//! — the most common cause of a stale cache, since change detection only fingerprints
//! declared inputs. The audit is opt-in (`mainstage --audit-inputs`) and relies on file
//! access-time (`atime`) tracking: it records a baseline at stage start, then after the
//! stage finds project files whose `atime` advanced (they were read) but whose `mtime` did
//! not (so they were not produced by the stage), excluding everything the stage declared.
//!
//! "Where the platform allows": on a `noatime` mount the audit simply finds nothing rather
//! than reporting false positives, so a probe ([`atime_supported`]) lets the front end warn
//! that the audit is inert rather than silently implying completeness.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The directory name skipped during the audit walk (Mainstage's own state).
use crate::cache::CACHE_DIR;

/// Probe whether the filesystem under `project_dir` updates access times, so the audit can
/// observe reads. Writes a temporary file, back-dates its timestamps, reads it, and checks
/// whether `atime` advanced. Returns `false` on a `noatime` mount (or any error), where the
/// audit would be inert.
pub fn atime_supported(project_dir: &Path) -> bool {
    let dir = project_dir.join(CACHE_DIR);
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let probe = dir.join(format!("atime-probe-{}", std::process::id()));
    let result = (|| -> std::io::Result<bool> {
        std::fs::write(&probe, b"probe")?;
        // Back-date both timestamps well into the past so that a subsequent read is
        // guaranteed to advance atime under `relatime` (which updates atime when it is
        // older than mtime) as well as `strictatime`.
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let f = std::fs::OpenOptions::new().write(true).open(&probe)?;
        f.set_times(std::fs::FileTimes::new().set_accessed(old).set_modified(old))?;
        drop(f);
        // Read the file (a real `read` syscall updates atime regardless of page cache).
        let _ = std::fs::read(&probe)?;
        let atime = std::fs::metadata(&probe)?.accessed()?;
        // atime advanced clearly past the back-dated value ⇒ tracking is active.
        Ok(atime > old + Duration::from_secs(60))
    })();
    let _ = std::fs::remove_file(&probe);
    result.unwrap_or(false)
}

/// Find project files that were *read* during a stage but not declared in its `inputs`.
///
/// `baseline` is the instant captured immediately before the stage's steps ran. A file is
/// flagged when its access time is after `baseline` (read during the stage) and its
/// modification time is at or before `baseline` (not written by the stage), and it is not
/// one of the stage's declared `inputs` or `outputs`. The walk skips dotfiles/dot-dirs
/// (including `.git` and the `.mainstage` cache). Returns project-relative paths, sorted.
pub fn undeclared_reads(
    project_dir: &Path,
    baseline: SystemTime,
    declared_inputs: &[PathBuf],
    declared_outputs: &[PathBuf],
) -> Vec<PathBuf> {
    // Canonicalize the declared paths once for prefix comparison (a directory input covers
    // every file beneath it). Paths that cannot be canonicalized are kept as-is.
    let declared: Vec<PathBuf> =
        declared_inputs.iter().chain(declared_outputs).map(|p| canonical(project_dir, p)).collect();
    // A canonical project root so reported paths can be shown relative to it (the raw
    // `project_dir` is often `.`, which would not prefix an absolute canonical path).
    let root = std::fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf());

    let mut found = Vec::new();
    let mut stack = vec![project_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            // Skip dotfiles and dot-directories (`.git`, `.mainstage`, editor state, …).
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(md) = entry.metadata() else { continue };
            if md.is_dir() {
                stack.push(path);
                continue;
            }
            if !md.is_file() {
                continue;
            }
            let (Ok(atime), Ok(mtime)) = (md.accessed(), md.modified()) else { continue };
            // Read during the stage (atime advanced) but not written by it (mtime did not).
            if atime <= baseline || mtime > baseline {
                continue;
            }
            let canon = canonical(project_dir, &path);
            if declared.iter().any(|d| canon == *d || canon.starts_with(d)) {
                continue;
            }
            // Report relative to the project root when possible, for a compact message.
            let display = canon.strip_prefix(&root).unwrap_or(&canon).to_path_buf();
            found.push(display);
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Canonicalize `path` (resolving it against `project_dir` when relative), falling back to
/// the joined path when canonicalization fails (e.g. the file no longer exists).
fn canonical(project_dir: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() { path.to_path_buf() } else { project_dir.join(path) };
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_audit_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn flags_read_of_undeclared_file_only() {
        let dir = unique_dir("undeclared");
        let declared = dir.join("declared.txt");
        let sneaky = dir.join("sneaky.txt");
        std::fs::write(&declared, "a").unwrap();
        std::fs::write(&sneaky, "b").unwrap();

        // Back-date both files so they predate the baseline (they are pre-existing sources).
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        for p in [&declared, &sneaky] {
            let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
            f.set_times(std::fs::FileTimes::new().set_accessed(old).set_modified(old)).unwrap();
        }

        // Baseline is "now": both files' mtimes are in the past (not written this stage).
        let baseline = SystemTime::now();
        // Simulate the stage reading only the sneaky (undeclared) file by advancing its atime.
        let after = SystemTime::now() + Duration::from_secs(10);
        let f = std::fs::OpenOptions::new().write(true).open(&sneaky).unwrap();
        f.set_times(std::fs::FileTimes::new().set_accessed(after).set_modified(old)).unwrap();
        drop(f);

        let reads = undeclared_reads(&dir, baseline, std::slice::from_ref(&declared), &[]);
        assert_eq!(reads, vec![PathBuf::from("sneaky.txt")], "only the undeclared read is flagged");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn declared_input_read_is_not_flagged() {
        let dir = unique_dir("declared_ok");
        let input = dir.join("in.txt");
        std::fs::write(&input, "x").unwrap();

        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let baseline = SystemTime::now();
        let after = baseline + Duration::from_secs(10);
        let f = std::fs::OpenOptions::new().write(true).open(&input).unwrap();
        f.set_times(std::fs::FileTimes::new().set_accessed(after).set_modified(old)).unwrap();
        drop(f);

        let reads = undeclared_reads(&dir, baseline, std::slice::from_ref(&input), &[]);
        assert!(reads.is_empty(), "a declared input read must not be flagged");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn freshly_written_output_is_not_flagged() {
        // A file written (mtime after baseline) and read during the stage is an output/
        // intermediate, not an undeclared external read.
        let dir = unique_dir("written");
        let out = dir.join("out.txt");
        let baseline = SystemTime::now();
        std::fs::write(&out, "y").unwrap(); // mtime is now > baseline
        let after = SystemTime::now() + Duration::from_secs(10);
        let f = std::fs::OpenOptions::new().write(true).open(&out).unwrap();
        f.set_times(
            std::fs::FileTimes::new()
                .set_accessed(after)
                .set_modified(SystemTime::now() + Duration::from_secs(5)),
        )
        .unwrap();
        drop(f);

        let reads = undeclared_reads(&dir, baseline, &[], &[]);
        assert!(reads.is_empty(), "a file written this stage is not an undeclared read");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
