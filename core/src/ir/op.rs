//! file: core/src/ir/op.rs
//! description: IR operation definitions.
//!
//! Defines the `IROp` enum used by the lowering pipeline and optimizers.
//! Each variant represents a low-level IR instruction with virtual register
//! operands.

use super::value::Value;

/// # Register
/// 
/// Type alias for virtual register indices used in IR operations.
///
type Register = usize;

/// # IROp
///
/// Represents an intermediate representation operation.
/// Each variant corresponds to a specific instruction with its operands.
/// 
/// # Notes
/// This enum is designed to be extensible for various IR instructions.
/// The operands are typically virtual register indices.
///
#[derive(Debug, Clone, PartialEq)]
pub enum IROp {
    /// # LConst
    /// Loads a constant value into a register.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `value` is the constant value to load.
    LConst { dest: Register, value: Value },
    
    /// # LLocal
    /// Loads a local variable into a register.
    ///
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `local_index` is the index of the local variable.
    LLocal { dest: Register, local_index: usize },
    /// # SLocal
    /// Stores a register value into a local variable.
    /// 
    /// # Parameters
    /// - `src` is the source register.
    /// - `local_index` is the index of the local variable.
    SLocal { src: Register, local_index: usize },
    
    /// Arithmetic operations
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `src1` and `src2` are the source registers.
    /// 
    /// These operations perform basic arithmetic on the values in `src1` and `src2`
    /// and store the result in `dest`.
    /// 
    /// `src1` is the left operand and `src2` is the right operand.
    
    Add { dest: Register, src1: Register, src2: Register },
    Sub { dest: Register, src1: Register, src2: Register },
    Mul { dest: Register, src1: Register, src2: Register },
    Div { dest: Register, src1: Register, src2: Register },
    Mod { dest: Register, src1: Register, src2: Register },

    Eq { dest: Register, src1: Register, src2: Register },
    Neq { dest: Register, src1: Register, src2: Register },
    Lt { dest: Register, src1: Register, src2: Register },
    Lte { dest: Register, src1: Register, src2: Register },
    Gt { dest: Register, src1: Register, src2: Register },
    Gte { dest: Register, src1: Register, src2: Register },
    And { dest: Register, src1: Register, src2: Register },
    Or { dest: Register, src1: Register, src2: Register },
    Not { dest: Register, src: Register },

    Inc { dest: Register },
    Dec { dest: Register },

    /// # Label
    /// Defines a label in the instruction stream.
    /// 
    /// # Parameters
    /// - `name` is the name of the label.
    Label { name: String },
    /// # Jump
    /// Unconditional jump to a target instruction index.
    /// 
    /// # Parameters
    /// - `target` is the index of the target instruction.
    Jump { target: usize },
    /// # BrTrue
    /// Conditional branch if the condition register is true.
    ///
    /// # Parameters
    /// - `cond` is the condition register.
    /// - `target` is the index of the target instruction.
    BrTrue { cond: Register, target: usize },
    /// # BrFalse
    /// Conditional branch if the condition register is false.
    ///
    /// # Parameters
    /// - `cond` is the condition register.
    /// - `target` is the index of the target instruction.
    BrFalse { cond: Register, target: usize },

    /// # Halt
    /// Stops execution of the program.
    Halt,

    /// # AllocClosure
    /// Allocates a new closure and stores it in the destination register.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    AllocClosure { dest: Register },
    /// # CStore
    /// Stores a value into a closure's field.
    /// 
    /// # Parameters
    /// - `closure` is the register containing the closure.
    /// - `field` is the index of the field within the closure.
    /// - `src` is the register containing the value to store.
    CStore { closure: Register, field: usize, src: Register },
    /// # CLoad
    /// Loads a value from a closure's field into a register.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `closure` is the register containing the closure.
    /// - `field` is the index of the field within the closure.
    CLoad { dest: Register, closure: Register, field: usize },
    
    /// # ArrayNew
    /// Creates a new array from a list of registers.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `elems` is the list of registers to include in the array.
    ArrayNew { dest: Register, elems: Vec<Register> },
    /// # LoadGlobal
    /// Loads a module-level register into a function-local register.
    /// 
    /// # Parameters
    /// - `dest` is a function-local register index that will be remapped into the module register space.
    /// - `src` is a module-global register index and should not be remapped during function finalization.
    LoadGlobal { dest: Register, src: Register },
    /// # ArrayGet
    /// Retrieves an element from an array.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `array` is the register containing the array.
    /// - `index` is the register containing the index.
    ArrayGet { dest: Register, array: Register, index: Register },
    /// # ArraySet
    /// Sets an element in an array.
    /// 
    /// # Parameters
    /// - `array` is the register containing the array.
    /// - `index` is the register containing the index.
    /// - `src` is the register containing the value to set.
    ArraySet { array: Register, index: Register, src: Register },

    /// # GetProp
    /// Retrieves a property from an object.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `obj` is the register containing the object.
    /// - `key` is the register containing the property key.
    GetProp { dest: Register, obj: Register, key: Register },
    /// # SetProp
    /// Sets a property on an object.
    /// 
    /// # Parameters
    /// - `obj` is the register containing the object.
    /// - `key` is the register containing the property key.
    /// - `src` is the register containing the value to set.
    SetProp { obj: Register, key: Register, src: Register },

