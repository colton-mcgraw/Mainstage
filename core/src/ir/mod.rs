//! file: core/src/ir/mod.rs
//! description: intermediate representation (IR) crate root.
//!
//! This module re-exports the IR submodules (lowering, optimizations,
//! bytecode emission) and provides the `lower_ast_to_ir` convenience
//! entrypoint used by callers.
//!
pub mod lower;
pub mod opt;
pub mod bytecode;
pub mod op;
pub mod value;
pub mod module;

pub use self::bytecode::emit_bytecode;
use self::module::IrModule;
use self::lower::lower_script_objects;

/// # lower_ast_to_ir
/// Lowers an AST node into an IR module.
/// 
/// # Parameters
/// - `ast`: The AST node to lower.
/// - `optimize`: Whether to run optimizations on the IR.
/// - `analysis`: Optional analysis output to assist lowering.
pub fn lower_ast_to_ir(
    ast: &crate::ast::AstNode,
    optimize: bool,
    analysis: Option<&crate::analyzers::output::AnalyzerOutput>,
) -> IrModule {
    let mut ir_mod = IrModule::new();
    lower_script_objects(ast, &mut ir_mod, analysis);
    if optimize {
        opt::optimize(&mut ir_mod);
    }
    ir_mod
}