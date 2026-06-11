//! `env` module — read environment variables.

use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    MethodSig, Module, ModuleCx, NamedParam, Param, ResolvedArg, ValueTy, named_string,
    require_positional_string,
};

/// `env.get("VAR")`, `env.get("VAR", default: "...")`.
pub struct EnvModule;

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig {
            name: "get".to_string(),
            params: vec![Param { name: "var".to_string(), ty: ValueTy::String, required: true }],
            named: vec![NamedParam {
                name: "default".to_string(),
                ty: ValueTy::String,
                required: false,
            }],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "has".to_string(),
            params: vec![Param { name: "var".to_string(), ty: ValueTy::String, required: true }],
            named: vec![],
            returns: ValueTy::Bool,
        },
    ]
});

impl Module for EnvModule {
    fn name(&self) -> &str {
        "env"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "get" => {
                let var = require_positional_string(args, 0, "env.get", cx)?;
                let val = match std::env::var(&var) {
                    Ok(v) => v,
                    // Variable not set — return the `default:` keyword argument or empty string.
                    Err(_) => named_string(args, "default").unwrap_or_default(),
                };
                Ok(Value::String(val))
            }
            "has" => {
                let var = require_positional_string(args, 0, "env.has", cx)?;
                Ok(Value::Bool(std::env::var_os(&var).is_some()))
            }
            _ => Err(cx.error(format!("env has no method '{}'", method))),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use std::path::{Path, PathBuf};

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.ms"),
            line_start: 1,
            col_start: 1,
            line_end: 1,
            col_end: 1,
        }
    }

    fn call(method: &str, args: &[ResolvedArg]) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: Path::new("."),
            permissions: crate::modules::Permissions::all(),
        };
        EnvModule.call(method, args, &cx)
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
        let result = call("get", &[str_arg("_MS_TEST_VAR")]).unwrap();
        assert!(matches!(result, Value::String(s) if s == "hello"));
        unsafe { std::env::remove_var("_MS_TEST_VAR") };
    }

    #[test]
    fn env_get_unset_returns_empty() {
        unsafe { std::env::remove_var("_MS_TEST_MISSING") };
        let result = call("get", &[str_arg("_MS_TEST_MISSING")]).unwrap();
        assert!(matches!(result, Value::String(s) if s.is_empty()));
    }

    #[test]
    fn env_get_unset_uses_default() {
        unsafe { std::env::remove_var("_MS_TEST_MISSING2") };
        let args = vec![
            str_arg("_MS_TEST_MISSING2"),
            kw_arg("default", Value::String("fallback".to_string())),
        ];
        let result = call("get", &args).unwrap();
        assert!(matches!(result, Value::String(s) if s == "fallback"));
    }

    #[test]
    fn env_has_reflects_presence() {
        // SAFETY: single-threaded test process; no concurrent env reads.
        unsafe { std::env::set_var("_MS_TEST_HAS", "1") };
        assert!(matches!(call("has", &[str_arg("_MS_TEST_HAS")]).unwrap(), Value::Bool(true)));
        unsafe { std::env::remove_var("_MS_TEST_HAS") };
        assert!(matches!(call("has", &[str_arg("_MS_TEST_HAS")]).unwrap(), Value::Bool(false)));
    }

    #[test]
    fn env_unknown_method_errors() {
        assert!(call("nonexistent", &[]).is_err());
    }
}
