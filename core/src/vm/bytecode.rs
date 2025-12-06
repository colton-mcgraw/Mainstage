//! file: core/src/vm/bytecode.rs
//! description: VM bytecode parser for the run-time executor.
//!
//! Parses the compact binary format emitted by the IR bytecode emitter and
//! produces a vector of runtime `Op` values and label maps consumed by the
//! executor.

use std::collections::HashMap;
use std::io::Cursor;

use crate::vm::{op::Op, value::Value};

pub(crate) fn parse_ops(
    bytes: &[u8],
) -> Result<(Vec<Op>, HashMap<usize, String>, HashMap<String, usize>), String> {
    use std::io::Read;
    let mut cur = Cursor::new(bytes);

    // header
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic)
        .map_err(|e| format!("missing header: {}", e))?;
    if &magic != b"MSBC" {
        return Err("invalid magic".to_string());
    }
    let version = read_u32(&mut cur)?;
    if version != 1 {
        return Err(format!("unsupported version {}", version));
    }

    let op_count = read_u32(&mut cur)? as usize;

    let mut ops: Vec<Op> = Vec::with_capacity(op_count);
    let mut label_pos: HashMap<usize, String> = HashMap::new();
    let mut label_by_name: HashMap<String, usize> = HashMap::new();

    for i in 0..op_count {
        let code = read_u8(&mut cur)?;
        match code {
            0x01 => {
                let dest = read_u32(&mut cur)? as usize;
                let val = read_value(&mut cur)?;
                ops.push(Op::LConst { dest, val });
            }
            0x02 => {
                let dest = read_u32(&mut cur)? as usize;
                let local = read_u32(&mut cur)? as usize;
                ops.push(Op::LLocal { dest, local });
            }
            0x03 => {
                let src = read_u32(&mut cur)? as usize;
                let local = read_u32(&mut cur)? as usize;
                ops.push(Op::SLocal { src, local });
            }
            0x10 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Add { dest, a, b });
            }
            0x11 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Sub { dest, a, b });
            }
            0x12 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Mul { dest, a, b });
            }
            0x13 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Div { dest, a, b });
            }
            0x14 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Mod { dest, a, b });
            }
            0x20 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Eq { dest, a, b });
            }
            0x21 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Neq { dest, a, b });
            }
            0x22 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Lt { dest, a, b });
            }
            0x23 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Lte { dest, a, b });
            }
            0x24 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Gt { dest, a, b });
            }
            0x25 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Gte { dest, a, b });
            }
            0x26 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::And { dest, a, b });
            }
            0x27 => {
                let dest = read_u32(&mut cur)? as usize;
                let a = read_u32(&mut cur)? as usize;
                let b = read_u32(&mut cur)? as usize;
                ops.push(Op::Or { dest, a, b });
            }
            0x28 => {
                let dest = read_u32(&mut cur)? as usize;
                let src = read_u32(&mut cur)? as usize;
                ops.push(Op::Not { dest, src });
            }
            0x30 => {
                let dest = read_u32(&mut cur)? as usize;
                ops.push(Op::Inc { dest });
            }
            0x31 => {
                let dest = read_u32(&mut cur)? as usize;
                ops.push(Op::Dec { dest });
            }
            0x40 => {
                let name = read_string(&mut cur)?;
                ops.push(Op::Label);
                label_pos.insert(i, name.clone());
                label_by_name.insert(name.clone(), i);
            }
            0x41 => {
                let target = read_u32(&mut cur)? as usize;
                ops.push(Op::Jump { target });
            }
            0x42 => {
                let cond = read_u32(&mut cur)? as usize;
                let target = read_u32(&mut cur)? as usize;
                ops.push(Op::BrTrue { cond, target });
            }
            0x43 => {
                let cond = read_u32(&mut cur)? as usize;
                let target = read_u32(&mut cur)? as usize;
                ops.push(Op::BrFalse { cond, target });
            }
            0x50 => {
                ops.push(Op::Halt);
            }
            0x60..=0x62 => {
                return Err("closure ops not supported in VM yet".to_string());
            }
            0x71 => {
                let dest = read_u32(&mut cur)? as usize;
                let lbl = read_u32(&mut cur)? as usize;
                let argc = read_u32(&mut cur)? as usize;
                let mut args = Vec::new();
                for _ in 0..argc {
                    args.push(read_u32(&mut cur)? as usize);
                }
                ops.push(Op::CallLabel {
                    dest,
                    label_index: lbl,
                    args,
                });
            }
            0x72 => {
                // PluginCall: plugin_name, func_name, argc, args..., has_result (u32 0/1), [result_reg]
                let plugin_name = read_string(&mut cur)?;
                let func_name = read_string(&mut cur)?;
                let argc = read_u32(&mut cur)? as usize;
                let mut args = Vec::new();
                for _ in 0..argc {
                    args.push(read_u32(&mut cur)? as usize);
                }
                let has_result = read_u32(&mut cur)?;
                let result = if has_result != 0 { Some(read_u32(&mut cur)? as usize) } else { None };
                ops.push(Op::PluginCall { plugin_name, func_name, args, result_target: result });
            }
            0x93 => {
                let dest = read_u32(&mut cur)? as usize;
                let obj = read_u32(&mut cur)? as usize;
                let key = read_u32(&mut cur)? as usize;
                ops.push(Op::GetProp { dest, obj, key });
            }
            0x94 => {
                let obj = read_u32(&mut cur)? as usize;
                let key = read_u32(&mut cur)? as usize;
                let src = read_u32(&mut cur)? as usize;
                ops.push(Op::SetProp { obj, key, src });
            }
            0x90 => {
                let dest = read_u32(&mut cur)? as usize;
                let len = read_u32(&mut cur)? as usize;
                let mut elems = Vec::new();
                for _ in 0..len {
                    elems.push(read_u32(&mut cur)? as usize);
                }
                ops.push(Op::ArrayNew { dest, elems });
            }
            0x95 => {
                let dest = read_u32(&mut cur)? as usize;
                let src = read_u32(&mut cur)? as usize;
                ops.push(Op::LoadGlobal { dest, src });
            }
            0x91 => {
                let dest = read_u32(&mut cur)? as usize;
                let array = read_u32(&mut cur)? as usize;
                let index = read_u32(&mut cur)? as usize;
                ops.push(Op::ArrayGet { dest, array, index });
            }
            0x92 => {
                let array = read_u32(&mut cur)? as usize;
                let index = read_u32(&mut cur)? as usize;
                let src = read_u32(&mut cur)? as usize;
                ops.push(Op::ArraySet { array, index, src });
            }
            0x80 => {
                let src = read_u32(&mut cur)? as usize;
                ops.push(Op::Ret { src });
            }
            other => return Err(format!("unknown opcode 0x{:02x} at op {}", other, i)),
        }
    }

    Ok((ops, label_pos, label_by_name))
}

