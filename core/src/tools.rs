//! Phase 53 — declared tool requirements.
//!
//! Checks a stage's `requires { … }` block before its steps run: every named program must
//! be present on `PATH`, and — when a version constraint is given — its reported version
//! must satisfy the constraint. A failed check produces a user-facing [`Error::Eval`]
//! carrying the requirement's span, so it behaves like any other stage failure (a `try`
//! block swallows it, the stage's `on_failure` fires, downstream stages are cancelled).

use std::path::{Path, PathBuf};

use crate::{
    ast::{ToolRequirement, VersionOp},
    error::{Diagnostic, Error, Result},
};

/// Check every requirement in `reqs`, accumulating one diagnostic per unmet requirement.
/// Returns `Ok(())` when all are satisfied (the common case, and when `reqs` is empty).
pub fn check_requirements(reqs: &[ToolRequirement]) -> Result<()> {
    let mut diags = Vec::new();
    for req in reqs {
        if let Err(d) = check_one(req) {
            diags.push(d);
        }
    }
    if diags.is_empty() { Ok(()) } else { Err(Error::Eval(diags)) }
}

/// Check a single requirement, returning the diagnostic to report when it is unmet.
fn check_one(req: &ToolRequirement) -> std::result::Result<(), Diagnostic> {
    if find_program(&req.program).is_none() {
        return Err(Diagnostic::new(format!(
            "required tool '{}' was not found on PATH",
            req.program
        ))
        .with_span(req.span.clone())
        .with_note("install it (or adjust PATH) so the stage can run"));
    }

    let Some(constraint) = &req.version else {
        return Ok(()); // presence-only requirement, satisfied.
    };

    let Some(found) = tool_version(&req.program) else {
        return Err(Diagnostic::new(format!(
            "could not determine the version of '{}' (needed {} {})",
            req.program,
            constraint.op.token(),
            constraint.version
        ))
        .with_span(req.span.clone())
        .with_note("ran `<tool> --version` but found no version number in its output"));
    };

    // The constraint's declared version is validated at analysis time; parse defensively here.
    let want = parse_version(&constraint.version).unwrap_or_default();
    let found_parts = parse_version(&found).unwrap_or_default();
    if !satisfies(&found_parts, constraint.op, &want) {
        return Err(Diagnostic::new(format!(
            "tool '{}' version {} does not satisfy {} {}",
            req.program,
            found,
            constraint.op.token(),
            constraint.version
        ))
        .with_span(req.span.clone()));
    }
    Ok(())
}

/// Resolve `program` to an executable path, or `None` when it is not found. A name
/// containing a path separator is checked directly; a bare name is searched on `PATH`
/// (honoring `PATHEXT` on Windows).
fn find_program(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    if p.components().count() > 1 || program.contains('/') || program.contains('\\') {
        return is_executable_file(p).then(|| p.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
        // On Windows, try each PATHEXT extension (`cargo` → `cargo.exe`).
        #[cfg(windows)]
        for ext in windows_path_exts() {
            let with_ext = dir.join(format!("{program}{ext}"));
            if is_executable_file(&with_ext) {
                return Some(with_ext);
            }
        }
    }
    None
}

/// Whether `path` exists and is a regular file (executable, on Unix). Directories never count.
fn is_executable_file(path: &Path) -> bool {
    let Ok(md) = std::fs::metadata(path) else {
        return false;
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The executable extensions from `PATHEXT`, lowercased and dot-prefixed, with a sensible
/// default when the variable is unset.
#[cfg(windows)]
fn windows_path_exts() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    raw.split(';').filter(|s| !s.is_empty()).map(|s| s.to_ascii_lowercase()).collect()
}

/// Run `program --version` and return its trimmed output, or `None` when the program could
/// not be spawned. Both stdout and stderr are considered (some tools print to stderr).
fn tool_version(program: &str) -> Option<String> {
    let out = std::process::Command::new(program).arg("--version").output().ok()?;
    // Prefer stdout; fall back to stderr only when the command succeeded (some tools, e.g.
    // `java`, print their version banner to stderr). A non-zero exit with empty stdout means
    // the tool does not understand `--version`, so we cannot determine a version.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if !stdout.trim().is_empty() {
        stdout.into_owned()
    } else if out.status.success() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        return None;
    };
    // Only accept output that actually contains a parseable version number.
    parse_version(&text).map(|_| text.trim().to_string())
}

