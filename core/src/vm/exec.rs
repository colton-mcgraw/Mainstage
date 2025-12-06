//! file: core/src/vm/exec.rs
//! description: bytecode executor implementation.
//!
//! This module implements the core interpreter loop for the VM. It decodes
//! `Op` values and executes them against an `ExecState` containing registers,
//! frames and plugin registry references.
//!
use std::collections::HashMap;
use crate::vm::{op::Op, value::Value};
use futures::executor::block_on;

fn coerce_to_f64(a: &Value) -> Option<f64> {
    match a {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        Value::Str(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn numeric_bin(
    a: &Value,
    b: &Value,
    int_op: fn(i64, i64) -> i64,
    float_op: fn(f64, f64) -> f64,
) -> Value {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Value::Int(int_op(*x, *y)),
        _ => {
            let ax = coerce_to_f64(a);
            let bx = coerce_to_f64(b);
            if let (Some(x), Some(y)) = (ax, bx) {
                Value::Float(float_op(x, y))
            } else {
                Value::Null
            }
        }
    }
}

pub struct Frame {
    pub locals: Vec<Value>,
    pub return_pc: Option<usize>,
    pub return_reg: Option<usize>,
}

pub(crate) fn ensure_reg(regs: &mut Vec<Value>, idx: usize) {
    if idx >= regs.len() {
        regs.resize_with(idx + 1, || Value::Null);
    }
}

pub(crate) fn take_args(regs: &Vec<Value>, args: &[usize]) -> Vec<Value> {
    let mut out = Vec::with_capacity(args.len());
    for &r in args.iter() {
        if r < regs.len() {
            out.push(regs[r].clone());
        } else {
            out.push(Value::Null);
        }
    }
    out
}

pub struct ExecState<'a> {
    pub ops: Vec<Op>,
    pub label_pos: HashMap<usize, String>,
    pub label_by_name: HashMap<String, usize>,
    pub regs: Vec<Value>,
    pub frames: Vec<Frame>,
    pub pc: usize,
    pub steps: usize,
    pub trace: bool,
    pub plugins: &'a crate::vm::plugin::PluginRegistry,
}

fn numeric_cmp(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        _ => {
            let ax = match a { Value::Int(i) => Some(*i as f64), Value::Float(f) => Some(*f), Value::Str(s) => s.parse::<f64>().ok(), _ => None };
            let bx = match b { Value::Int(i) => Some(*i as f64), Value::Float(f) => Some(*f), Value::Str(s) => s.parse::<f64>().ok(), _ => None };
            if let (Some(x), Some(y)) = (ax, bx) {
                if x < y { Some(std::cmp::Ordering::Less) } else if x > y { Some(std::cmp::Ordering::Greater) } else { Some(std::cmp::Ordering::Equal) }
            } else { None }
        }
    }
}

