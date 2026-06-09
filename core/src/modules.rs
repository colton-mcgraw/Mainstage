//! Phase 4 — Module System.
//!
//! Implements the built-in modules callable from Mainstage scripts via
//! `import "<name>" as <alias>` declarations followed by `<alias>.<method>(...)` calls.
//!
//! Supported modules:
//! - `env` — read environment variables
//! - `git` — query the host git repository

use std::path::Path;

use crate::error::{Diagnostic, Error, Result, Span};
use crate::eval::Value;

// ── Resolved argument ─────────────────────────────────────────────────────────

/// A module-call argument whose expression has already been evaluated.
#[derive(Debug)]
pub struct ResolvedArg {
    /// `Some(name)` for keyword arguments (e.g. `short: true`); `None` for positional.
    pub name: Option<String>,
    pub value: Value,
}

// ── Public dispatch ───────────────────────────────────────────────────────────

/// Route a module method call to the matching built-in implementation.
///
/// `module_name` is the raw name from the `import` declaration (e.g. `"git"`), not the
/// local alias. `args` must already be evaluated. `script_dir` is the directory of the
/// `.ms` file, used as the working directory for `git` commands.
pub fn dispatch(
    module_name: &str,
    method: &str,
    args: &[ResolvedArg],
    span: &Span,
    script_dir: &Path,
) -> Result<Value> {
    match module_name {
        "env" => env_call(method, args, span),
        "git" => git_call(method, args, span, script_dir),
        _ => Err(mod_err(format!("unknown module '{}'", module_name), span)),
    }
}

// ── env module ────────────────────────────────────────────────────────────────

fn env_call(method: &str, args: &[ResolvedArg], span: &Span) -> Result<Value> {
    match method {
        "get" => {
            let var = require_positional_string(args, 0, "env.get", span)?;
            let val = match std::env::var(&var) {
                Ok(v) => v,
                // Variable not set — return the `default:` keyword argument or empty string.
                Err(_) => named_string(args, "default").unwrap_or_default(),
            };
            Ok(Value::String(val))
        }
        _ => Err(mod_err(format!("env has no method '{}'", method), span)),
    }
}

// ── git module ────────────────────────────────────────────────────────────────

fn git_call(method: &str, args: &[ResolvedArg], span: &Span, dir: &Path) -> Result<Value> {
    match method {
        "sha" => {
            let short = named_bool(args, "short").unwrap_or(false);
            if short {
                run_git(&["rev-parse", "--short", "HEAD"], dir, span)
            } else {
                run_git(&["rev-parse", "HEAD"], dir, span)
            }
        }
        "tag" => run_git(&["describe", "--tags", "--abbrev=0"], dir, span),
        _ => Err(mod_err(format!("git has no method '{}'", method), span)),
    }
}

fn run_git(git_args: &[&str], dir: &Path, span: &Span) -> Result<Value> {
    let output = std::process::Command::new("git")
        .args(git_args)
        .current_dir(dir)
        .output()
        .map_err(|e| mod_err(format!("failed to run git: {}", e), span))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(mod_err(
            format!("git {}: {}", git_args.join(" "), stderr.trim()),
            span,
        ));
    }

    Ok(Value::String(String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

// ── Argument helpers ──────────────────────────────────────────────────────────

/// Return the `idx`-th positional (unnamed) argument as a `String`, or error.
fn require_positional_string(
    args: &[ResolvedArg],
    idx: usize,
    fn_name: &str,
    span: &Span,
) -> Result<String> {
    let positional: Vec<&ResolvedArg> = args.iter().filter(|a| a.name.is_none()).collect();
    match positional.get(idx) {
        Some(a) => match &a.value {
            Value::String(s) => Ok(s.clone()),
            _ => Err(mod_err(
                format!("{}: argument {} must be a string", fn_name, idx + 1),
                span,
            )),
        },
        None => Err(mod_err(
            format!("{} requires at least {} positional argument(s)", fn_name, idx + 1),
            span,
        )),
    }
}

/// Return the value of a named `String` argument, or `None` if absent or wrong type.
fn named_string(args: &[ResolvedArg], name: &str) -> Option<String> {
    args.iter()
        .find(|a| a.name.as_deref() == Some(name))
        .and_then(|a| match &a.value {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
}

/// Return the value of a named `Bool` argument, or `None` if absent or wrong type.
fn named_bool(args: &[ResolvedArg], name: &str) -> Option<bool> {
    args.iter()
        .find(|a| a.name.as_deref() == Some(name))
        .and_then(|a| match a.value {
            Value::Bool(b) => Some(b),
            _ => None,
        })
}

// ── Error helper ──────────────────────────────────────────────────────────────

fn mod_err(msg: impl Into<String>, span: &Span) -> Error {
    Error::Eval(vec![Diagnostic::new(msg).with_span(span.clone())])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use std::path::PathBuf;

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    fn str_arg(v: &str) -> ResolvedArg {
        ResolvedArg { name: None, value: Value::String(v.to_string()) }
    }

    fn kw_arg(name: &str, v: Value) -> ResolvedArg {
        ResolvedArg { name: Some(name.to_string()), value: v }
    }

    #[test]
    fn env_get_set_var() {
        // SAFETY: single-threaded test process; no concurrent env reads.
        unsafe { std::env::set_var("_MS_TEST_VAR", "hello") };
        let result = env_call("get", &[str_arg("_MS_TEST_VAR")], &span()).unwrap();
        assert!(matches!(result, Value::String(s) if s == "hello"));
        unsafe { std::env::remove_var("_MS_TEST_VAR") };
    }

    #[test]
    fn env_get_unset_returns_empty() {
        unsafe { std::env::remove_var("_MS_TEST_MISSING") };
        let result = env_call("get", &[str_arg("_MS_TEST_MISSING")], &span()).unwrap();
        assert!(matches!(result, Value::String(s) if s.is_empty()));
    }

    #[test]
    fn env_get_unset_uses_default() {
        unsafe { std::env::remove_var("_MS_TEST_MISSING2") };
        let args = vec![
            str_arg("_MS_TEST_MISSING2"),
            kw_arg("default", Value::String("fallback".to_string())),
        ];
        let result = env_call("get", &args, &span()).unwrap();
        assert!(matches!(result, Value::String(s) if s == "fallback"));
    }

    #[test]
    fn env_unknown_method_errors() {
        assert!(env_call("nonexistent", &[], &span()).is_err());
    }

    #[test]
    fn git_unknown_method_errors() {
        assert!(git_call("nonexistent", &[], &span(), Path::new(".")).is_err());
    }
}
