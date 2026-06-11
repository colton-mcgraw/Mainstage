//! `str` module — string manipulation. Pure and deterministic.

use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    require_positional_list, require_positional_string, MethodSig, Module, ModuleCx, Param,
    ResolvedArg, ValueTy,
};

/// `str.upper`, `str.lower`, `str.trim`, `str.replace`, `str.split`, `str.join`,
/// `str.contains`, `str.starts_with`, `str.ends_with`, `str.len`.
pub struct StrModule;

/// A method taking a single string and returning a string.
fn unary(name: &str) -> MethodSig {
    MethodSig {
        name: name.to_string(),
        params: vec![Param { name: "s".to_string(), ty: ValueTy::String, required: true }],
        named: vec![],
        returns: ValueTy::String,
    }
}

/// A method taking a string plus a second string, returning `returns`.
fn binary(name: &str, second: &str, returns: ValueTy) -> MethodSig {
    MethodSig {
        name: name.to_string(),
        params: vec![
            Param { name: "s".to_string(), ty: ValueTy::String, required: true },
            Param { name: second.to_string(), ty: ValueTy::String, required: true },
        ],
        named: vec![],
        returns,
    }
}

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        unary("upper"),
        unary("lower"),
        unary("trim"),
        MethodSig {
            name: "replace".to_string(),
            params: vec![
                Param { name: "s".to_string(), ty: ValueTy::String, required: true },
                Param { name: "from".to_string(), ty: ValueTy::String, required: true },
                Param { name: "to".to_string(), ty: ValueTy::String, required: true },
            ],
            named: vec![],
            returns: ValueTy::String,
        },
        binary("split", "sep", ValueTy::List),
        MethodSig {
            name: "join".to_string(),
            params: vec![
                Param { name: "parts".to_string(), ty: ValueTy::List, required: true },
                Param { name: "sep".to_string(), ty: ValueTy::String, required: true },
            ],
            named: vec![],
            returns: ValueTy::String,
        },
        binary("contains", "needle", ValueTy::Bool),
        binary("starts_with", "prefix", ValueTy::Bool),
        binary("ends_with", "suffix", ValueTy::Bool),
        unary("len"),
    ]
});

impl Module for StrModule {
    fn name(&self) -> &str {
        "str"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "upper" => str1(args, "str.upper", cx, |s| Value::String(s.to_uppercase())),
            "lower" => str1(args, "str.lower", cx, |s| Value::String(s.to_lowercase())),
            "trim" => str1(args, "str.trim", cx, |s| Value::String(s.trim().to_string())),
            // Character count, returned as a string so existing scripts and
            // interpolations that consume `str.len` keep working unchanged.
            "len" => str1(args, "str.len", cx, |s| Value::String(s.chars().count().to_string())),
            "replace" => {
                let s = require_positional_string(args, 0, "str.replace", cx)?;
                let from = require_positional_string(args, 1, "str.replace", cx)?;
                let to = require_positional_string(args, 2, "str.replace", cx)?;
                Ok(Value::String(s.replace(&from, &to)))
            }
            "split" => {
                let s = require_positional_string(args, 0, "str.split", cx)?;
                let sep = require_positional_string(args, 1, "str.split", cx)?;
                let parts = if sep.is_empty() {
                    // Splitting on an empty separator yields the individual characters.
                    s.chars().map(|c| Value::String(c.to_string())).collect()
                } else {
                    s.split(&sep).map(|p| Value::String(p.to_string())).collect()
                };
                Ok(Value::List(parts))
            }
            "join" => {
                let parts = require_positional_list(args, 0, "str.join", cx)?;
                let sep = require_positional_string(args, 1, "str.join", cx)?;
                let joined =
                    parts.iter().map(|v| v.display_string()).collect::<Vec<_>>().join(&sep);
                Ok(Value::String(joined))
            }
            "contains" => str2(args, "str.contains", cx, |s, n| Value::Bool(s.contains(n))),
            "starts_with" => str2(args, "str.starts_with", cx, |s, p| Value::Bool(s.starts_with(p))),
            "ends_with" => str2(args, "str.ends_with", cx, |s, p| Value::Bool(s.ends_with(p))),
            _ => Err(cx.error(format!("str has no method '{}'", method))),
        }
    }
}

