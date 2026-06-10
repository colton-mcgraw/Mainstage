//! External subprocess plugins.
//!
//! An [`ExternalModule`] wraps a long-lived child process that speaks the
//! newline-delimited JSON [`protocol`]. The process is spawned once when the module
//! is loaded — at which point its `describe` response fixes the module name and
//! method signatures — and is reused for every `call` for the duration of a single
//! `mainstage` run. Because [`Module::call`] takes `&self`, the process handles live
//! behind a [`Mutex`] so calls are serialized.
//!
//! Plugins are discovered (see [`discover`]) from two sources, after the built-in
//! registry which always wins:
//!
//! 1. executables under `.mainstage/plugins/` (nested paths form namespaced names
//!    like `acme/lint`), then
//! 2. a `plugins.toml` manifest mapping module names to executable paths.

pub mod protocol;

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde::de::DeserializeOwned;

use crate::error::{Diagnostic, Error, Result};
use crate::eval::Value;
use crate::modules::{resolve_path, MethodSig, Module, ModuleCx, ResolvedArg};

use protocol::{CallResponse, DescribeResponse, Request, WireArg};

/// The directory under the project root scanned for plugin executables.
const PLUGINS_DIR: &str = ".mainstage/plugins";
/// The manifest file (relative to the project root) mapping names to executables.
const MANIFEST: &str = "plugins.toml";

// ── External module ─────────────────────────────────────────────────────────────

/// A module backed by an external subprocess plugin.
pub struct ExternalModule {
    name: String,
    methods: Vec<MethodSig>,
    /// The live plugin process. Serialized behind a `Mutex` so the `&self` `call`
    /// can drive its stdio without interior-mutability races.
    process: Mutex<PluginProcess>,
}

impl ExternalModule {
    /// Spawn `exe`, run `describe`, and return a loaded module registered under
    /// `name`. The discovered `name` — not the plugin's self-reported name — is
    /// authoritative, so discovery controls the namespace.
    pub fn load(name: &str, exe: &Path, script_dir: &Path) -> Result<Self> {
        let mut process = PluginProcess::spawn(exe, script_dir)
            .map_err(|e| load_error(format!("plugin '{}': {}", name, e)))?;

        let describe: DescribeResponse = process
            .request(&Request::Describe)
            .map_err(|e| load_error(format!("plugin '{}': describe failed: {}", name, e)))?;

        let methods = describe
            .methods
            .into_iter()
            .map(|m| m.into_sig())
            .collect::<std::result::Result<Vec<_>, String>>()
            .map_err(|e| load_error(format!("plugin '{}': invalid signature: {}", name, e)))?;

        Ok(ExternalModule { name: name.to_string(), methods, process: Mutex::new(process) })
    }
}

impl Module for ExternalModule {
    fn name(&self) -> &str {
        &self.name
    }

    fn methods(&self) -> &[MethodSig] {
        &self.methods
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        let request = Request::Call {
            method: method.to_string(),
            args: args.iter().map(WireArg::from_resolved).collect(),
        };

        let response: CallResponse = {
            // Hold the lock only across the request/response round-trip.
            let mut process = self.process.lock().unwrap_or_else(|p| p.into_inner());
            process
                .request(&request)
                .map_err(|e| cx.error(format!("plugin '{}': {}", self.name, e)))?
        };

        // Map the plugin's `err` to an eval error carrying the call span; a missing
        // `ok` with no `err` is a protocol violation rather than a user error.
        match (response.ok, response.err) {
            (Some(value), _) => Ok(value.into_value()),
            (None, Some(message)) => {
                Err(cx.error(format!("{}.{}: {}", self.name, method, message)))
            }
            (None, None) => Err(cx.error(format!(
                "plugin '{}': malformed response to {}.{} (neither 'ok' nor 'err')",
                self.name, self.name, method
            ))),
        }
    }
}

/// Build a load-time [`Error`] (no call span is available yet).
fn load_error(message: impl Into<String>) -> Error {
    Error::Eval(vec![Diagnostic::new(message)])
}

// ── Plugin process ──────────────────────────────────────────────────────────────

/// A spawned plugin process with buffered access to its stdio. Dropping it closes
/// stdin, which signals the plugin to exit.
struct PluginProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl PluginProcess {
    fn spawn(exe: &Path, script_dir: &Path) -> std::result::Result<Self, String> {
        let mut child = Command::new(exe)
            .current_dir(script_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Leave stderr inherited so plugin diagnostics surface to the user.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("failed to start '{}': {}", exe.display(), e))?;

        let stdin = child.stdin.take().ok_or("could not open plugin stdin")?;
        let stdout = child.stdout.take().ok_or("could not open plugin stdout")?;
        Ok(PluginProcess { child, stdin, stdout: BufReader::new(stdout) })
    }

