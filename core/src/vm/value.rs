//! file: core/src/vm/value.rs
//! description: runtime `Value` representation used by the VM.
//!
//! This module defines the `Value` enum used inside the VM executor and
//! runtime-facing APIs. Conversions to/from the IR `Value` live here as
//! well to centralize marshalling logic.

use crate::ir::value::Value as IrValue;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Symbol(String),
    Array(Vec<Value>),
    Object(std::collections::HashMap<String, Value>),
    Null,
}

impl From<IrValue> for Value {
    fn from(v: IrValue) -> Self {
        match v {
            IrValue::Int(i) => Value::Int(i),
            IrValue::Float(f) => Value::Float(f),
            IrValue::Bool(b) => Value::Bool(b),
            IrValue::Str(s) => Value::Str(s),
            IrValue::Symbol(s) => Value::Symbol(s),
            IrValue::Array(a) => Value::Array(a.into_iter().map(From::from).collect()),
            IrValue::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, v.into())).collect()),
            IrValue::Null => Value::Null,
        }
    }
}

impl Value {
    pub(crate) fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Symbol(_) => true,
            Value::Array(a) => !a.is_empty(),
            Value::Object(m) => !m.is_empty(),
            Value::Null => false,
        }
    }

    pub fn to_value(&self) -> IrValue {
        match self {
            Value::Int(i) => IrValue::Int(*i),
            Value::Float(f) => IrValue::Float(*f),
            Value::Bool(b) => IrValue::Bool(*b),
            Value::Str(s) => IrValue::Str(s.clone()),
            Value::Symbol(s) => IrValue::Symbol(s.clone()),
            Value::Array(a) => IrValue::Array(a.iter().map(|rv| rv.to_value()).collect()),
            Value::Object(m) => IrValue::Object(m.iter().map(|(k, v)| (k.clone(), v.to_value())).collect()),
            Value::Null => IrValue::Null,
        }
    }
}

// JSON marshalling helpers used by in-process plugin loader
impl Value {
    /// Convert `Value` to `serde_json::Value` for FFI-friendly transport.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Int(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
            Value::Float(f) => serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap_or(serde_json::Number::from(0))),
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::Symbol(s) => serde_json::Value::String(s.clone()),
            Value::Array(a) => serde_json::Value::Array(a.iter().map(|v| v.to_json()).collect()),
            Value::Object(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
            Value::Null => serde_json::Value::Null,
        }
    }

    /// Convert from `serde_json::Value` to VM `Value`.
    pub fn from_json(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => {
                if n.is_i64() {
                    Value::Int(n.as_i64().unwrap())
                } else if n.is_u64() {
                    // u64 might overflow i64, clamp
                    Value::Int(n.as_u64().unwrap() as i64)
                } else if n.is_f64() {
                    Value::Float(n.as_f64().unwrap())
                } else {
                    Value::Null
                }
            }
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Array(a) => Value::Array(a.iter().map(Value::from_json).collect()),
            serde_json::Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), Value::from_json(v))).collect()),
        }
    }
}

/// Helpers to marshall Vec<Value> into serde_json::Value (array)
pub fn values_to_json_array(vals: &[Value]) -> serde_json::Value {
    serde_json::Value::Array(vals.iter().map(|v| v.to_json()).collect())
}

pub fn json_to_value(v: &serde_json::Value) -> Value {
    Value::from_json(v)
}