/// Apply `f` to the first positional string argument.
fn str1(
    args: &[ResolvedArg],
    fn_name: &str,
    cx: &ModuleCx,
    f: impl FnOnce(&str) -> Value,
) -> Result<Value> {
    let s = require_positional_string(args, 0, fn_name, cx)?;
    Ok(f(&s))
}

/// Apply `f` to the first two positional string arguments.
fn str2(
    args: &[ResolvedArg],
    fn_name: &str,
    cx: &ModuleCx,
    f: impl FnOnce(&str, &str) -> Value,
) -> Result<Value> {
    let a = require_positional_string(args, 0, fn_name, cx)?;
    let b = require_positional_string(args, 1, fn_name, cx)?;
    Ok(f(&a, &b))
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

    fn call(method: &str, args: &[ResolvedArg]) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: Path::new("."),
            permissions: crate::modules::Permissions::all(),
        };
        StrModule.call(method, args, &cx)
    }

    fn s(v: &str) -> ResolvedArg {
        ResolvedArg { name: None, value: Value::String(v.to_string()) }
    }

    fn list(items: &[&str]) -> ResolvedArg {
        ResolvedArg {
            name: None,
            value: Value::List(items.iter().map(|i| Value::String(i.to_string())).collect()),
        }
    }

    #[test]
    fn upper_lower_trim() {
        assert!(matches!(call("upper", &[s("aB")]).unwrap(), Value::String(x) if x == "AB"));
        assert!(matches!(call("lower", &[s("aB")]).unwrap(), Value::String(x) if x == "ab"));
        assert!(matches!(call("trim", &[s("  x  ")]).unwrap(), Value::String(x) if x == "x"));
    }

    #[test]
    fn replace_substitutes() {
        let r = call("replace", &[s("a-b-c"), s("-"), s("_")]).unwrap();
        assert!(matches!(r, Value::String(x) if x == "a_b_c"));
    }

    #[test]
    fn split_and_join_roundtrip() {
        let parts = call("split", &[s("a,b,c"), s(",")]).unwrap();
        assert!(matches!(&parts, Value::List(items) if items.len() == 3));
        let joined = call("join", &[list(&["a", "b", "c"]), s("/")]).unwrap();
        assert!(matches!(joined, Value::String(x) if x == "a/b/c"));
    }

    #[test]
    fn split_empty_separator_yields_chars() {
        let parts = call("split", &[s("ab"), s("")]).unwrap();
        assert!(matches!(parts, Value::List(items) if items.len() == 2));
    }

    #[test]
    fn predicates_return_bool() {
        assert!(matches!(call("contains", &[s("hello"), s("ell")]).unwrap(), Value::Bool(true)));
        assert!(matches!(call("starts_with", &[s("hello"), s("he")]).unwrap(), Value::Bool(true)));
        assert!(matches!(call("ends_with", &[s("hello"), s("lo")]).unwrap(), Value::Bool(true)));
        assert!(matches!(call("contains", &[s("hello"), s("xyz")]).unwrap(), Value::Bool(false)));
    }

    #[test]
    fn len_counts_characters() {
        // Counts characters, not bytes: "é" is one char.
        assert!(matches!(call("len", &[s("café")]).unwrap(), Value::String(x) if x == "4"));
    }

    #[test]
    fn join_renders_non_string_values() {
        // `join` stringifies each element via `display_string`, so non-string values
        // (here a bool) are rendered rather than rejected.
        let parts = ResolvedArg {
            name: None,
            value: Value::List(vec![Value::String("on".to_string()), Value::Bool(true)]),
        };
        assert!(matches!(call("join", &[parts, s(":")]).unwrap(), Value::String(x) if x == "on:true"));
    }

    #[test]
    fn wrong_argument_type_errors() {
        // A string method given a non-string positional argument is an error.
        let b = ResolvedArg { name: None, value: Value::Bool(true) };
        assert!(call("upper", &[b]).is_err());
    }

    #[test]
    fn missing_arguments_error() {
        // `replace` needs three positionals; fewer is an arity error.
        assert!(call("replace", &[s("a"), s("b")]).is_err());
        assert!(call("contains", &[s("a")]).is_err());
    }

    #[test]
    fn unknown_method_errors() {
        assert!(call("nope", &[s("x")]).is_err());
    }
}
