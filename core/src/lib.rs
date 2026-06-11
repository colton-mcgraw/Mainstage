//! `mainstage_core` — language core: parser, AST, semantic analysis, and evaluator.

pub mod ast;
pub mod cache;
pub mod error;
pub mod eval;
pub mod executor;
pub mod modules;
pub mod parser;
pub mod runner;
pub mod sema;
pub mod source;

pub use error::{Diagnostic, Error, Result, Span};
pub use eval::{
    EvalContext, FileEntry, Value, eval_condition, eval_expr, eval_program, eval_program_with,
};
pub use executor::{execute_step, execute_steps};
pub use modules::{Capability, ExternalModule, Module, ModuleRegistry, Permissions};
pub use parser::parse;
pub use runner::{NoopReporter, Reporter, run_pipeline, run_pipeline_reported};
pub use sema::{AnalysisResult, analyze, analyze_with};
pub use source::Source;
