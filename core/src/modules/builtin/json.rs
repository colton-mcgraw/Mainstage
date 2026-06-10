//! `json` module — opaque-string JSON with path getters.
//!
//! JSON values are carried as plain strings (their serialized text) rather than a
//! new `Value` variant, so interpolation and `if/else` type-compatibility are
//! unaffected. `parse` validates and compacts, `stringify` validates and pretty-
//! prints, and `get` extracts a value at a dotted path as a string.

use std::sync::LazyLock;

use serde_json::Value as Json;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    require_positional_string, MethodSig, Module, ModuleCx, Param, ResolvedArg, ValueTy,
};

/// `json.parse(text)`, `json.stringify(text)`, `json.get(text, "a.b.0")`.
pub struct JsonModule;

fn text_param() -> Param {
    Param { name: "text".to_string(), ty: ValueTy::String, required: true }
}

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig {
            name: "parse".to_string(),
            params: vec![text_param()],
            named: vec![],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "stringify".to_string(),
            params: vec![text_param()],
            named: vec![],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "get".to_string(),
            params: vec![
                text_param(),
                Param { name: "path".to_string(), ty: ValueTy::String, required: true },
            ],
            named: vec![],
            returns: ValueTy::String,
        },
    ]
});

impl Module for JsonModule {
    fn name(&self) -> &str {
        "json"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "parse" => {
                let text = require_positional_string(args, 0, "json.parse", cx)?;
                let value = parse(&text, cx)?;
                // Canonical compact form; also serves as validation.
                Ok(Value::String(value.to_string()))
            }
            "stringify" => {
                let text = require_positional_string(args, 0, "json.stringify", cx)?;
                let value = parse(&text, cx)?;
                let pretty = serde_json::to_string_pretty(&value)
                    .map_err(|e| cx.error(format!("json.stringify: {}", e)))?;
                Ok(Value::String(pretty))
            }
            "get" => {
                let text = require_positional_string(args, 0, "json.get", cx)?;
                let path = require_positional_string(args, 1, "json.get", cx)?;
                let root = parse(&text, cx)?;
                match get_path(&root, &path) {
                    Some(found) => Ok(Value::String(render(found))),
                    None => Err(cx.error(format!("json.get: no value at path '{}'", path))),
                }
            }
            _ => Err(cx.error(format!("json has no method '{}'", method))),
        }
    }
}

fn parse(text: &str, cx: &ModuleCx) -> Result<Json> {
    serde_json::from_str(text).map_err(|e| cx.error(format!("json: invalid JSON: {}", e)))
}

/// Navigate a dotted path (`"a.b.0"`): object keys by name, array elements by index.
/// An empty path returns the root.
fn get_path<'a>(root: &'a Json, path: &str) -> Option<&'a Json> {
    if path.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for seg in path.split('.') {
        cur = match cur {
            Json::Object(map) => map.get(seg)?,
            Json::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Render a JSON value as a string: scalars unquoted, composites as compact JSON
/// (so a nested object/array can be fed back into another `json.get`).
fn render(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        Json::Null => "null".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        composite => composite.to_string(),
    }
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

    fn call(method: &str, args: &[&str]) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: Path::new("."),
            permissions: crate::modules::Permissions::all(),
        };
        let resolved: Vec<ResolvedArg> = args
            .iter()
            .map(|a| ResolvedArg { name: None, value: Value::String(a.to_string()) })
            .collect();
        JsonModule.call(method, &resolved, &cx)
    }

    fn unwrap_str(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn parse_compacts_and_validates() {
        assert_eq!(unwrap_str(call("parse", &[r#"{ "a" : 1 }"#]).unwrap()), r#"{"a":1}"#);
        assert!(call("parse", &["{not json}"]).is_err());
    }

    #[test]
    fn get_scalar_and_nested() {
        let doc = r#"{"name": "app", "tags": ["x", "y"], "meta": {"v": 2}}"#;
        assert_eq!(unwrap_str(call("get", &[doc, "name"]).unwrap()), "app");
        assert_eq!(unwrap_str(call("get", &[doc, "tags.1"]).unwrap()), "y");
        assert_eq!(unwrap_str(call("get", &[doc, "meta.v"]).unwrap()), "2");
        // A composite value is returned as compact JSON, chainable into another get.
        assert_eq!(unwrap_str(call("get", &[doc, "meta"]).unwrap()), r#"{"v":2}"#);
    }

    #[test]
    fn get_missing_path_errors() {
        assert!(call("get", &[r#"{"a": 1}"#, "b"]).is_err());
        assert!(call("get", &[r#"["x"]"#, "5"]).is_err());
    }

    #[test]
    fn stringify_pretty_prints() {
        let out = unwrap_str(call("stringify", &[r#"{"a":1}"#]).unwrap());
        assert!(out.contains('\n'), "expected indented output, got: {out}");
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn stringify_invalid_json_errors() {
        assert!(call("stringify", &["{nope}"]).is_err());
    }

    #[test]
    fn get_empty_path_returns_root() {
        // An empty path navigates nowhere and yields the (compacted) root document.
        assert_eq!(unwrap_str(call("get", &[r#"{ "a" : 1 }"#, ""]).unwrap()), r#"{"a":1}"#);
    }

    #[test]
    fn get_renders_scalar_kinds_unquoted() {
        let doc = r#"{"s": "txt", "n": 3, "b": false, "nil": null}"#;
        assert_eq!(unwrap_str(call("get", &[doc, "s"]).unwrap()), "txt");
        assert_eq!(unwrap_str(call("get", &[doc, "n"]).unwrap()), "3");
        assert_eq!(unwrap_str(call("get", &[doc, "b"]).unwrap()), "false");
        assert_eq!(unwrap_str(call("get", &[doc, "nil"]).unwrap()), "null");
    }

    #[test]
    fn get_into_scalar_errors() {
        // A path segment descending into a scalar has nowhere to go.
        assert!(call("get", &[r#"{"a": 1}"#, "a.b"]).is_err());
    }

    #[test]
    fn unknown_method_errors() {
        assert!(call("nope", &["{}"]).is_err());
    }
}
