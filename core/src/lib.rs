//! file: core/src/lib.rs
//! description: public facade for the `core` crate.
//!
//! This crate root re-exports the primary compiler components (AST,
//! analyzers, IR and VM) and provides a small set of helper functions used
//! by callers and the CLI. Keep this module lightweight; most logic lives in
//! the submodules.

pub mod ast;
pub mod error;
pub mod location;
pub mod script;
pub mod analyzers;
pub mod ir;
pub mod vm;

pub use ast::RulesParser;
pub use error::{Level, MainstageErrorExt};
pub use location::{Location, Span};
pub use script::Script;
pub use analyzers::{analyze_semantic_rules, analyze_acyclic_rules};
pub use ir::{lower_ast_to_ir, emit_bytecode};
pub use vm::VM;

pub fn generate_error_report<E: MainstageErrorExt>(error: &E) -> String {
    let level = error.level();
    let location = match error.location() {
        Some(loc) => loc.to_string(),
        None => "unknown location".to_string(),
    };
    let message = error.message();

    format!("MAINSTAGE | {} | {} | {}", level, location, message)
}

pub fn generate_ir_from_ast(
    ast: &str,
    analysis: &str,
) -> Result<String, Box<dyn MainstageErrorExt>> {
    // Placeholder implementation
    Ok(format!("IR({} + {})", ast, analysis))
}

pub fn optimize_ir(ir: &str) -> Result<String, Box<dyn MainstageErrorExt>> {
    Ok(format!("Optimized({})", ir))
}

pub fn run_ir_in_vm(_ir: &str) -> Result<String, Box<dyn MainstageErrorExt>> {
    Ok("IR".to_string())
}

pub fn compile_source_to_ir(source: &Script) -> Result<String, Box<dyn MainstageErrorExt>> {
    let mut ast = ast::generate_ast_from_source(source)?;
    let (_entry, _analysis) = match analyze_semantic_rules(&mut ast, None) {
        Ok((name, analysis)) => (name, analysis),
        Err(diags) => return Err(diags.into_iter().next().unwrap()),
    };
    //let ir = generate_ir_from_ast("", &ast)?;
    //let optimized_ir = optimize_ir(&ir)?;
    //Ok(optimized_ir)

    Ok(String::default())
}
