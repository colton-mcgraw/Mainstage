//! `path` module — path manipulation. Pure string/path operations; no I/O.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    require_positional_string, MethodSig, Module, ModuleCx, Param, ResolvedArg, ValueTy,
};

/// `path.join`, `path.dir`, `path.base`, `path.stem`, `path.ext`, `path.with_ext`,
/// `path.abs`.
pub struct PathModule;

/// A method taking a single path string and returning a string.
fn unary(name: &str) -> MethodSig {
    MethodSig {
        name: name.to_string(),
        params: vec![Param { name: "path".to_string(), ty: ValueTy::String, required: true }],
        named: vec![],
        returns: ValueTy::String,
    }
}

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig {
            name: "join".to_string(),
            params: vec![
                Param { name: "base".to_string(), ty: ValueTy::String, required: true },
                Param { name: "child".to_string(), ty: ValueTy::String, required: true },
            ],
            named: vec![],
            returns: ValueTy::String,
        },
        unary("dir"),
        unary("base"),
        unary("stem"),
        unary("ext"),
        MethodSig {
            name: "with_ext".to_string(),
            params: vec![
                Param { name: "path".to_string(), ty: ValueTy::String, required: true },
                Param { name: "ext".to_string(), ty: ValueTy::String, required: true },
            ],
            named: vec![],
            returns: ValueTy::String,
        },
        unary("abs"),
    ]
});

impl Module for PathModule {
    fn name(&self) -> &str {
        "path"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "join" => {
                let base = require_positional_string(args, 0, "path.join", cx)?;
                let child = require_positional_string(args, 1, "path.join", cx)?;
                Ok(string(PathBuf::from(base).join(child)))
            }
            "dir" => path1(args, "path.dir", cx, |p| {
                p.parent().map(Path::to_path_buf).unwrap_or_default()
            }),
            "base" => component(args, "path.base", cx, Path::file_name),
            "stem" => component(args, "path.stem", cx, Path::file_stem),
            "ext" => component(args, "path.ext", cx, Path::extension),
            "with_ext" => {
                let p = require_positional_string(args, 0, "path.with_ext", cx)?;
                let ext = require_positional_string(args, 1, "path.with_ext", cx)?;
                let mut buf = PathBuf::from(p);
                // An empty extension drops the extension entirely.
                buf.set_extension(ext.trim_start_matches('.'));
                Ok(string(buf))
            }
            "abs" => {
                let p = require_positional_string(args, 0, "path.abs", cx)?;
                let path = PathBuf::from(&p);
                let abs =
                    if path.is_absolute() { path } else { cx.script_dir.join(path) };
                Ok(string(abs))
            }
            _ => Err(cx.error(format!("path has no method '{}'", method))),
        }
    }
}

/// Apply a `Path -> PathBuf` transform to the first positional string argument.
fn path1(
    args: &[ResolvedArg],
    fn_name: &str,
    cx: &ModuleCx,
    f: impl FnOnce(&Path) -> PathBuf,
) -> Result<Value> {
    let p = require_positional_string(args, 0, fn_name, cx)?;
    Ok(string(f(Path::new(&p))))
}

/// Return a single `OsStr` component (e.g. file name, stem, extension) as a string,
/// or the empty string when the path has no such component.
fn component(
    args: &[ResolvedArg],
    fn_name: &str,
    cx: &ModuleCx,
    f: impl FnOnce(&Path) -> Option<&std::ffi::OsStr>,
) -> Result<Value> {
    let p = require_positional_string(args, 0, fn_name, cx)?;
    let value = f(Path::new(&p)).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Value::String(value))
}

fn string(path: PathBuf) -> Value {
    Value::String(path.to_string_lossy().into_owned())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    fn call_in(method: &str, args: &[ResolvedArg], dir: &Path) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: dir,
            permissions: crate::modules::Permissions::all(),
        };
        PathModule.call(method, args, &cx)
    }

    fn call(method: &str, args: &[ResolvedArg]) -> Result<Value> {
        call_in(method, args, Path::new("."))
    }

    fn s(v: &str) -> ResolvedArg {
        ResolvedArg { name: None, value: Value::String(v.to_string()) }
    }

    fn unwrap_str(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn join_combines() {
        assert_eq!(unwrap_str(call("join", &[s("a/b"), s("c.txt")]).unwrap()), "a/b/c.txt");
    }

    #[test]
    fn components() {
        assert_eq!(unwrap_str(call("dir", &[s("a/b/c.txt")]).unwrap()), "a/b");
        assert_eq!(unwrap_str(call("base", &[s("a/b/c.txt")]).unwrap()), "c.txt");
        assert_eq!(unwrap_str(call("stem", &[s("a/b/c.txt")]).unwrap()), "c");
        assert_eq!(unwrap_str(call("ext", &[s("a/b/c.txt")]).unwrap()), "txt");
    }

    #[test]
    fn component_absent_is_empty() {
        assert_eq!(unwrap_str(call("ext", &[s("README")]).unwrap()), "");
        assert_eq!(unwrap_str(call("dir", &[s("file")]).unwrap()), "");
    }

    #[test]
    fn with_ext_replaces_and_accepts_dot() {
        assert_eq!(unwrap_str(call("with_ext", &[s("a/c.txt"), s("md")]).unwrap()), "a/c.md");
        assert_eq!(unwrap_str(call("with_ext", &[s("a/c.txt"), s(".rs")]).unwrap()), "a/c.rs");
    }

    #[test]
    fn abs_joins_relative_to_script_dir_and_keeps_absolute() {
        let dir = Path::new("/home/project");
        assert_eq!(unwrap_str(call_in("abs", &[s("src/main")], dir).unwrap()), "/home/project/src/main");
        assert_eq!(unwrap_str(call_in("abs", &[s("/etc/hosts")], dir).unwrap()), "/etc/hosts");
    }

    #[test]
    fn unknown_method_errors() {
        assert!(call("nope", &[s("x")]).is_err());
    }
}
