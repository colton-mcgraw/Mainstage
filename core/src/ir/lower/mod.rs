//! file: core/src/ir/lower/mod.rs
//! description: lowering pipeline modules.
//!
//! Re-exports lowering submodules used to transform `AstNode` trees into
//! an `IrModule`. This package contains helpers for statement/expression
//! lowering, per-function builders, and context wiring that consumes
//! analyzer output.
//!
mod lower_objects;
pub mod lower_expr;
pub mod lower_stmt;
pub mod lowering_context;
pub mod function_builder;

pub use lower_objects::lower_script_objects;
pub use lowering_context::LoweringContext;