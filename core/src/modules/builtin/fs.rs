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
    require_positional_string, resolve_path, MethodSig, Module, ModuleCx, Param, ResolvedArg,
    ValueTy,
};

/// `fs.exists`, `fs.read`, `fs.is_dir`, `fs.is_file`, `fs.size`, `fs.list`.
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
        // Every method takes a single path argument; resolve it up front.
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
                // No integer type in the language — render the byte count as a string.
                Ok(Value::String(meta.len().to_string()))
            }
            "list" => list_dir(&raw, &path, cx),
            _ => Err(cx.error(format!("fs has no method '{}'", method))),
        }
    }
}

/// List directory entries as paths joined onto the caller's `base` string (preserving
/// their relative/absolute form), sorted for deterministic output.
fn list_dir(base: &str, resolved: &Path, cx: &ModuleCx) -> Result<Value> {
    let read = std::fs::read_dir(resolved)
        .map_err(|e| cx.error(format!("fs.list '{}': {}", resolved.display(), e)))?;

    let mut entries: Vec<String> = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| cx.error(format!("fs.list '{}': {}", resolved.display(), e)))?;
        let name = entry.file_name();
        let joined = Path::new(base).join(&name);
        entries.push(joined.to_string_lossy().into_owned());
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
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    fn call_in(method: &str, arg: &str, dir: &Path) -> Result<Value> {
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: dir };
        let args = vec![ResolvedArg { name: None, value: Value::String(arg.to_string()) }];
        FsModule.call(method, &args, &cx)
    }

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
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

        assert!(matches!(call_in("read", "f.txt", &dir).unwrap(), Value::String(s) if s == "hello"));
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

    #[test]
    fn unknown_method_errors() {
        assert!(call_in("nope", "x", Path::new(".")).is_err());
    }
}
