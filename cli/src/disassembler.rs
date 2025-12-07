//! file: cli/src/disassembler.rs
//! description: simple bytecode disassembler used by the CLI.
//!
//! Converts the compact IR/bytecode format into a human-readable text form
//! for debugging and inspection. This is intentionally lightweight and not
//! intended as a full-fidelity decompiler.
//!
use std::collections::HashMap;
use std::io::Cursor;

use mainstage_core::ir::op::IROp;
use mainstage_core::ir::value::Value as IRValue;

pub fn disassemble(bytes: &[u8]) -> Result<String, String> {
    let mut cur = Cursor::new(bytes);
    let mut out = String::new();

    use std::io::Read;

    // Get magic
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic)
        .map_err(|e| format!("missing header: {}", e))?;
    out.push_str(&format!("Magic: {}\n", String::from_utf8_lossy(&magic)));

    // Get version
    let version = read_u32(&mut cur)?;
    out.push_str(&format!("Version: {}\n", version));

    // Get op count
    let op_count = read_u32(&mut cur)? as usize;
    out.push_str(&format!("Op count: {}\n\n", op_count));

    // First pass: parse all ops into a vector and collect label positions
    let mut ops: Vec<IROp> = Vec::with_capacity(op_count);
    let mut label_map: HashMap<usize, String> = HashMap::new();
    // Also record labels by ordinal (the Nth Label seen)
    let mut label_ordinals: Vec<String> = Vec::new();

    for i in 0..op_count {
        ops.push(convert_byte_to_opcode(&mut cur, i, &mut label_map, &mut label_ordinals)?);
    }

    // Build mapping name -> ordinal for nicer label printing
    let mut name_to_ord: HashMap<String, usize> = HashMap::new();
    for (idx, name) in label_ordinals.iter().enumerate() {
        name_to_ord.insert(name.clone(), idx);
    }

    // Second pass: render ops, resolving numeric targets to label names when possible
    for (i, op) in ops.iter().enumerate() {
        match op {
            IROp::LConst { dest, value } => {
                out.push_str(&format!("{:04}  LConst r{} <- {:?}\n", i, dest, value))
            }
            IROp::LLocal { dest, local_index } => out.push_str(&format!(
                "{:04}  LLocal r{} <- local[{}]\n",
                i, dest, local_index
            )),
            IROp::SLocal { src, local_index } => out.push_str(&format!(
                "{:04}  SLocal local[{}] <- r{}\n",
                i, local_index, src
            )),
            IROp::Add { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Add r{} <- r{} + r{}\n",
                i, dest, src1, src2
            )),
            IROp::Sub { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Sub r{} <- r{} - r{}\n",
                i, dest, src1, src2
            )),
            IROp::Mul { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Mul r{} <- r{} * r{}\n",
                i, dest, src1, src2
            )),
            IROp::Div { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Div r{} <- r{} / r{}\n",
                i, dest, src1, src2
            )),
            IROp::Mod { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Mod r{} <- r{} % r{}\n",
                i, dest, src1, src2
            )),
            IROp::Eq { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Eq r{} <- r{} == r{}\n",
                i, dest, src1, src2
            )),
            IROp::Neq { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Neq r{} <- r{} != r{}\n",
                i, dest, src1, src2
            )),
            IROp::Lt { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Lt r{} <- r{} < r{}\n",
                i, dest, src1, src2
            )),
            IROp::Lte { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Lte r{} <- r{} <= r{}\n",
                i, dest, src1, src2
            )),
            IROp::Gt { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Gt r{} <- r{} > r{}\n",
                i, dest, src1, src2
            )),
            IROp::Gte { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Gte r{} <- r{} >= r{}\n",
                i, dest, src1, src2
            )),
            IROp::And { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  And r{} <- r{} && r{}\n",
                i, dest, src1, src2
            )),
            IROp::Or { dest, src1, src2 } => out.push_str(&format!(
                "{:04}  Or r{} <- r{} || r{}\n",
                i, dest, src1, src2
            )),
            IROp::Not { dest, src } => {
                out.push_str(&format!("{:04}  Not r{} <- !r{}\n", i, dest, src))
            }
            IROp::Inc { dest } => out.push_str(&format!("{:04}  Inc r{} ++\n", i, dest)),
            IROp::Dec { dest } => out.push_str(&format!("{:04}  Dec r{} --\n", i, dest)),
            IROp::Label { name } => {
                if let Some(ord) = name_to_ord.get(name) {
                    out.push_str(&format!(
                        "{:04}  Label {} (op {}, ord {})\n",
                        i, name, i, ord
                    ));
                } else {
                    out.push_str(&format!("{:04}  Label {} (op {})\n", i, name, i));
                }
            }
            IROp::Jump { target } => {
                let t = *target;
                if let Some(name) = label_map.get(&t) {
                    out.push_str(&format!("{:04}  Jump {} ({})\n", i, name, t));
                } else if let Some(name) = label_ordinals.get(t) {
                    out.push_str(&format!("{:04}  Jump {} (ord:{})\n", i, name, t));
                } else {
                    out.push_str(&format!("{:04}  Jump L{}\n", i, t));
                }
            }
            IROp::BrTrue { cond, target } => {
                let t = *target;
                if let Some(name) = label_map.get(&t) {
                    out.push_str(&format!("{:04}  BrTrue r{} -> {} ({})\n", i, cond, name, t));
                } else if let Some(name) = label_ordinals.get(t) {
                    out.push_str(&format!(
                        "{:04}  BrTrue r{} -> {} (ord:{})\n",
                        i, cond, name, t
                    ));
                } else {
                    out.push_str(&format!("{:04}  BrTrue r{} -> L{}\n", i, cond, t));
                }
            }
            IROp::BrFalse { cond, target } => {
                let t = *target;
                if let Some(name) = label_map.get(&t) {
                    out.push_str(&format!(
                        "{:04}  BrFalse r{} -> {} ({})\n",
                        i, cond, name, t
                    ));
                } else if let Some(name) = label_ordinals.get(t) {
                    out.push_str(&format!(
                        "{:04}  BrFalse r{} -> {} (ord:{})\n",
                        i, cond, name, t
                    ));
                } else {
                    out.push_str(&format!("{:04}  BrFalse r{} -> L{}\n", i, cond, t));
                }
            }
            IROp::Halt => out.push_str(&format!("{:04}  Halt\n", i)),
            IROp::AllocClosure { dest } => {
                out.push_str(&format!("{:04}  AllocClosure r{}\n", i, dest))
            }
            IROp::CStore {
                closure,
                field,
                src,
            } => out.push_str(&format!(
                "{:04}  CStore clo[r{}].{} <- r{}\n",
                i, closure, field, src
            )),
            IROp::CLoad {
                dest,
                closure,
                field,
            } => out.push_str(&format!(
                "{:04}  CLoad r{} <- clo[r{}].{}\n",
                i, dest, closure, field
            )),
            IROp::ArrayNew { dest, elems } => {
                out.push_str(&format!("{:04}  ArrayNew r{} <- [", i, dest));
                for (j, r) in elems.iter().enumerate() {
                    if j > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&format!("r{}", r));
                }
                out.push_str("]\n");
            }
            IROp::LoadGlobal { dest, src } => {
                out.push_str(&format!("{:04}  LoadGlobal r{} <- r{}\n", i, dest, src))
            }
            IROp::ArrayGet { dest, array, index } => out.push_str(&format!(
                "{:04}  ArrayGet r{} <- r{}[r{}]\n",
                i, dest, array, index
            )),
            IROp::ArraySet { array, index, src } => out.push_str(&format!(
                "{:04}  ArraySet r{}[r{}] <- r{}\n",
                i, array, index, src
            )),
            IROp::GetProp { dest, obj, key } => out.push_str(&format!(
                "{:04}  GetProp r{} <- r{}.r{}\n",
                i, dest, obj, key
            )),
            IROp::SetProp { obj, key, src } => out.push_str(&format!(
                "{:04}  SetProp r{}.r{} <- r{}\n",
                i, obj, key, src
            )),
            IROp::CallLabel {
                dest,
                label_index,
                args,
            } => {
                let t = *label_index;
                if let Some(name) = label_map.get(&t) {
                    out.push_str(&format!("{:04}  CallLabel r{} <- {}(", i, dest, name));
                } else {
                    out.push_str(&format!(
                        "{:04}  CallLabel r{} <- L{}(",
                        i, dest, label_index
                    ));
                }
                for (j, a) in args.iter().enumerate() {
                    if j > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&format!("r{}", a));
                }
                out.push_str(")\n");
            }
            IROp::PluginCall { dest, plugin_name, func_name, args } => {
                if let Some(d) = dest {
                    out.push_str(&format!("{:04}  PluginCall r{} <- {}.{}(", i, d, plugin_name, func_name));
                } else {
                    out.push_str(&format!("{:04}  PluginCall <- {}.{}(", i, plugin_name, func_name));
                }
                for (j, a) in args.iter().enumerate() {
                    if j > 0 { out.push_str(", "); }
                    out.push_str(&format!("r{}", a));
                }
                out.push_str(" )\n");
            }
            IROp::Ret { src } => out.push_str(&format!("{:04}  Ret r{}\n", i, src)),
        }
    }

    Ok(out)
}

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

