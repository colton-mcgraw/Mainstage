//! `mainstage_core` — language core: parser, AST, semantic analysis, and evaluator.

pub mod ast;
pub mod cache;
pub mod error;
pub mod eval;
pub mod executor;
pub mod format;
pub mod matrix;
pub mod modules;
pub mod parser;
pub mod runner;
pub mod sema;
pub mod source;
pub mod trivia;

pub use error::{Diagnostic, Error, Result, Span};
pub use eval::{
    AssertionResult, EvalContext, FileEntry, OutputSink, ReporterHandle, TestRecorder, Value,
    eval_condition, eval_expr, eval_program, eval_program_with,
};
pub use executor::{execute_step, execute_steps};
pub use format::format;
pub use matrix::expand as expand_matrix;
pub use modules::{
    Capability, ExternalModule, LintFinding, LintLevel, LintReport, Module, ModuleRegistry,
    Permissions, lint_plugin,
};
pub use parser::parse;
pub use runner::{
    CancelToken, NoopReporter, Plan, PlanStatus, PlannedStage, Reporter, StageOutcome,
    pipeline_input_paths, plan_pipeline, run_pipeline, run_pipeline_cancellable,
    run_pipeline_reported, run_pipeline_reported_jobs,
};
pub use sema::{AnalysisResult, analyze, analyze_with};
pub use source::Source;
pub use trivia::{
    Comment, CommentKind, NodeTrivia, SyntaxToken, TokenKind, TriviaMap, attach as attach_trivia,
    comments, lex, render,
};