// helpers for reading bytecode values (copied from bytecode emitter format)
fn read_u8(cur: &mut Cursor<&[u8]>) -> Result<u8, String> {
    use std::io::Read;
    let mut b = [0u8; 1];
    cur.read_exact(&mut b)
        .map_err(|e| format!("unexpected eof: {}", e))?;
    Ok(b[0])
}
fn read_u32(cur: &mut Cursor<&[u8]>) -> Result<u32, String> {
    use std::io::Read;
    let mut b = [0u8; 4];
    cur.read_exact(&mut b)
        .map_err(|e| format!("unexpected eof: {}", e))?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64(cur: &mut Cursor<&[u8]>) -> Result<u64, String> {
    use std::io::Read;
    let mut b = [0u8; 8];
    cur.read_exact(&mut b)
        .map_err(|e| format!("unexpected eof: {}", e))?;
    Ok(u64::from_le_bytes(b))
}
fn read_string(cur: &mut Cursor<&[u8]>) -> Result<String, String> {
    let len = read_u32(cur)? as usize;
    let mut buf = vec![0u8; len];
    use std::io::Read;
    cur.read_exact(&mut buf)
        .map_err(|e| format!("unexpected eof reading string: {}", e))?;
    String::from_utf8(buf).map_err(|e| format!("invalid utf8: {}", e))
}

fn read_value(cur: &mut Cursor<&[u8]>) -> Result<Value, String> {
    use std::io::Read;
    let mut tag = [0u8; 1];
    cur.read_exact(&mut tag)
        .map_err(|e| format!("eof reading value tag: {}", e))?;
    match tag[0] {
        0x01 => {
            let v = read_u64(cur)? as i64;
            Ok(Value::Int(v))
        }
        0x02 => {
            let bits = read_u64(cur)?;
            Ok(Value::Float(f64::from_bits(bits)))
        }
        0x03 => {
            let mut b = [0u8; 1];
            cur.read_exact(&mut b)
                .map_err(|e| format!("eof bool: {}", e))?;
            Ok(Value::Bool(b[0] != 0))
        }
        0x04 => {
            let s = read_string(cur)?;
            Ok(Value::Str(s))
        }
        0x05 => {
            let s = read_string(cur)?;
            Ok(Value::Symbol(s))
        }
        0x06 => {
            let len = read_u32(cur)? as usize;
            let mut items = Vec::new();
            for _ in 0..len {
                items.push(read_value(cur)?);
            }
            Ok(Value::Array(items))
        }
        0x08 => {
            let len = read_u32(cur)? as usize;
            let mut map = std::collections::HashMap::new();
            for _ in 0..len {
                let key = read_string(cur)?;
                let val = read_value(cur)?;
                map.insert(key, val);
            }
            Ok(Value::Object(map))
        }
        0x07 => Ok(Value::Null),
        other => Err(format!("unknown value tag 0x{:02x}", other)),
    }
}
