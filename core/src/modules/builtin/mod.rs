//! Built-in standard-library modules.
//!
//! Each module is a zero-sized type implementing [`Module`](super::Module) and is
//! registered by [`ModuleRegistry::standard`](super::ModuleRegistry::standard).

mod env;
mod git;

pub use env::EnvModule;
pub use git::GitModule;
