//! file: core/src/vm/mod.rs
//! description: virtual machine top-level API and plugin discovery helpers.
//!
//! This module exposes the `VM` type which holds plugin descriptors/instances
//! and a bytecode image. It provides helpers to discover plugin manifests and
//! run a bytecode image via the executor.

mod op;
pub mod value;
pub mod manifest;
pub mod external;
pub mod inprocess;
pub mod plugin;
mod run;
mod bytecode;
mod exec;

pub struct VM
{
    plugins: plugin::PluginRegistry,
    bytecode: Vec<u8>,
}

impl VM
{
    pub fn new(bytecode: Vec<u8>) -> Self
    {
        VM {
            plugins: plugin::PluginRegistry::new(),
            bytecode,
        }
    }

    /// Discover plugin manifests in `dir` (if provided) or the default plugin location.
    /// Returns number of manifests registered.
    pub fn discover_plugins<P: AsRef<std::path::Path>>(&mut self, dir: Option<P>) -> Result<usize, String> {
        use std::path::{PathBuf};

        // Determine directory to scan: provided -> env var -> ./plugins
        let scan_dir: PathBuf = if let Some(d) = dir {
            d.as_ref().to_path_buf()
        } else if let Ok(envp) = std::env::var("MAINSTAGE_PLUGIN_DIR") {
            PathBuf::from(envp)
        } else {
            // default to ./plugins in current working dir
            let mut p = std::env::current_dir().map_err(|e| format!("cwd: {}", e))?;
            p.push("plugins");
            p
        };

        let manifests = crate::vm::manifest::discover_manifests_in_dir(&scan_dir)?;
        let mut count = 0usize;
        for (manifest, path) in manifests {
            // store descriptor in registry (path is manifest file path)
            let dir_path = path.parent().map(|p| p.to_path_buf());
            self.plugins.register_descriptor(manifest, dir_path);
            count += 1;
        }
        // After registering descriptors, attempt to register external plugin instances
        // for manifests that ship a companion executable in the same directory.
        for desc in self.plugins.descriptors_map().values() {
            self.plugins.try_register_external(desc);
        }
        Ok(count)
    }

    /// Return a cloned map of plugin descriptors discovered/registered in the VM.
    pub fn plugin_descriptors(&self) -> std::collections::HashMap<String, crate::vm::plugin::PluginDescriptor> {
        self.plugins.descriptors_map()
    }

    /// Return the list of currently registered runtime plugin names.
    pub fn registered_plugin_names(&self) -> Vec<String> {
        self.plugins.registered_names()
    }

    pub fn run(&self, enable_tracing: bool) -> Result<(), String>
    {
        return run::run_bytecode(&self.bytecode, enable_tracing, &self.plugins);
    }

    pub fn register_plugin(&mut self, plugin: std::sync::Arc<dyn plugin::Plugin>)
    {
        self.plugins.register(plugin);
    }

    pub fn unregister_plugin(&mut self, name: &str)
    {
        self.plugins.unregister(name);
    }
}