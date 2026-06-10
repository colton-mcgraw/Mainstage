//! `git` module — query the host git repository.

use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{named_bool, named_string, MethodSig, Module, ModuleCx, NamedParam, ResolvedArg, ValueTy};

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
            "tag" => {
                match run_git(&["describe", "--tags", "--abbrev=0"], cx) {
                    Ok(v) => Ok(v),
                    Err(e) => match named_string(args, "default") {
                        Some(d) => Ok(Value::String(d)),
                        None => Err(e),
                    },
                }
            }
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

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    #[test]
    fn git_unknown_method_errors() {
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: Path::new(".") };
        assert!(GitModule.call("nonexistent", &[], &cx).is_err());
    }
}