pub(crate) fn dispatch_op(state: &mut ExecState) -> Result<(), String> {
    let op = &state.ops[state.pc];
    match op {
        Op::LoadGlobal { dest, src } => {
            ensure_reg(&mut state.regs, *dest);
            if *src < state.regs.len() {
                state.regs[*dest] = state.regs[*src].clone();
            } else {
                state.regs[*dest] = Value::Null;
            }
            state.pc += 1;
        }
        Op::LConst { dest, val } => {
            ensure_reg(&mut state.regs, *dest);
            state.regs[*dest] = val.clone();
            state.pc += 1;
        }
        Op::LLocal { dest, local } => {
            ensure_reg(&mut state.regs, *dest);
            if let Some(frame) = state.frames.last() {
                if *local < frame.locals.len() {
                    state.regs[*dest] = frame.locals[*local].clone();
                } else {
                    state.regs[*dest] = Value::Null;
                }
            } else {
                state.regs[*dest] = Value::Null;
            }
            state.pc += 1;
        }
        Op::SLocal { src, local } => {
            ensure_reg(&mut state.regs, *src);
            if let Some(frame) = state.frames.last_mut() {
                if *local >= frame.locals.len() {
                    frame.locals.resize(*local + 1, Value::Null);
                }
                frame.locals[*local] = state.regs[*src].clone();
            }
            state.pc += 1;
        }
        Op::Label { .. } => {
            state.pc += 1;
        }
        Op::Add { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            let is_str = matches!(&state.regs[*a], Value::Str(_)) || matches!(&state.regs[*b], Value::Str(_));
            if is_str {
                fn val_to_string(v: &Value) -> String {
                    match v {
                        Value::Str(s) => s.clone(),
                        Value::Symbol(s) => s.clone(),
                        Value::Int(i) => i.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => "null".to_string(),
                        Value::Array(_) | Value::Object(_) => format!("{:?}", v.to_value()),
                    }
                }
                let s1 = val_to_string(&state.regs[*a]);
                let s2 = val_to_string(&state.regs[*b]);
                state.regs[*dest] = Value::Str(format!("{}{}", s1, s2));
            } else {
                state.regs[*dest] = numeric_bin(&state.regs[*a], &state.regs[*b], |x, y| x + y, |x, y| x + y);
            }
            state.pc += 1;
        }
        Op::Sub { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            state.regs[*dest] = numeric_bin(&state.regs[*a], &state.regs[*b], |x, y| x - y, |x, y| x - y);
            state.pc += 1;
        }
        Op::Mul { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            state.regs[*dest] = numeric_bin(&state.regs[*a], &state.regs[*b], |x, y| x * y, |x, y| x * y);
            state.pc += 1;
        }
        Op::Div { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            // integer division when both ints and evenly divisible
            match (&state.regs[*a], &state.regs[*b]) {
                (Value::Int(x), Value::Int(y)) => {
                    if *y != 0 && x % y == 0 {
                        state.regs[*dest] = Value::Int(x / y);
                    } else {
                        state.regs[*dest] = numeric_bin(&state.regs[*a], &state.regs[*b], |x, y| x / y, |x, y| x / y);
                    }
                }
                _ => {
                    state.regs[*dest] = numeric_bin(&state.regs[*a], &state.regs[*b], |x, y| x / y, |x, y| x / y);
                }
            }
            state.pc += 1;
        }
        Op::Mod { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            if let (Value::Int(x), Value::Int(y)) = (&state.regs[*a], &state.regs[*b]) {
                if *y != 0 {
                    state.regs[*dest] = Value::Int(x % y);
                } else {
                    state.regs[*dest] = Value::Null;
                }
            } else {
                state.regs[*dest] = Value::Null;
            }
            state.pc += 1;
        }
        Op::Eq { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            if let Some(ord) = numeric_cmp(&state.regs[*a], &state.regs[*b]) {
                state.regs[*dest] = Value::Bool(ord == std::cmp::Ordering::Equal);
            } else {
                state.regs[*dest] = Value::Bool(state.regs[*a].to_value() == state.regs[*b].to_value());
            }
            state.pc += 1;
        }
        Op::Neq { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            if let Some(ord) = numeric_cmp(&state.regs[*a], &state.regs[*b]) {
                state.regs[*dest] = Value::Bool(ord != std::cmp::Ordering::Equal);
            } else {
                state.regs[*dest] = Value::Bool(state.regs[*a].to_value() != state.regs[*b].to_value());
            }
            state.pc += 1;
        }
        Op::Lt { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            if let Some(ord) = numeric_cmp(&state.regs[*a], &state.regs[*b]) {
                state.regs[*dest] = Value::Bool(ord == std::cmp::Ordering::Less);
            } else {
                state.regs[*dest] = Value::Bool(false);
            }
            state.pc += 1;
        }
        Op::Lte { dest, a, b } => {
            ensure_reg(&mut state.regs, *dest);
            if let Some(ord) = numeric_cmp(&state.regs[*a], &state.regs[*b]) {
                state.regs[*dest] = Value::Bool(ord != std::cmp::Ordering::Greater);
            } else {
                state.regs[*dest] = Value::Bool(false);
            }
            state.pc += 1;
        }
        Op::Gt { dest, a, b } => {
            ensure_reg(&mut state.regs, *dest);
            if let Some(ord) = numeric_cmp(&state.regs[*a], &state.regs[*b]) {
                state.regs[*dest] = Value::Bool(ord == std::cmp::Ordering::Greater);
            } else {
                state.regs[*dest] = Value::Bool(false);
            }
            state.pc += 1;
        }
        Op::Gte { dest, a, b } => {
            ensure_reg(&mut state.regs, *dest);
            if let Some(ord) = numeric_cmp(&state.regs[*a], &state.regs[*b]) {
                state.regs[*dest] = Value::Bool(ord != std::cmp::Ordering::Less);
            } else {
                state.regs[*dest] = Value::Bool(false);
            }
            state.pc += 1;
        }
        Op::And { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            let v = state.regs[*a].as_bool() && state.regs[*b].as_bool();
            state.regs[*dest] = Value::Bool(v);
            state.pc += 1;
        }
        Op::Or { dest, a, b } => {
            ensure_reg(&mut state.regs, *a);
            ensure_reg(&mut state.regs, *b);
            ensure_reg(&mut state.regs, *dest);
            let v = state.regs[*a].as_bool() || state.regs[*b].as_bool();
            state.regs[*dest] = Value::Bool(v);
            state.pc += 1;
        }
        Op::Not { dest, src } => {
            ensure_reg(&mut state.regs, *src);
            ensure_reg(&mut state.regs, *dest);
            state.regs[*dest] = Value::Bool(!state.regs[*src].as_bool());
            state.pc += 1;
        }
        Op::Inc { dest } => {
            ensure_reg(&mut state.regs, *dest);
            if let Value::Int(i) = &mut state.regs[*dest] {
                *i += 1;
            }
            state.pc += 1;
        }
        Op::Dec { dest } => {
            ensure_reg(&mut state.regs, *dest);
            if let Value::Int(i) = &mut state.regs[*dest] {
                *i -= 1;
            }
            state.pc += 1;
        }
        Op::Jump { target } => {
            state.pc = *target;
        }
        Op::BrTrue { cond, target } => {
            ensure_reg(&mut state.regs, *cond);
            if state.regs[*cond].as_bool() {
                state.pc = *target;
            } else {
                state.pc += 1;
            }
        }
        Op::BrFalse { cond, target } => {
            ensure_reg(&mut state.regs, *cond);
            if !state.regs[*cond].as_bool() {
                state.pc = *target;
            } else {
                state.pc += 1;
            }
        }
        Op::Halt => {
            state.pc = state.ops.len();
        }
        Op::ArrayNew { dest, elems } => {
            let mut items: Vec<Value> = Vec::new();
            for &r in elems.iter() { ensure_reg(&mut state.regs, r); items.push(state.regs[r].clone()); }
            ensure_reg(&mut state.regs, *dest);
            state.regs[*dest] = Value::Array(items);
            state.pc += 1;
        }
        Op::ArrayGet { dest, array, index } => {
            ensure_reg(&mut state.regs, *array);
            ensure_reg(&mut state.regs, *index);
            ensure_reg(&mut state.regs, *dest);
            let arr_val = state.regs[*array].clone();
            let idx_val = state.regs[*index].clone();
            
            match arr_val {
                Value::Array(a) => {
                    if let Value::Int(i) = idx_val {
                        let idx = i as isize;
                        if idx >= 0 && (idx as usize) < a.len() { state.regs[*dest] = a[idx as usize].clone(); } else { state.regs[*dest] = Value::Null; }
                    } else { state.regs[*dest] = Value::Null; }
                }
                _ => { state.regs[*dest] = Value::Null; }
            }
            state.pc += 1;
        }
        Op::ArraySet { array, index, src } => {
            ensure_reg(&mut state.regs, *array);
            ensure_reg(&mut state.regs, *index);
            ensure_reg(&mut state.regs, *src);
            let idx_val = state.regs[*index].clone();
            let src_val = state.regs[*src].clone();
            match &mut state.regs[*array] {
                Value::Array(a) => {
                    if let Value::Int(i) = idx_val { let idx = i as usize; if idx >= a.len() { a.resize(idx + 1, Value::Null); } a[idx] = src_val; }
                }
                other => {
                    if let Value::Int(i) = idx_val { let idx = i as usize; let mut a: Vec<Value> = Vec::new(); a.resize(idx + 1, Value::Null); a[idx] = src_val; *other = Value::Array(a); }
                }
            }
            state.pc += 1;
        }
        Op::GetProp { dest, obj, key } => {
            ensure_reg(&mut state.regs, *obj);
            ensure_reg(&mut state.regs, *key);
            ensure_reg(&mut state.regs, *dest);

            match &state.regs[*obj] {
                Value::Object(map) => {
                    let k = match &state.regs[*key] {
                        Value::Symbol(s) => s.clone(),
                        Value::Str(s) => s.clone(),
                        _ => String::new(),
                    };
                    if let Some(v) = map.get(&k) {
                        state.regs[*dest] = v.clone();
                    } else {
                        state.regs[*dest] = Value::Null;
                    }
                }
                Value::Array(a) => {
                    match &state.regs[*key] {
                        Value::Symbol(s) | Value::Str(s) => {
                            if s == "length" {
                                state.regs[*dest] = Value::Int(a.len() as i64);
                            } else {
                                state.regs[*dest] = Value::Null;
                            }
                        }
                        _ => {
                            state.regs[*dest] = Value::Null;
                        }
                    }
                }
                Value::Str(s) => {
                    match &state.regs[*key] {
                        Value::Symbol(k) | Value::Str(k) => {
                            if k == "length" {
                                state.regs[*dest] = Value::Int(s.chars().count() as i64);
                            } else {
                                state.regs[*dest] = Value::Null;
                            }
                        }
                        _ => {
                            state.regs[*dest] = Value::Null;
                        }
                    }
                }
                _ => {
                    state.regs[*dest] = Value::Null;
                }
            }

            state.pc += 1;
        }
        Op::SetProp { obj, key, src } => {
            ensure_reg(&mut state.regs, *obj);
            ensure_reg(&mut state.regs, *key);
            ensure_reg(&mut state.regs, *src);
            let key_str = match &state.regs[*key] { Value::Symbol(s) => s.clone(), Value::Str(s) => s.clone(), _ => String::new() };
            let src_val = state.regs[*src].clone();
            match &mut state.regs[*obj] {
                Value::Object(map) => { map.insert(key_str, src_val); }
                other => { let mut m = std::collections::HashMap::new(); m.insert(key_str, src_val); *other = Value::Object(m); }
            }
            state.pc += 1;
        }
        Op::Ret { src } => {
            ensure_reg(&mut state.regs, *src);
            if let Some(f) = state.frames.pop() {
                if let Some(ret_reg) = f.return_reg { ensure_reg(&mut state.regs, ret_reg); state.regs[ret_reg] = state.regs[*src].clone(); }
                if let Some(ret_pc) = f.return_pc { state.pc = ret_pc; } else { state.pc = state.ops.len(); }
            } else { state.pc = state.ops.len(); }
        }
        Op::CallLabel { dest, label_index, args } => {
            let return_pc = state.pc + 1;
            let mut f = Frame { locals: Vec::new(), return_pc: Some(return_pc), return_reg: Some(*dest) };
            for (i, &areg) in args.iter().enumerate() {
                ensure_reg(&mut state.regs, areg);
                if i >= f.locals.len() { f.locals.resize(i + 1, Value::Null); }
                f.locals[i] = state.regs[areg].clone();
            }
            let label_name = format!("L{}", label_index);
            let resolved = state.label_by_name.get(&label_name).copied();
            state.frames.push(f);
            if let Some(idx) = resolved { state.pc = idx + 1; } else { return Err(format!("CallLabel: unknown label '{}'", label_name)); }
        }
        Op::PluginCall { plugin_name, func_name, args, result_target } => {
            let arg_vals = take_args(&state.regs, args);
            
            if let Some(plugin) = state.plugins.get(plugin_name) {
                let call_res = block_on(plugin.call(func_name, arg_vals));
                match call_res {
                    Ok(val) => {
                        
                        if let Some(dest) = result_target {
                            ensure_reg(&mut state.regs, *dest);
                            state.regs[*dest] = val;
                        }
                        state.pc += 1;
                    }
                    Err(e) => return Err(format!("Plugin - '{} '\nCall - '{} '\nError: {}", plugin_name, func_name, e)),
                }
            } else {
                return Err(format!("unknown plugin '{}'", plugin_name));
            }
        }
    }
    Ok(())
}

pub(crate) fn run_bytecode(bytes: &[u8], trace: bool, plugins: &crate::vm::plugin::PluginRegistry) -> Result<(), String> {
    // parse bytecode into ops + label maps
    let (ops, label_pos, label_by_name) = crate::vm::bytecode::parse_ops(bytes)?;

    // prepare execution state
    let mut state = ExecState {
        ops,
        label_pos,
        label_by_name,
        regs: Vec::new(),
        frames: vec![Frame { locals: Vec::new(), return_pc: None, return_reg: None }],
        pc: 0,
        steps: 0,
        trace,
        plugins,
    };

    // main dispatch loop
    while state.pc < state.ops.len() {
        state.steps += 1;
        if state.steps > 200 {
            return Err("VM step limit exceeded".to_string());
        }
        if state.trace {
            if let Some(lbl) = state.label_pos.get(&state.pc) {
                println!("== Label: {} ==", lbl);
            } else {
                println!("PC {}: {:?}", state.pc, state.ops[state.pc]);
            }
        }

        dispatch_op(&mut state)?;
    }

    Ok(())
}
