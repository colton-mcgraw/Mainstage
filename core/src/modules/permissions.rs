//! Capability-based permissions for side-effecting modules.
//!
//! The side-effecting standard-library modules (`shell`, `http`) are each gated on a
//! capability that the user must grant explicitly — either per-invocation via a CLI
//! flag (`--allow-run`, `--allow-net`) or per-project via a `[permissions]` block in
//! the `plugins.toml` manifest. Both default to *denied*, so a script can never spawn
//! a process or reach the network unless the user opts in.

use std::path::Path;

use crate::error::{Diagnostic, Error, Result};
use crate::modules::plugin::MANIFEST;

/// A capability that a module call may require before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Spawn external processes — required by the `shell` module.
    Run,
    /// Make network requests — required by the `http` module.
    Net,
}

impl Capability {
    /// The capability name as written in the manifest `[permissions]` block.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Run => "run",
            Capability::Net => "net",
        }
    }

    /// The CLI flag that grants this capability.
    pub fn flag(self) -> &'static str {
        match self {
            Capability::Run => "--allow-run",
            Capability::Net => "--allow-net",
        }
    }
}

/// The set of capabilities granted to a run. Every capability defaults to denied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Permissions {
    pub run: bool,
    pub net: bool,
}

impl Permissions {
    /// Grant every capability (used by `--allow-all` and in tests).
    pub fn all() -> Self {
        Self { run: true, net: true }
    }

    /// Whether `cap` is granted.
    pub fn grants(self, cap: Capability) -> bool {
        match cap {
            Capability::Run => self.run,
            Capability::Net => self.net,
        }
    }

    /// The union of two permission sets — a capability granted by *either* is granted.
    /// Used to combine manifest-declared and flag-granted permissions.
    pub fn union(self, other: Self) -> Self {
        Self { run: self.run || other.run, net: self.net || other.net }
    }

    /// Read the optional `[permissions]` block from the `plugins.toml` manifest under
    /// `script_dir`. A missing manifest or block grants nothing; a malformed manifest
    /// is a hard error. Unknown keys (e.g. the `[plugins]` table) are ignored.
    pub fn from_manifest(script_dir: &Path) -> Result<Self> {
        let manifest = script_dir.join(MANIFEST);
        let text = match std::fs::read_to_string(&manifest) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(manifest_error(format!("could not read {}: {}", MANIFEST, e))),
        };

        #[derive(serde::Deserialize, Default)]
        struct Table {
            #[serde(default)]
            run: bool,
            #[serde(default)]
            net: bool,
        }
        #[derive(serde::Deserialize)]
        struct Manifest {
            #[serde(default)]
            permissions: Table,
        }

        let parsed: Manifest = toml::from_str(&text)
            .map_err(|e| manifest_error(format!("invalid {}: {}", MANIFEST, e)))?;
        Ok(Self { run: parsed.permissions.run, net: parsed.permissions.net })
    }
}

/// Build a manifest-loading [`Error`] (no source span is available at load time).
fn manifest_error(message: impl Into<String>) -> Error {
    Error::Eval(vec![Diagnostic::new(message)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_grants_either_side() {
        let a = Permissions { run: true, net: false };
        let b = Permissions { run: false, net: true };
        let both = a.union(b);
        assert!(both.grants(Capability::Run));
        assert!(both.grants(Capability::Net));
    }

    #[test]
    fn default_denies_everything() {
        let p = Permissions::default();
        assert!(!p.grants(Capability::Run));
        assert!(!p.grants(Capability::Net));
        assert!(Permissions::all().grants(Capability::Run));
        assert!(Permissions::all().grants(Capability::Net));
    }

    fn write_manifest(tag: &str, body: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_perms_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), body).unwrap();
        dir
    }

    #[test]
    fn from_manifest_reads_permissions_block() {
        let dir = write_manifest("read", "[permissions]\nrun = true\nnet = false\n");
        let p = Permissions::from_manifest(&dir).unwrap();
        assert!(p.run && !p.net);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_manifest_ignores_unrelated_tables() {
        // A manifest with only `[plugins]` and no `[permissions]` grants nothing.
        let dir = write_manifest("plugins", "[plugins]\nlint = \"./lint\"\n");
        assert_eq!(Permissions::from_manifest(&dir).unwrap(), Permissions::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_manifest_missing_file_denies() {
        let dir = std::env::temp_dir().join("ms_perms_absent_dir_xyz");
        assert_eq!(Permissions::from_manifest(&dir).unwrap(), Permissions::default());
    }
}
