//! Wire protocol for subprocess plugins.
//!
//! A plugin is an external executable that speaks newline-delimited JSON over
//! stdio. The host sends one request per line and reads exactly one response line.
//! Two operations exist:
//!
//! - `describe` — `{"op":"describe"}` → `{"name":"<module>","methods":[<sig>,…]}`
//! - `call` — `{"op":"call","method":"<m>","args":[<arg>,…]}` →
//!   `{"ok":<value>}` on success or `{"err":"<message>"}` on failure.
//!
//! The host-facing [`Value`](crate::eval::Value) and [`MethodSig`](crate::modules::MethodSig)
//! types are translated to and from the serde-friendly wire types defined here so
//! the core types never need serde derives of their own.

use serde::{Deserialize, Serialize};

use crate::eval::{FileEntry, Value};
use crate::modules::{MethodSig, NamedParam, Param, ResolvedArg, ValueTy};

// ── Value types on the wire ─────────────────────────────────────────────────────

/// A [`ValueTy`] tag as the single lowercase word used on the wire
/// (`"string"`, `"bool"`, `"list"`, `"fileset"`, `"any"`). Only the in-crate test
/// plugin re-encodes signatures; real plugins only ever produce these tags.
#[cfg(test)]
fn ty_to_wire(ty: ValueTy) -> &'static str {
    match ty {
        ValueTy::String => "string",
        ValueTy::Int => "int",
        ValueTy::Bool => "bool",
        ValueTy::List => "list",
        ValueTy::FileSet => "fileset",
        ValueTy::Any => "any",
    }
}

/// Parse a wire type tag, or `None` if it is not a recognized [`ValueTy`].
fn ty_from_wire(s: &str) -> Option<ValueTy> {
    Some(match s {
        "string" => ValueTy::String,
        "int" => ValueTy::Int,
        "bool" => ValueTy::Bool,
        "list" => ValueTy::List,
        "fileset" => ValueTy::FileSet,
        "any" => ValueTy::Any,
        _ => return None,
    })
}

/// One file entry inside a [`WireValue::Fileset`].
#[derive(Debug, Serialize, Deserialize)]
pub struct WireFile {
    pub path: String,
    pub name: String,
    pub stem: String,
    pub ext: String,
    pub dir: String,
}

/// A [`Value`] encoded for the wire as an internally tagged object, e.g.
/// `{"type":"string","value":"hi"}` or `{"type":"list","value":[…]}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum WireValue {
    String(String),
    Int(i64),
    Bool(bool),
    List(Vec<WireValue>),
    Fileset(Vec<WireFile>),
}

impl WireValue {
    /// Encode a host [`Value`] for transmission to a plugin.
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::String(s) => WireValue::String(s.clone()),
            Value::Int(n) => WireValue::Int(*n),
            Value::Bool(b) => WireValue::Bool(*b),
            Value::List(items) => {
                WireValue::List(items.iter().map(WireValue::from_value).collect())
            }
            Value::FileSet(entries) => WireValue::Fileset(
                entries
                    .iter()
                    .map(|e| WireFile {
                        path: e.path.display().to_string(),
                        name: e.name.clone(),
                        stem: e.stem.clone(),
                        ext: e.ext.clone(),
                        dir: e.dir.display().to_string(),
                    })
                    .collect(),
            ),
        }
    }

    /// Decode a plugin-supplied value back into a host [`Value`].
    pub fn into_value(self) -> Value {
        match self {
            WireValue::String(s) => Value::String(s),
            WireValue::Int(n) => Value::Int(n),
            WireValue::Bool(b) => Value::Bool(b),
            WireValue::List(items) => {
                Value::List(items.into_iter().map(WireValue::into_value).collect())
            }
            WireValue::Fileset(files) => Value::FileSet(
                files
                    .into_iter()
                    .map(|f| FileEntry {
                        path: f.path.into(),
                        name: f.name,
                        stem: f.stem,
                        ext: f.ext,
                        dir: f.dir.into(),
                    })
                    .collect(),
            ),
        }
    }
}

