//! Built-in standard-library modules.
//!
//! Each module is a zero-sized type implementing [`Module`](super::Module) and is
//! registered by [`ModuleRegistry::standard`](super::ModuleRegistry::standard).

mod env;
mod fs;
mod git;
mod hash;
mod http;
mod json;
mod path;
mod shell;
mod str;
mod time;

pub use env::EnvModule;
pub use fs::FsModule;
pub use git::GitModule;
pub use hash::HashModule;
pub use http::HttpModule;
pub use json::JsonModule;
pub use path::PathModule;
pub use shell::ShellModule;
pub use str::StrModule;
pub use time::TimeModule;
