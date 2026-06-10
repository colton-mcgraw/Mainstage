//! Built-in standard-library modules.
//!
//! Each module is a zero-sized type implementing [`Module`](super::Module) and is
//! registered by [`ModuleRegistry::standard`](super::ModuleRegistry::standard).

mod env;
mod git;
mod hash;
mod path;
mod str;

pub use env::EnvModule;
pub use git::GitModule;
pub use hash::HashModule;
pub use path::PathModule;
pub use str::StrModule;
