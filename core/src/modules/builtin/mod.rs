//! Built-in standard-library modules.
//!
//! Each module is a zero-sized type implementing [`Module`](super::Module) and is
//! registered by [`ModuleRegistry::standard`](super::ModuleRegistry::standard).

mod env;
mod fs;
mod git;
mod hash;
mod json;
mod path;
mod str;

pub use env::EnvModule;
pub use fs::FsModule;
pub use git::GitModule;
pub use hash::HashModule;
pub use json::JsonModule;
pub use path::PathModule;
pub use str::StrModule;