/// Extract the first dotted-numeric version from `text` (e.g. `"cargo 1.70.0 (…)"` → `[1,70,0]`).
/// Returns `None` when no digit run is present. Public so semantic analysis can validate a
/// declared constraint version with the same parser the runtime uses.
pub fn parse_version(text: &str) -> Option<Vec<u64>> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|b| b.is_ascii_digit())?;
    // Consume a run of digits and dots; stop at the first other character.
    let end = bytes[start..]
        .iter()
        .position(|b| !(b.is_ascii_digit() || *b == b'.'))
        .map(|i| start + i)
        .unwrap_or(bytes.len());
    let parts: Vec<u64> = text[start..end]
        .split('.')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect();
    (!parts.is_empty()).then_some(parts)
}

/// Whether `found` satisfies `op found-vs-want`. Versions are compared component-wise, the
/// shorter padded with zeros (so `1.70` and `1.70.0` compare equal).
fn satisfies(found: &[u64], op: VersionOp, want: &[u64]) -> bool {
    let ordering = compare_versions(found, want);
    match op {
        VersionOp::Ge => ordering.is_ge(),
        VersionOp::Gt => ordering.is_gt(),
        VersionOp::Le => ordering.is_le(),
        VersionOp::Lt => ordering.is_lt(),
        VersionOp::Eq => ordering.is_eq(),
    }
}

/// Component-wise version comparison, padding the shorter with zeros.
fn compare_versions(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    for i in 0..len {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_from_tool_output() {
        assert_eq!(parse_version("cargo 1.70.0 (abc 2023)"), Some(vec![1, 70, 0]));
        assert_eq!(parse_version("rustc 1.81.1"), Some(vec![1, 81, 1]));
        assert_eq!(parse_version("Python 3.11.2"), Some(vec![3, 11, 2]));
        assert_eq!(parse_version("go version go1.21.3 linux/amd64"), Some(vec![1, 21, 3]));
        assert_eq!(parse_version("v2"), Some(vec![2]));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn version_padding_treats_missing_components_as_zero() {
        assert_eq!(compare_versions(&[1, 70], &[1, 70, 0]), std::cmp::Ordering::Equal);
        assert!(compare_versions(&[1, 70, 1], &[1, 70]).is_gt());
        assert!(compare_versions(&[1, 9], &[1, 10]).is_lt());
    }

    #[test]
    fn satisfies_each_operator() {
        let found = vec![1, 70, 0];
        assert!(satisfies(&found, VersionOp::Ge, &[1, 70]));
        assert!(satisfies(&found, VersionOp::Ge, &[1, 60]));
        assert!(!satisfies(&found, VersionOp::Ge, &[1, 71]));
        assert!(satisfies(&found, VersionOp::Gt, &[1, 69]));
        assert!(!satisfies(&found, VersionOp::Gt, &[1, 70, 0]));
        assert!(satisfies(&found, VersionOp::Le, &[1, 70]));
        assert!(satisfies(&found, VersionOp::Lt, &[2, 0]));
        assert!(satisfies(&found, VersionOp::Eq, &[1, 70, 0]));
        assert!(!satisfies(&found, VersionOp::Eq, &[1, 70, 1]));
    }

    #[test]
    fn missing_program_is_not_found() {
        assert!(find_program("ms_definitely_no_such_program_zzz").is_none());
    }
}
