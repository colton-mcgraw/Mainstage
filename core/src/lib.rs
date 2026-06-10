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
    eval_condition, eval_expr, eval_program, eval_program_with, EvalContext, FileEntry, Value,
};
pub use executor::{execute_step, execute_steps};
pub use modules::{ExternalModule, Module, ModuleRegistry};
pub use runner::{run_pipeline, run_pipeline_reported, NoopReporter, Reporter};
pub use parser::parse;
pub use sema::{analyze, analyze_with, AnalysisResult};
pub use source::Source;
