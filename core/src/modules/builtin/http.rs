//! `http` module — make outbound HTTP(S) requests.
//!
//! Gated on the [`Net`](crate::modules::Capability::Net) capability: a script may only
//! reach the network when the user grants it with `--allow-net` or a manifest
//! `[permissions]` block. Non-2xx responses are reported as errors.

use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::modules::{
    require_positional_string, resolve_path, Capability, MethodSig, Module, ModuleCx, Param,
    ResolvedArg, ValueTy,
};

/// `http.get(url)` → response body; `http.download(url, path)` → the written path.
pub struct HttpModule;

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![
        MethodSig {
            name: "get".to_string(),
            params: vec![Param { name: "url".to_string(), ty: ValueTy::String, required: true }],
            named: vec![],
            returns: ValueTy::String,
        },
        MethodSig {
            name: "download".to_string(),
            params: vec![
                Param { name: "url".to_string(), ty: ValueTy::String, required: true },
                Param { name: "path".to_string(), ty: ValueTy::String, required: true },
            ],
            named: vec![],
            returns: ValueTy::String,
        },
    ]
});

impl Module for HttpModule {
    fn name(&self) -> &str {
        "http"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "get" => {
                cx.require(Capability::Net)?;
                let url = require_positional_string(args, 0, "http.get", cx)?;
                let body = fetch(&url, cx)?
                    .read_to_string()
                    .map_err(|e| cx.error(format!("http.get '{}': {}", url, e)))?;
                Ok(Value::String(body))
            }
            "download" => {
                cx.require(Capability::Net)?;
                let url = require_positional_string(args, 0, "http.download", cx)?;
                let dest = require_positional_string(args, 1, "http.download", cx)?;
                let bytes = fetch(&url, cx)?
                    .read_to_vec()
                    .map_err(|e| cx.error(format!("http.download '{}': {}", url, e)))?;
                let path = resolve_path(cx.script_dir, &dest);
                std::fs::write(&path, &bytes)
                    .map_err(|e| cx.error(format!("http.download '{}': {}", path.display(), e)))?;
                Ok(Value::String(path.to_string_lossy().into_owned()))
            }
            _ => Err(cx.error(format!("http has no method '{}'", method))),
        }
    }
}

/// Issue a GET and return the response body reader, mapping transport and non-2xx
/// status failures to a span-carrying error.
fn fetch(url: &str, cx: &ModuleCx) -> Result<ureq::Body> {
    let response =
        ureq::get(url).call().map_err(|e| cx.error(format!("http.get '{}': {}", url, e)))?;
    Ok(response.into_body())
}