    /// # CallLabel
    /// Calls a function by its label index.
    /// 
    /// # Parameters
    /// - `dest` is the destination register.
    /// - `label_index` is the index of the label to call.
    /// - `args` is the list of argument registers.
    CallLabel { dest: Register, label_index: usize, args: Vec<Register> },
    /// # PluginCall
    /// Calls a plugin function.
    /// 
    /// # Parameters
    /// - `dest` is the optional destination register for the return value.
    /// - `plugin_name` is the name of the plugin.
    /// - `func_name` is the name of the function within the plugin.
    /// - `args` is the list of argument registers.
    PluginCall { dest: Option<Register>, plugin_name: String, func_name: String, args: Vec<Register> },
    /// # Ret
    /// Returns from the current function.
    /// 
    /// # Parameters
    /// - `src` is the register containing the return value.
    Ret { src: Register },
}

impl std::fmt::Display for IROp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IROp::LConst { dest, value } => write!(f, "LConst r{} <- {:?}", dest, value),
            IROp::LLocal { dest, local_index } => write!(f, "LLocal r{} <- local[{}]", dest, local_index),
            IROp::SLocal { src, local_index } => write!(f, "SLocal local[{}] <- r{}", local_index, src),
            IROp::Add    { dest, src1, src2 } => write!(f, "Add r{} <- r{} + r{}", dest, src1, src2),
            IROp::Sub    { dest, src1, src2 } => write!(f, "Sub r{} <- r{} - r{}", dest, src1, src2),
            IROp::Mul    { dest, src1, src2 } => write!(f, "Mul r{} <- r{} * r{}", dest, src1, src2),
            IROp::Div    { dest, src1, src2 } => write!(f, "Div r{} <- r{} / r{}", dest, src1, src2),
            IROp::Mod    { dest, src1, src2 } => write!(f, "Mod r{} <- r{} % r{}", dest, src1, src2),
            IROp::Eq     { dest, src1, src2 } => write!(f, "Eq r{} <- r{} == r{}", dest, src1, src2),
            IROp::Neq    { dest, src1, src2 } => write!(f, "Neq r{} <- r{} != r{}", dest, src1, src2),
            IROp::Lt     { dest, src1, src2 } => write!(f, "Lt r{} <- r{} < r{}", dest, src1, src2),
            IROp::Lte    { dest, src1, src2 } => write!(f, "Lte r{} <- r{} <= r{}", dest, src1, src2),
            IROp::Gt     { dest, src1, src2 } => write!(f, "Gt r{} <- r{} > r{}", dest, src1, src2),
            IROp::Gte    { dest, src1, src2 } => write!(f, "Gte r{} <- r{} >= r{}", dest, src1, src2),
            IROp::And    { dest, src1, src2 } => write!(f, "And r{} <- r{} && r{}", dest, src1, src2),
            IROp::Or     { dest, src1, src2 } => write!(f, "Or r{} <- r{} || r{}", dest, src1, src2),
            IROp::Not    { dest, src } => write!(f, "Not r{} <- !r{}", dest, src),
            IROp::Inc    { dest } => write!(f, "Inc r{} ++", dest),
            IROp::Dec    { dest } => write!(f, "Dec r{} --", dest),
            IROp::Label  { name } => write!(f, "Label {}", name),
            IROp::Jump   { target } => write!(f, "Jump {}", target),
            IROp::BrTrue { cond, target } => write!(f, "BrTrue r{} -> {}", cond, target),
            IROp::BrFalse { cond, target } => write!(f, "BrFalse r{} -> {}", cond, target),
            IROp::Halt => write!(f, "Halt"),
            IROp::AllocClosure { dest } => write!(f, "AllocClosure r{}", dest),
            IROp::CStore { closure, field, src } => write!(f, "CStore clo[r{}].{} <- r{}", closure, field, src),
            IROp::CLoad { dest, closure, field } => write!(f, "CLoad r{} <- clo[r{}].{}", dest, closure, field),
            IROp::CallLabel { dest, label_index, args } => {
                write!(f, "CallLabel r{} <- L{}(", dest, label_index)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "r{}", arg)?;
                }
                write!(f, ")")
            }
            IROp::PluginCall { dest, plugin_name, func_name, args } => {
                if let Some(d) = dest {
                    write!(f, "PluginCall r{} <- {}.{}(", d, plugin_name, func_name)?;
                } else {
                    write!(f, "PluginCall <- {}.{}(", plugin_name, func_name)?;
                }
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "r{}", arg)?;
                }
                write!(f, ")")
            }
            IROp::Ret { src } => write!(f, "Ret r{}", src),
            IROp::ArrayNew { dest, elems } => {
                write!(f, "ArrayNew r{} <- [", dest)?;
                for (i, r) in elems.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "r{}", r)?;
                }
                write!(f, "]")
            }
            IROp::LoadGlobal { dest, src } => write!(f, "LoadGlobal r{} <- r{}", dest, src),
            IROp::ArrayGet { dest, array, index } => write!(f, "ArrayGet r{} <- r{}[r{}]", dest, array, index),
            IROp::ArraySet { array, index, src } => write!(f, "ArraySet r{}[r{}] <- r{}", array, index, src),
            IROp::GetProp { dest, obj, key } => write!(f, "GetProp r{} <- r{}.r{}", dest, obj, key),
            IROp::SetProp { obj, key, src } => write!(f, "SetProp r{}.r{} <- r{}", obj, key, src),
        }
    }
}