//! file: core/src/vm/op.rs
//! description: runtime operation representation for the bytecode interpreter.
//!
//! The `Op` enum mirrors the wire-format bytecode and is the decoded form
//! the executor matches over during runtime. Registers are indexes into the
//! VM register file.

type Register = usize;

// Parsed runtime op to execute
#[derive(Debug, Clone)]
pub(crate) enum Op {
    LConst {
        dest: Register,
        val: super::value::Value,
    },
    LLocal {
        dest: Register,
        local: Register,
    },
    SLocal {
        src: Register,
        local: Register,
    },
    Add {
        dest: Register,
        a: Register,
        b: Register,
    },
    Sub {
        dest: Register,
        a: Register,
        b: Register,
    },
    Mul {
        dest: Register,
        a: Register,
        b: Register,
    },
    Div {
        dest: Register,
        a: Register,
        b: Register,
    },
    Mod {
        dest: Register,
        a: Register,
        b: Register,
    },
    Eq {
        dest: Register,
        a: Register,
        b: Register,
    },
    Neq {
        dest: Register,
        a: Register,
        b: Register,
    },
    Lt {
        dest: Register,
        a: Register,
        b: Register,
    },
    Lte {
        dest: Register,
        a: Register,
        b: Register,
    },
    Gt {
        dest: Register,
        a: Register,
        b: Register,
    },
    Gte {
        dest: Register,
        a: Register,
        b: Register,
    },
    And {
        dest: Register,
        a: Register,
        b: Register,
    },
    Or {
        dest: Register,
        a: Register,
        b: Register,
    },
    Not {
        dest: Register,
        src: Register,
    },
    Inc {
        dest: Register,
    },
    Dec {
        dest: Register,
    },
    Label,
    Jump {
        target: Register,
    },
    BrTrue {
        cond: Register,
        target: Register,
    },
    BrFalse {
        cond: Register,
        target: Register,
    },
    Halt,
    CallLabel {
        dest: Register,
        label_index: Register,
        args: Vec<Register>,
    },
    PluginCall {
        plugin_name: String,
        func_name: String,
        args: Vec<Register>,
        result_target: Option<Register>,
    },
    ArrayNew {
        dest: Register,
        elems: Vec<Register>,
    },
    LoadGlobal {
        dest: Register,
        src: Register,
    },
    ArrayGet {
        dest: Register,
        array: Register,
        index: Register,
    },
    ArraySet {
        array: Register,
        index: Register,
        src: Register,
    },
    GetProp {
        dest: Register,
        obj: Register,
        key: Register,
    },
    SetProp {
        obj: Register,
        key: Register,
        src: Register,
    },
    Ret {
        src: Register,
    },
}