//! `fs` module — read-only filesystem queries.
//!
//! File *mutation* stays in the step layer (`write` / `copy` / `move` / `delete`);
//! this module only inspects and reads. Relative paths resolve against the script
//! directory.

use std::path::Path;
use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    MethodSig, Module, ModuleCx, NamedParam, Param, ResolvedArg, ValueTy, named_string,
    path_to_slash_string, require_positional_list, require_positional_string, resolve_path,
};

/// `fs.exists`, `fs.read`, `fs.is_dir`, `fs.is_file`, `fs.size`, `fs.list`, `fs.find_first`.
pub struct FsModule;

/// A method taking a single path string and returning `returns`.
fn unary(name: &str, returns: ValueTy) -> MethodSig {
    MethodSig {
        name: name.to_string(),
        params: vec![Param { name: "path".to_string(), ty: ValueTy::String, required: true }],
        named: vec![],
        returns,
    }
}

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        unary("exists", ValueTy::Bool),
        unary("read", ValueTy::String),
        unary("is_dir", ValueTy::Bool),
        unary("is_file", ValueTy::Bool),
        unary("size", ValueTy::String),
        unary("list", ValueTy::List),
        // Return the first path in `paths` that exists; falls back to `default:` or errors.
        MethodSig {
            name: "find_first".to_string(),
            params: vec![Param { name: "paths".to_string(), ty: ValueTy::List, required: true }],
            named: vec![NamedParam {
                name: "default".to_string(),
                ty: ValueTy::String,
                required: false,
            }],
            returns: ValueTy::String,
        },
    ]
});

impl Module for FsModule {
    fn name(&self) -> &str {
        "fs"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        // `find_first` takes a *list* of candidates rather than a single path.
        if method == "find_first" {
            return find_first(args, cx);
        }

        // Every other method takes a single path argument; resolve it up front.
        let raw = require_positional_string(args, 0, &format!("fs.{method}"), cx)?;
        let path = resolve_path(cx.script_dir, &raw);

        match method {
            "exists" => Ok(Value::Bool(path.exists())),
            "is_dir" => Ok(Value::Bool(path.is_dir())),
            "is_file" => Ok(Value::Bool(path.is_file())),
            "read" => {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| cx.error(format!("fs.read '{}': {}", path.display(), e)))?;
                Ok(Value::String(text))
            }
            "size" => {
                let meta = std::fs::metadata(&path)
                    .map_err(|e| cx.error(format!("fs.size '{}': {}", path.display(), e)))?;
                // Byte count returned as a string for parity with the other `fs`
                // string getters and to keep interpolation output stable.
                Ok(Value::String(meta.len().to_string()))
            }
            "list" => list_dir(&raw, &path, cx),
            _ => Err(cx.error(format!("fs has no method '{}'", method))),
        }
    }
}

/// Return the first candidate in `paths` that exists on disk (resolved against the script
/// directory), preserving the caller's string form. When none exist, return the `default:`
/// keyword argument if given, otherwise error. Lets a script resolve a file whose location
/// varies across systems — e.g. `OVMF_CODE_4M.fd` vs `OVMF_CODE.fd` — portably.
fn find_first(args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
    let candidates = require_positional_list(args, 0, "fs.find_first", cx)?;
    for candidate in &candidates {
        let s = match candidate {
            Value::String(s) => s,
            _ => return Err(cx.error("fs.find_first: every candidate path must be a string")),
        };
        if resolve_path(cx.script_dir, s).exists() {
            return Ok(Value::String(s.clone()));
        }
    }
    match named_string(args, "default") {
        Some(default) => Ok(Value::String(default)),
        None => Err(cx.error(format!(
            "fs.find_first: none of the {} candidate path(s) exist",
            candidates.len()
        ))),
    }
}

