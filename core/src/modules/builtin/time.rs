//! `time` module — read the host wall clock.
//!
//! **Determinism note:** every method reads the current time, so feeding a `time`
//! result into a stage's `inputs`/`outputs` defeats change detection — the digest
//! changes on every run, so the stage never reports as up-to-date. Prefer `time` for
//! display and metadata (e.g. a build timestamp), not for cache keys.
//!
//! Unlike `shell` and `http`, `time` reads no external resource and is *not* gated on
//! a capability.

use std::sync::LazyLock;

use chrono::format::StrftimeItems;
use chrono::Utc;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    require_positional_string, MethodSig, Module, ModuleCx, Param, ResolvedArg, ValueTy,
};

/// `time.now()` (RFC 3339), `time.unix()` (seconds since the epoch),
/// `time.format("%Y-%m-%d")` (strftime).
pub struct TimeModule;

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig { name: "now".to_string(), params: vec![], named: vec![], returns: ValueTy::String },
        MethodSig {
            name: "unix".to_string(),
            params: vec![],
            named: vec![],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "format".to_string(),
            params: vec![Param {
                name: "fmt".to_string(),
                ty: ValueTy::String,
                required: true,
            }],
            named: vec![],
            returns: ValueTy::String,
        },
    ]
});

impl Module for TimeModule {
    fn name(&self) -> &str {
        "time"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "now" => Ok(Value::String(Utc::now().to_rfc3339())),
            // Epoch second count returned as a string to keep interpolation output
            // stable and avoid feeding a live clock into a typed numeric position.
            "unix" => Ok(Value::String(Utc::now().timestamp().to_string())),
            "format" => {
                let fmt = require_positional_string(args, 0, "time.format", cx)?;
                // Validate the format string up front: `format_with_items` over parsed
                // items cannot panic, whereas formatting a bad pattern directly would.
                let items = StrftimeItems::new(&fmt).parse().map_err(|e| {
                    cx.error(format!("time.format: invalid format '{}': {}", fmt, e))
                })?;
                Ok(Value::String(Utc::now().format_with_items(items.iter()).to_string()))
            }
            _ => Err(cx.error(format!("time has no method '{}'", method))),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use crate::modules::Permissions;
    use std::path::{Path, PathBuf};

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    fn call(method: &str, args: &[ResolvedArg]) -> Result<Value> {
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: Path::new("."), permissions: Permissions::default() };
        TimeModule.call(method, args, &cx)
    }

    fn unwrap_str(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn now_is_parseable_rfc3339() {
        let s = unwrap_str(call("now", &[]).unwrap());
        assert!(chrono::DateTime::parse_from_rfc3339(&s).is_ok(), "not RFC 3339: {s}");
    }

    #[test]
    fn unix_is_a_positive_integer() {
        let s = unwrap_str(call("unix", &[]).unwrap());
        let secs: i64 = s.parse().expect("unix timestamp should be an integer");
        assert!(secs > 0, "expected a positive epoch, got {secs}");
    }

    #[test]
    fn format_renders_pattern() {
        let arg = ResolvedArg { name: None, value: Value::String("%Y".to_string()) };
        let year = unwrap_str(call("format", &[arg]).unwrap());
        assert_eq!(year.len(), 4, "a %Y year is four digits: {year}");
        assert!(year.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn format_invalid_pattern_errors() {
        // `%` followed by an unknown specifier is rejected rather than panicking.
        let arg = ResolvedArg { name: None, value: Value::String("%Q".to_string()) };
        assert!(call("format", &[arg]).is_err());
    }

    #[test]
    fn unknown_method_errors() {
        assert!(call("nope", &[]).is_err());
    }
}
