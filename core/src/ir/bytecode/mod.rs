//! Simple IR bytecode emitter
//!
//! This module serializes a lowered `IrModule` into a compact, transportable
//! byte sequence. The format is intentionally simple (magic, version, ops),
//! suitable for testing and prototyping a bytecode runtime.

use crate::ir::op::IROp;
use crate::ir::value::Value;

/// Serialize the provided IR module into a Vec<u8> containing the bytecode.
///
/// Format (little-endian):
/// - 4 bytes: magic: b"MSBC"
/// - 4 bytes: u32 version (1)
/// - 4 bytes: u32 op_count
/// - sequence of ops: each op is encoded as: 1 byte opcode + payload
///
/// The payload uses u32 for register/local indices and lengths, and
/// strings are encoded as u32 length + UTF-8 bytes.
pub fn emit_bytecode(module: &crate::ir::module::IrModule) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    // header
    out.extend_from_slice(b"MSBC");
    out.extend_from_slice(&1u32.to_le_bytes());

    // number of ops
    let op_count = module.ops.len() as u32;
    out.extend_from_slice(&op_count.to_le_bytes());

    for op in module.ops.iter() {
        match op {
            IROp::LConst { dest, value } => {
                out.push(0x01);
                write_u32(&mut out, *dest as u32);
                write_value(&mut out, value);
            }
            IROp::LLocal { dest, local_index } => {
                out.push(0x02);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *local_index as u32);
            }
            IROp::SLocal { src, local_index } => {
                out.push(0x03);
                write_u32(&mut out, *src as u32);
                write_u32(&mut out, *local_index as u32);
            }
            IROp::Add { dest, src1, src2 } => {
                out.push(0x10);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Sub { dest, src1, src2 } => {
                out.push(0x11);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Mul { dest, src1, src2 } => {
                out.push(0x12);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Div { dest, src1, src2 } => {
                out.push(0x13);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Mod { dest, src1, src2 } => {
                out.push(0x14);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Eq { dest, src1, src2 } => {
                out.push(0x20);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Neq { dest, src1, src2 } => {
                out.push(0x21);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Lt { dest, src1, src2 } => {
                out.push(0x22);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Lte { dest, src1, src2 } => {
                out.push(0x23);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Gt { dest, src1, src2 } => {
                out.push(0x24);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Gte { dest, src1, src2 } => {
                out.push(0x25);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::And { dest, src1, src2 } => {
                out.push(0x26);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Or { dest, src1, src2 } => {
                out.push(0x27);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src1 as u32);
                write_u32(&mut out, *src2 as u32);
            }
            IROp::Not { dest, src } => {
                out.push(0x28);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src as u32);
            }
            IROp::Inc { dest } => {
                out.push(0x30);
                write_u32(&mut out, *dest as u32);
            }
            IROp::Dec { dest } => {
                out.push(0x31);
                write_u32(&mut out, *dest as u32);
            }
            IROp::Label { name } => {
                out.push(0x40);
                write_string(&mut out, name);
            }
            IROp::Jump { target } => {
                out.push(0x41);
                write_u32(&mut out, *target as u32);
            }
            IROp::BrTrue { cond, target } => {
                out.push(0x42);
                write_u32(&mut out, *cond as u32);
                write_u32(&mut out, *target as u32);
            }
            IROp::BrFalse { cond, target } => {
                out.push(0x43);
                write_u32(&mut out, *cond as u32);
                write_u32(&mut out, *target as u32);
            }
            IROp::Halt => {
                out.push(0x50);
            }
            IROp::AllocClosure { dest } => {
                out.push(0x60);
                write_u32(&mut out, *dest as u32);
            }
            IROp::CStore {
                closure,
                field,
                src,
            } => {
                out.push(0x61);
                write_u32(&mut out, *closure as u32);
                write_u32(&mut out, *field as u32);
                write_u32(&mut out, *src as u32);
            }
            IROp::CLoad {
                dest,
                closure,
                field,
            } => {
                out.push(0x62);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *closure as u32);
                write_u32(&mut out, *field as u32);
            }
            IROp::CallLabel {
                dest,
                label_index,
                args,
            } => {
                out.push(0x71);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *label_index as u32);
                write_u32(&mut out, args.len() as u32);
                for a in args.iter() {
                    write_u32(&mut out, *a as u32);
                }
            }
            IROp::PluginCall { dest, plugin_name, func_name, args } => {
                out.push(0x72);
                write_string(&mut out, plugin_name);
                write_string(&mut out, func_name);
                write_u32(&mut out, args.len() as u32);
                for a in args.iter() {
                    write_u32(&mut out, *a as u32);
                }
                match dest {
                    Some(d) => { write_u32(&mut out, 1); write_u32(&mut out, *d as u32); }
                    None => { write_u32(&mut out, 0); }
                }
            }
            IROp::Ret { src } => {
                out.push(0x80);
                write_u32(&mut out, *src as u32);
            }
            IROp::ArrayNew { dest, elems } => {
                out.push(0x90);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, elems.len() as u32);
                for r in elems.iter() {
                    write_u32(&mut out, *r as u32);
                }
            }
            IROp::LoadGlobal { dest, src } => {
                out.push(0x95);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *src as u32);
            }
            IROp::ArrayGet { dest, array, index } => {
                out.push(0x91);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *array as u32);
                write_u32(&mut out, *index as u32);
            }
            IROp::ArraySet { array, index, src } => {
                out.push(0x92);
                write_u32(&mut out, *array as u32);
                write_u32(&mut out, *index as u32);
                write_u32(&mut out, *src as u32);
            }
            IROp::GetProp { dest, obj, key } => {
                out.push(0x93);
                write_u32(&mut out, *dest as u32);
                write_u32(&mut out, *obj as u32);
                write_u32(&mut out, *key as u32);
            }
            IROp::SetProp { obj, key, src } => {
                out.push(0x94);
                write_u32(&mut out, *obj as u32);
                write_u32(&mut out, *key as u32);
                write_u32(&mut out, *src as u32);
            }
        }
    }

    out
}

 

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    write_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

fn write_value(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Int(i) => {
            out.push(0x01);
            write_u64(out, *i as u64);
        }
        Value::Float(f) => {
            out.push(0x02);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::Bool(b) => {
            out.push(0x03);
            out.push(if *b { 1 } else { 0 });
        }
        Value::Str(s) => {
            out.push(0x04);
            write_string(out, s);
        }
        Value::Symbol(s) => {
            out.push(0x05);
            write_string(out, s);
        }
        Value::Array(arr) => {
            out.push(0x06);
            write_u32(out, arr.len() as u32);
            for el in arr.iter() {
                write_value(out, el);
            }
        }
        Value::Object(map) => {
            out.push(0x08);
            write_u32(out, map.len() as u32);
            for (k, v) in map.iter() {
                write_string(out, k);
                write_value(out, v);
            }
        }
        Value::Null => {
            out.push(0x07);
        }
    }
}