fn read_string(cur: &mut Cursor<&[u8]>) -> Result<String, String> {
    let len = read_u32(cur)? as usize;
    let mut buf = vec![0u8; len];
    use std::io::Read;
    cur.read_exact(&mut buf)
        .map_err(|e| format!("unexpected eof reading string: {}", e))?;
    String::from_utf8(buf).map_err(|e| format!("invalid utf8: {}", e))
}

fn read_parsed_value(cur: &mut Cursor<&[u8]>) -> Result<IRValue, String> {
    use std::io::Read;
    let mut tag = [0u8; 1];
    cur.read_exact(&mut tag)
        .map_err(|e| format!("eof reading value tag: {}", e))?;
    match tag[0] {
        0x01 => {
            let mut ib = [0u8; 8];
            cur.read_exact(&mut ib)
                .map_err(|e| format!("eof int: {}", e))?;
            Ok(IRValue::Int(u64::from_le_bytes(ib) as i64))
        }
        0x02 => {
            let mut fb = [0u8; 8];
            cur.read_exact(&mut fb)
                .map_err(|e| format!("eof float: {}", e))?;
            Ok(IRValue::Float(f64::from_bits(u64::from_le_bytes(fb))))
        }
        0x03 => {
            let mut vb = [0u8; 1];
            cur.read_exact(&mut vb)
                .map_err(|e| format!("eof bool: {}", e))?;
            Ok(IRValue::Bool(vb[0] != 0))
        }
        0x04 => {
            let s = read_string(cur)?;
            Ok(IRValue::Str(s))
        }
        0x05 => {
            let s = read_string(cur)?;
            Ok(IRValue::Symbol(s))
        }
        0x06 => {
            let len = read_u32(cur)?;
            let mut items = Vec::new();
            for _ in 0..len {
                items.push(read_parsed_value(cur)?);
            }
            Ok(IRValue::Array(items))
        }
        0x08 => {
            let entries = read_u32(cur)? as usize;
            let mut map = HashMap::new();
            for _ in 0..entries {
                let key = read_string(cur)?;
                let val = read_parsed_value(cur)?;
                map.insert(key, val);
            }
            Ok(IRValue::Object(map))
        }
        0x07 => Ok(IRValue::Null),
        other => Err(format!("unknown value tag 0x{:02x}", other)),
    }
}

