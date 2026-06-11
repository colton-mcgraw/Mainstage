//! `git` module — query the host git repository.

use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    MethodSig, Module, ModuleCx, NamedParam, ResolvedArg, ValueTy, named_bool, named_string,
};

/// `git.sha()`, `git.sha(short: true)`, `git.tag()`.
pub struct GitModule;

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig {
            name: "sha".to_string(),
            params: vec![],
            named: vec![NamedParam {
                name: "short".to_string(),
                ty: ValueTy::Bool,
                required: false,
            }],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "tag".to_string(),
            params: vec![],
            named: vec![NamedParam {
                name: "default".to_string(),
                ty: ValueTy::String,
                required: false,
            }],
            returns: ValueTy::String,
        },
    ]
});

impl Module for GitModule {
    fn name(&self) -> &str {
        "git"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "sha" => {
                let short = named_bool(args, "short").unwrap_or(false);
                if short {
                    run_git(&["rev-parse", "--short", "HEAD"], cx)
                } else {
                    run_git(&["rev-parse", "HEAD"], cx)
                }
            }
            "tag" => match run_git(&["describe", "--tags", "--abbrev=0"], cx) {
                Ok(v) => Ok(v),
                Err(e) => match named_string(args, "default") {
                    Some(d) => Ok(Value::String(d)),
                    None => Err(e),
                },
            },
            _ => Err(cx.error(format!("git has no method '{}'", method))),
        }
    }
}

fn run_git(git_args: &[&str], cx: &ModuleCx) -> Result<Value> {
    let output = std::process::Command::new("git")
        .args(git_args)
        .current_dir(cx.script_dir)
        .output()
        .map_err(|e| cx.error(format!("failed to run git: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(cx.error(format!("git {}: {}", git_args.join(" "), stderr.trim())));
    }

    Ok(Value::String(String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.ms"),
            line_start: 1,
            col_start: 1,
            line_end: 1,
            col_end: 1,
        }
    }

    fn call_in(method: &str, args: &[ResolvedArg], dir: &Path) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: dir,
            permissions: crate::modules::Permissions::all(),
        };
        GitModule.call(method, args, &cx)
    }

    fn kw_arg(name: &str, v: Value) -> ResolvedArg {
        ResolvedArg { name: Some(name.to_string()), value: v }
    }

    fn unwrap_str(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected string, got {other:?}"),
        }
    }

    /// A unique temporary directory for repo-touching tests.
    fn unique_dir(tag: &str) -> PathBuf {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_git_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Run a git subcommand in `dir`, asserting success. Identity and signing are
    /// supplied via `-c` flags so the test does not depend on the host's global git
    /// config (and so commits never trigger GPG/SSH signing in CI sandboxes).
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git should be runnable");
        assert!(status.success(), "git {args:?} failed");
    }

    /// Initialise a repo in a fresh temp dir with a single commit, returning the dir.
    fn repo_with_commit(tag: &str) -> PathBuf {
        let dir = unique_dir(tag);
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("file.txt"), "contents").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "initial"]);
        dir
    }

    #[test]
    fn sha_returns_full_commit_hash() {
        let dir = repo_with_commit("sha");
        let sha = unwrap_str(call_in("sha", &[], &dir).unwrap());
        assert_eq!(sha.len(), 40, "full SHA-1 hash is 40 hex chars: {sha}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "non-hex in {sha}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha_short_is_a_prefix_of_full() {
        let dir = repo_with_commit("shashort");
        let full = unwrap_str(call_in("sha", &[], &dir).unwrap());
        let short =
            unwrap_str(call_in("sha", &[kw_arg("short", Value::Bool(true))], &dir).unwrap());
        assert!(short.len() < full.len(), "short ({short}) should be shorter than full ({full})");
        assert!(full.starts_with(&short), "{full} should start with {short}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tag_returns_the_annotated_tag() {
        let dir = repo_with_commit("tag");
        git(&dir, &["tag", "v1.2.3"]);
        assert_eq!(unwrap_str(call_in("tag", &[], &dir).unwrap()), "v1.2.3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tag_falls_back_to_default_when_untagged() {
        // A repo with a commit but no tags: `git describe` fails, so the `default:`
        // keyword is returned instead of an error.
        let dir = repo_with_commit("tagdefault");
        let result =
            call_in("tag", &[kw_arg("default", Value::String("v0.0.0".to_string()))], &dir);
        assert_eq!(unwrap_str(result.unwrap()), "v0.0.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tag_errors_when_untagged_and_no_default() {
        let dir = repo_with_commit("tagerr");
        assert!(call_in("tag", &[], &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha_errors_outside_a_repo() {
        // A plain directory that is not a git repository.
        let dir = unique_dir("norepo");
        assert!(call_in("sha", &[], &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_unknown_method_errors() {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: Path::new("."),
            permissions: crate::modules::Permissions::all(),
        };
        assert!(GitModule.call("nonexistent", &[], &cx).is_err());
    }
}
