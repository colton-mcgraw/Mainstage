use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// A loaded `.ms` source file — path plus the full UTF-8 text.
#[derive(Debug, Clone)]
pub struct Source {
    /// Absolute or relative path used for span reporting.
    pub path: PathBuf,
    /// Complete source text of the file.
    pub text: String,
}

impl Source {
    /// Read a source file from disk, returning an [`Error::Io`] on failure.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path)
            .map_err(|source| Error::Io { path: path.clone(), source })?;
        Ok(Self { path, text })
    }

    /// Construct a `Source` directly from an in-memory string — useful for tests.
    pub fn from_str(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        Self { path: path.into(), text: text.into() }
    }
}