    /// Send one request line and read exactly one response line, deserialized as `T`.
    fn request<T: DeserializeOwned>(&mut self, request: &Request) -> std::result::Result<T, String> {
        let line = serde_json::to_string(request)
            .map_err(|e| format!("could not encode request: {}", e))?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("could not write to plugin: {}", e))?;

        let mut response = String::new();
        let read = self
            .stdout
            .read_line(&mut response)
            .map_err(|e| format!("could not read from plugin: {}", e))?;
        if read == 0 {
            return Err(self.exit_diagnosis());
        }

        serde_json::from_str(response.trim()).map_err(|e| {
            format!("malformed JSON response: {} (in: {})", e, response.trim())
        })
    }

    /// Describe why the plugin closed its stdout, checking the process exit status.
    fn exit_diagnosis(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                format!("plugin exited with {}", status)
            }
            Ok(Some(_)) => "plugin closed its output without responding".to_string(),
            _ => "plugin closed its output unexpectedly".to_string(),
        }
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        // Best-effort cleanup: reap the child so it does not linger after the run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Discovery ───────────────────────────────────────────────────────────────────

/// Discover plugin executables under `script_dir`, skipping any whose module name
/// is already claimed by a built-in (in `reserved`) — built-ins are never shadowed.
///
/// Directory entries take precedence over manifest entries with the same name. The
/// returned modules are spawned and described; an error from any one plugin aborts
/// discovery so problems surface immediately rather than at first call.
pub fn discover(script_dir: &Path, reserved: &HashSet<String>) -> Result<Vec<ExternalModule>> {
    let mut specs: BTreeMap<String, PathBuf> = BTreeMap::new();

    // Source 1: executables under `.mainstage/plugins/`.
    let dir = script_dir.join(PLUGINS_DIR);
    if dir.is_dir() {
        collect_dir(&dir, &dir, &mut specs);
    }

    // Source 2: the `plugins.toml` manifest; directory entries win on conflict.
    for (name, path) in read_manifest(script_dir)? {
        specs.entry(name).or_insert_with(|| resolve_path(script_dir, &path));
    }

    let mut modules = Vec::new();
    for (name, exe) in specs {
        // Built-ins always win — silently skip a plugin that would shadow one.
        if reserved.contains(&name) {
            continue;
        }
        modules.push(ExternalModule::load(&name, &exe, script_dir)?);
    }
    Ok(modules)
}

/// Recursively collect executable files under `root`, keying each by its path
/// relative to `root` (with `/` separators and any extension stripped) so nested
/// files yield namespaced names like `acme/lint`.
fn collect_dir(root: &Path, dir: &Path, specs: &mut BTreeMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir(root, &path, specs);
        } else if is_executable(&path) && let Some(name) = module_name(root, &path) {
            specs.entry(name).or_insert(path);
        }
    }
}

/// Derive a `/`-separated, extension-stripped module name from `path`'s location
/// relative to the plugins `root`.
fn module_name(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let with_ext: PathBuf = rel.components().collect();
    // Strip the file extension from the final component only.
    let stem = match (with_ext.parent(), with_ext.file_stem()) {
        (Some(parent), Some(stem)) if !parent.as_os_str().is_empty() => parent.join(stem),
        (_, Some(stem)) => PathBuf::from(stem),
        _ => with_ext,
    };
    let name = stem
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    (!name.is_empty()).then_some(name)
}

/// Whether `path` should be treated as a runnable plugin. On Unix this checks the
/// owner-execute bit; on other platforms every regular file qualifies.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.permissions().mode() & 0o100 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Parse the optional `plugins.toml` manifest into `name → path` pairs. A missing
/// manifest yields an empty map; a malformed one is a hard error.
fn read_manifest(script_dir: &Path) -> Result<BTreeMap<String, String>> {
    let manifest = script_dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(load_error(format!("could not read {}: {}", MANIFEST, e))),
    };

    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(default)]
        plugins: BTreeMap<String, String>,
    }

    let parsed: Manifest = toml::from_str(&text)
        .map_err(|e| load_error(format!("invalid {}: {}", MANIFEST, e)))?;
    Ok(parsed.plugins)
}

#[cfg(test)]
mod tests;