/// List directory entries as paths joined onto the caller's `base` string (preserving
/// their relative/absolute form), sorted for deterministic output.
fn list_dir(base: &str, resolved: &Path, cx: &ModuleCx) -> Result<Value> {
    let read = std::fs::read_dir(resolved)
        .map_err(|e| cx.error(format!("fs.list '{}': {}", resolved.display(), e)))?;

    let mut entries: Vec<String> = Vec::new();
    for entry in read {
        let entry =
            entry.map_err(|e| cx.error(format!("fs.list '{}': {}", resolved.display(), e)))?;
        let name = entry.file_name();
        let joined = Path::new(base).join(&name);
        entries.push(path_to_slash_string(&joined));
    }
    entries.sort();
    Ok(Value::List(entries.into_iter().map(Value::String).collect()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.ms"),
            line_start: 1,
            col_start: 1,
            line_end: 1,
            col_end: 1,
        }
    }

    fn call_in(method: &str, arg: &str, dir: &Path) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: dir,
            permissions: crate::modules::Permissions::all(),
        };
        let args = vec![ResolvedArg { name: None, value: Value::String(arg.to_string()) }];
        FsModule.call(method, &args, &cx)
    }

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_fs_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn exists_and_type_predicates() {
        let dir = unique_dir("types");
        std::fs::write(dir.join("f.txt"), "abc").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        assert!(matches!(call_in("exists", "f.txt", &dir).unwrap(), Value::Bool(true)));
        assert!(matches!(call_in("exists", "nope", &dir).unwrap(), Value::Bool(false)));
        assert!(matches!(call_in("is_file", "f.txt", &dir).unwrap(), Value::Bool(true)));
        assert!(matches!(call_in("is_dir", "f.txt", &dir).unwrap(), Value::Bool(false)));
        assert!(matches!(call_in("is_dir", "sub", &dir).unwrap(), Value::Bool(true)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_and_size() {
        let dir = unique_dir("read");
        std::fs::write(dir.join("f.txt"), "hello").unwrap();

        assert!(
            matches!(call_in("read", "f.txt", &dir).unwrap(), Value::String(s) if s == "hello")
        );
        assert!(matches!(call_in("size", "f.txt", &dir).unwrap(), Value::String(s) if s == "5"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_is_sorted_and_path_joined() {
        let dir = unique_dir("list");
        std::fs::create_dir(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/b.txt"), "").unwrap();
        std::fs::write(dir.join("data/a.txt"), "").unwrap();

        match call_in("list", "data", &dir).unwrap() {
            Value::List(items) => {
                let names: Vec<String> = items
                    .into_iter()
                    .map(|v| match v {
                        Value::String(s) => s,
                        _ => panic!("expected strings"),
                    })
                    .collect();
                // Joined onto the caller's path, sorted.
                assert_eq!(names, vec!["data/a.txt".to_string(), "data/b.txt".to_string()]);
            }
            other => panic!("expected list, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_errors() {
        let dir = unique_dir("missing");
        assert!(call_in("read", "nope.txt", &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn size_and_list_missing_error() {
        // Unlike the predicate methods, `size` and `list` surface the I/O failure.
        let dir = unique_dir("missing2");
        assert!(call_in("size", "nope.txt", &dir).is_err());
        assert!(call_in("list", "no_such_dir", &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn predicates_are_false_for_missing_paths() {
        // The boolean predicates never error — a missing path is simply false.
        let dir = unique_dir("missing3");
        assert!(matches!(call_in("is_file", "nope", &dir).unwrap(), Value::Bool(false)));
        assert!(matches!(call_in("is_dir", "nope", &dir).unwrap(), Value::Bool(false)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Call a method with arbitrary resolved arguments (for the list-taking `find_first`).
    fn call_args(method: &str, args: &[ResolvedArg], dir: &Path) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: dir,
            permissions: crate::modules::Permissions::all(),
        };
        FsModule.call(method, args, &cx)
    }

    fn list_arg(items: &[&str]) -> ResolvedArg {
        ResolvedArg {
            name: None,
            value: Value::List(items.iter().map(|s| Value::String(s.to_string())).collect()),
        }
    }

    #[test]
    fn find_first_returns_first_existing_candidate() {
        let dir = unique_dir("find_first");
        std::fs::write(dir.join("b.fd"), "").unwrap();
        std::fs::write(dir.join("c.fd"), "").unwrap();

        // `a.fd` is missing; `b.fd` is the first that exists and its original form is kept.
        let result = call_args("find_first", &[list_arg(&["a.fd", "b.fd", "c.fd"])], &dir).unwrap();
        assert!(matches!(result, Value::String(s) if s == "b.fd"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_first_falls_back_to_default() {
        let dir = unique_dir("find_first_default");
        let args = vec![
            list_arg(&["nope1", "nope2"]),
            ResolvedArg { name: Some("default".to_string()), value: Value::String("fb".into()) },
        ];
        let result = call_args("find_first", &args, &dir).unwrap();
        assert!(matches!(result, Value::String(s) if s == "fb"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_first_errors_when_none_exist_and_no_default() {
        let dir = unique_dir("find_first_err");
        assert!(call_args("find_first", &[list_arg(&["nope1", "nope2"])], &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_method_errors() {
        assert!(call_in("nope", "x", Path::new(".")).is_err());
    }
}