fn convert_byte_to_opcode(cur: &mut Cursor<&[u8]>, i: usize, label_map: &mut HashMap<usize, String>, label_ordinals: &mut Vec<String>) -> Result<IROp, String> {
    let code = read_u8(cur)?;
    let parsed = match code {
        0x01 => {
            let dest = read_u32(cur)?;
            let v = read_parsed_value(cur)?;
            IROp::LConst {
                dest: dest as usize,
                value: v,
            }
        }
        0x02 => {
            let dest = read_u32(cur)?;
            let local = read_u32(cur)?;
            IROp::LLocal {
                dest: dest as usize,
                local_index: local as usize,
            }
        }
        0x03 => {
            let src = read_u32(cur)?;
            let local = read_u32(cur)?;
            IROp::SLocal {
                src: src as usize,
                local_index: local as usize,
            }
        }
        0x10 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Add {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x11 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Sub {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x12 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Mul {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x13 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Div {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x14 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Mod {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x20 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Eq {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x21 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Neq {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x22 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Lt {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x23 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Lte {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x24 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Gt {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x25 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Gte {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x26 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::And {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x27 => {
            let dest = read_u32(cur)?;
            let a = read_u32(cur)?;
            let b = read_u32(cur)?;
            IROp::Or {
                dest: dest as usize,
                src1: a as usize,
                src2: b as usize,
            }
        }
        0x28 => {
            let dest = read_u32(cur)?;
            let src = read_u32(cur)?;
            IROp::Not {
                dest: dest as usize,
                src: src as usize,
            }
        }
        0x30 => {
            let dest = read_u32(cur)?;
            IROp::Inc {
                dest: dest as usize,
            }
        }
        0x31 => {
            let dest = read_u32(cur)?;
            IROp::Dec {
                dest: dest as usize,
            }
        }
        0x40 => {
            let name = read_string(cur)?;
            IROp::Label { name }
        }
        0x41 => {
            let target = read_u32(cur)?;
            IROp::Jump {
                target: target as usize,
            }
        }
        0x42 => {
            let cond = read_u32(cur)?;
            let target = read_u32(cur)?;
            IROp::BrTrue {
                cond: cond as usize,
                target: target as usize,
            }
        }
        0x43 => {
            let cond = read_u32(cur)?;
            let target = read_u32(cur)?;
            IROp::BrFalse {
                cond: cond as usize,
                target: target as usize,
            }
        }
        0x50 => IROp::Halt,
        0x60 => {
            let dest = read_u32(cur)?;
            IROp::AllocClosure {
                dest: dest as usize,
            }
        }
        0x61 => {
            let clo = read_u32(cur)?;
            let field = read_u32(cur)?;
            let src = read_u32(cur)?;
            IROp::CStore {
                closure: clo as usize,
                field: field as usize,
                src: src as usize,
            }
        }
        0x62 => {
            let dest = read_u32(cur)?;
            let clo = read_u32(cur)?;
            let field = read_u32(cur)?;
            IROp::CLoad {
                dest: dest as usize,
                closure: clo as usize,
                field: field as usize,
            }
        }
        0x71 => {
            let dest = read_u32(cur)?;
            let lbl = read_u32(cur)?;
            let argc = read_u32(cur)?;
            let mut args = Vec::new();
            for _ in 0..argc {
                args.push(read_u32(cur)? as usize);
            }
            IROp::CallLabel {
                dest: dest as usize,
                label_index: lbl as usize,
                args,
            }
        }
        0x80 => {
            let src = read_u32(cur)?;
            IROp::Ret { src: src as usize }
        }
        0x90 => {
            let dest = read_u32(cur)?;
            let len = read_u32(cur)?;
            let mut elems = Vec::new();
            for _ in 0..len {
                elems.push(read_u32(cur)? as usize);
            }
            IROp::ArrayNew {
                dest: dest as usize,
                elems,
            }
        }
        0x91 => {
            let dest = read_u32(cur)?;
            let array = read_u32(cur)?;
            let index = read_u32(cur)?;
            IROp::ArrayGet {
                dest: dest as usize,
                array: array as usize,
                index: index as usize,
            }
        }
        0x92 => {
            let array = read_u32(cur)?;
            let index = read_u32(cur)?;
            let src = read_u32(cur)?;
            IROp::ArraySet {
                array: array as usize,
                index: index as usize,
                src: src as usize,
            }
        }
        0x93 => {
            let dest = read_u32(cur)?;
            let obj = read_u32(cur)?;
            let key = read_u32(cur)?;
            IROp::GetProp {
                dest: dest as usize,
                obj: obj as usize,
                key: key as usize,
            }
        }
        0x94 => {
            let obj = read_u32(cur)?;
            let key = read_u32(cur)?;
            let src = read_u32(cur)?;
            IROp::SetProp {
                obj: obj as usize,
                key: key as usize,
                src: src as usize,
            }
        }
        0x95 => {
            let dest = read_u32(cur)?;
            let src = read_u32(cur)?;
            IROp::LoadGlobal {
                dest: dest as usize,
                src: src as usize,
            }
        }
        other => return Err(format!("unknown opcode 0x{:02x} at op {}", other, i)),
    };

    // if this op was a Label, record it against this op index and its ordinal
    if let IROp::Label { name } = &parsed {
        label_map.insert(i, name.clone());
        label_ordinals.push(name.clone());
    }

    Ok(parsed)
}