/// A single call argument on the wire — positional when `name` is absent.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireArg {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    pub value: WireValue,
}

impl WireArg {
    /// Encode an already-evaluated [`ResolvedArg`] for transmission.
    pub fn from_resolved(arg: &ResolvedArg) -> Self {
        WireArg { name: arg.name.clone(), value: WireValue::from_value(&arg.value) }
    }
}

// ── Method signatures on the wire ───────────────────────────────────────────────

/// A positional parameter as described by a plugin.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireParam {
    pub name: String,
    #[serde(rename = "type", default = "default_ty")]
    pub ty: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

/// A keyword parameter as described by a plugin.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireNamedParam {
    pub name: String,
    #[serde(rename = "type", default = "default_ty")]
    pub ty: String,
    #[serde(default)]
    pub required: bool,
}

/// A method signature as described by a plugin's `describe` response.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireMethodSig {
    pub name: String,
    #[serde(default)]
    pub params: Vec<WireParam>,
    #[serde(default)]
    pub named: Vec<WireNamedParam>,
    #[serde(default = "default_ty")]
    pub returns: String,
}

fn default_ty() -> String {
    "any".to_string()
}

fn default_true() -> bool {
    true
}

impl WireMethodSig {
    /// Convert into the host [`MethodSig`], validating every declared type tag.
    /// Returns the offending tag (as `Err`) when a parameter or the return type
    /// names a type the host does not recognize.
    pub fn into_sig(self) -> std::result::Result<MethodSig, String> {
        let params = self
            .params
            .into_iter()
            .map(|p| Ok(Param { ty: parse_ty(&p.ty)?, name: p.name, required: p.required }))
            .collect::<std::result::Result<Vec<_>, String>>()?;
        let named = self
            .named
            .into_iter()
            .map(|p| Ok(NamedParam { ty: parse_ty(&p.ty)?, name: p.name, required: p.required }))
            .collect::<std::result::Result<Vec<_>, String>>()?;
        Ok(MethodSig { name: self.name, params, named, returns: parse_ty(&self.returns)? })
    }
}

/// Encode a host [`MethodSig`] on the wire — used only by the in-crate test plugin.
#[cfg(test)]
pub fn sig_to_wire(sig: &MethodSig) -> WireMethodSig {
    WireMethodSig {
        name: sig.name.clone(),
        params: sig
            .params
            .iter()
            .map(|p| WireParam {
                name: p.name.clone(),
                ty: ty_to_wire(p.ty).to_string(),
                required: p.required,
            })
            .collect(),
        named: sig
            .named
            .iter()
            .map(|p| WireNamedParam {
                name: p.name.clone(),
                ty: ty_to_wire(p.ty).to_string(),
                required: p.required,
            })
            .collect(),
        returns: ty_to_wire(sig.returns).to_string(),
    }
}

fn parse_ty(s: &str) -> std::result::Result<ValueTy, String> {
    ty_from_wire(s).ok_or_else(|| format!("unknown value type '{}'", s))
}

// ── Requests and responses ──────────────────────────────────────────────────────

/// A request sent to a plugin process, serialized as one JSON line.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    /// Ask the plugin to report its module name and method signatures.
    Describe,
    /// Invoke a method with already-evaluated arguments.
    Call { method: String, args: Vec<WireArg> },
}

/// The plugin's reply to a `describe` request.
#[derive(Debug, Deserialize)]
pub struct DescribeResponse {
    pub name: String,
    #[serde(default)]
    pub methods: Vec<WireMethodSig>,
}

/// The plugin's reply to a `call` request: exactly one of `ok` / `err` is set.
#[derive(Debug, Deserialize)]
pub struct CallResponse {
    #[serde(default)]
    pub ok: Option<WireValue>,
    #[serde(default)]
    pub err: Option<String>,
}
