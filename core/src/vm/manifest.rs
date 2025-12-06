//! file: core/src/vm/manifest.rs
//! description: plugin manifest types and discovery helpers.
//!
//! Defines `PluginManifest` and helper functions used to discover and
//! validate plugin manifests on disk. Manifests are expected to be JSON
//! files describing exported functions and metadata.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionArg {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    #[serde(default)]
    /// Optional domain/namespace for the function (e.g., "util", "fs").
    /// When present, callers should treat the fully-qualified name as
    /// `"<domain>.<name>"`. If absent, `name` may already be qualified.
    pub domain: Option<String>,
    #[serde(default)]
    pub args: Vec<FunctionArg>,
    #[serde(default)]
    pub returns: Option<FunctionArg>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_abi")]
    pub abi: String,
    #[serde(default)]
    /// Optional hint for loader: "inprocess" (shared library) or "external" (separate process).
    /// Defaults to "inprocess".
    pub kind: Option<String>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub functions: Vec<FunctionSpec>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn default_abi() -> String { "inprocess".to_string() }

impl PluginManifest {
    /// Load a manifest from a JSON file path.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<PluginManifest, String> {
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("read manifest: {}", e))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse manifest: {}", e))
    }

    /// Optionally validate the manifest (basic checks).
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("manifest name is empty".to_string());
        }
        // function names must be unique
        let mut seen = std::collections::HashSet::new();
        for f in &self.functions {
            if !seen.insert(f.name.clone()) {
                return Err(format!("duplicate function name '{}'", f.name));
            }
        }
        Ok(())
    }

    /// Return true when manifest explicitly requests in-process loading.
    pub fn prefers_inprocess(&self) -> bool {
        match self.kind.as_deref() {
            Some("inprocess") => true,
            Some("external") => false,
            _ => self.abi == "inprocess",
        }
    }
}

/// Discover manifests in a directory. Expects each plugin in its own subdirectory
/// with a `manifest.json` file. Returns (manifest, manifest_path) tuples.
pub fn discover_manifests_in_dir<P: AsRef<Path>>(dir: P) -> Result<Vec<(PluginManifest, std::path::PathBuf)>, String> {
    let mut out = Vec::new();
    let dir = dir.as_ref();
    if !dir.exists() {
        return Ok(out);
    }
    // First, check if a manifest.json exists directly in the provided directory.
    let root_manifest = dir.join("manifest.json");
    if root_manifest.exists() {
        let manifest = PluginManifest::load_from_file(&root_manifest)?;
        manifest.validate()?;
        out.push((manifest, root_manifest));
    }
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read dir: {}", e))? {
        let entry = entry.map_err(|e| format!("read dir entry: {}", e))?;
        if !entry.file_type().map_err(|e| format!("file type: {}", e))?.is_dir() {
            continue;
        }
        let manifest_path = entry.path().join("manifest.json");
        if manifest_path.exists() {
            let manifest = PluginManifest::load_from_file(&manifest_path)?;
            manifest.validate()?;
            out.push((manifest, manifest_path));
        }
    }
    Ok(out)
}