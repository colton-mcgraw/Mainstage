//! file: core/src/ir/value.rs
//! description: IR-level `Value` type used in the intermediate representation.
//!
//! This module defines the `Value` enum used by the IR phase. It includes
//! hashing and equality implementations that make IR values usable as map
//! keys in optimization passes.
//!
use std::hash::{Hash, Hasher};

/// # Value
/// 
/// Represents a value in the IR.
/// This can be a constant integer, float, string, boolean, array, object, etc.
/// This type is used in IR operations to represent constant values.
/// 
/// # Notes
/// `Value` cannot implement Hash or Eq directly if it contains floating-point numbers
/// due to the nature of floating-point equality. However, in this implementation,
/// we use the bit representation of floats for hashing and equality checks.
/// 
#[derive(Debug, Clone, PartialEq)]
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

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        use Value::*;
        match self {
            Int(i) => { 0u8.hash(state); i.hash(state); }
            Float(f) => { 1u8.hash(state); f.to_bits().hash(state); }
            Bool(b) => { 2u8.hash(state); b.hash(state); }
            Str(s) => { 3u8.hash(state); s.hash(state); }
            Symbol(s) => { 4u8.hash(state); s.hash(state); }
            Array(a) => { 5u8.hash(state); a.len().hash(state); for v in a.iter() { v.hash(state); } }
            Object(m) => {
                6u8.hash(state);
                // Sort keys for deterministic hashing
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                for k in keys.into_iter() {
                    k.hash(state);
                    m.get(k).unwrap().hash(state);
                }
            }
            Null => { 7u8.hash(state); }
        }
    }
}