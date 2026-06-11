//! `hash` module — SHA-256 hashing, reusing the Phase 7 (change-detection) hasher.

use std::sync::LazyLock;

use crate::cache::sha256_hex;
use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    MethodSig, Module, ModuleCx, Param, ResolvedArg, ValueTy, require_positional_string,
    resolve_path,
};

/// `hash.sha256("text")`, `hash.sha256_file("path")`.
pub struct HashModule;

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig {
            name: "sha256".to_string(),
            params: vec![Param { name: "text".to_string(), ty: ValueTy::String, required: true }],
            named: vec![],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "sha256_file".to_string(),
            params: vec![Param { name: "path".to_string(), ty: ValueTy::String, required: true }],
            named: vec![],
            returns: ValueTy::String,
        },
    ]
});

impl Module for HashModule {
    fn name(&self) -> &str {
        "hash"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "sha256" => {
                let text = require_positional_string(args, 0, "hash.sha256", cx)?;
                Ok(Value::String(sha256_hex(text.as_bytes())))
            }
            "sha256_file" => {
                let p = require_positional_string(args, 0, "hash.sha256_file", cx)?;
                let path = resolve_path(cx.script_dir, &p);
                let bytes = std::fs::read(&path).map_err(|e| {
                    cx.error(format!("hash.sha256_file '{}': {}", path.display(), e))
                })?;
                Ok(Value::String(sha256_hex(&bytes)))
            }
            _ => Err(cx.error(format!("hash has no method '{}'", method))),
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

    fn call_in(method: &str, args: &[ResolvedArg], dir: &Path) -> Result<Value> {
        let span = span();
        let cx = ModuleCx {
            span: &span,
            script_dir: dir,
            permissions: crate::modules::Permissions::all(),
        };
        HashModule.call(method, args, &cx)
    }

    fn s(v: &str) -> ResolvedArg {
        ResolvedArg { name: None, value: Value::String(v.to_string()) }
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256 of the empty string.
        let r = call_in("sha256", &[s("")], Path::new(".")).unwrap();
        assert!(matches!(r, Value::String(x)
            if x == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
    }

    #[test]
    fn sha256_file_matches_string_hash() {
        let dir = std::env::temp_dir();
        let file = dir.join("ms_hash_test.txt");
        std::fs::write(&file, "abc").unwrap();

        let from_file = call_in("sha256_file", &[s("ms_hash_test.txt")], &dir).unwrap();
        let from_str = call_in("sha256", &[s("abc")], Path::new(".")).unwrap();
        match (from_file, from_str) {
            (Value::String(a), Value::String(b)) => assert_eq!(a, b),
            _ => panic!("expected strings"),
        }
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn sha256_file_missing_errors() {
        assert!(call_in("sha256_file", &[s("does_not_exist_xyz")], Path::new(".")).is_err());
    }
}
