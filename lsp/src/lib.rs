//! `mainstage_lsp` — the Mainstage Language Server.
//!
//! A thin protocol shell over `mainstage_core`: it reuses the same `parse` /
//! `analyze_with` pipeline and [`ModuleRegistry`](mainstage_core::ModuleRegistry)
//! as the CLI, so editor behavior never diverges from the command line. The
//! server runs over stdio on a `tokio` runtime and is launched either by the
//! `mainstage-lsp` binary or the `mainstage lsp` CLI subcommand.

pub mod analysis;
pub mod completion;
pub mod convert;
pub mod cursor;
pub mod hover;
pub mod index;
pub mod navigation;
pub mod server;
pub mod signature;

pub use analysis::{analyze, build_registry, parse_text, Analysis};
pub use convert::{span_to_range, to_lsp_diagnostic, to_lsp_diagnostics};
pub use server::{serve, Backend};

/// Launch the language server on stdio, blocking until the client disconnects.
///
/// Sets up a `tokio` runtime and drives [`server::serve`] on it. This is the
/// shared entry point used by both the `mainstage-lsp` binary and the
/// `mainstage lsp` CLI subcommand.
pub fn run_stdio() {
    let runtime = tokio::runtime::Runtime::new()
        .expect("failed to start the tokio runtime for the language server");
    runtime.block_on(server::serve());
}
