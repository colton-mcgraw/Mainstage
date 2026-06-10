//! External subprocess plugins.
//!
//! A plugin is an executable that speaks a newline-delimited JSON protocol over
//! stdio. The host sends one request object per line and reads one response object
//! per line:
//!
//! - `{"op":"describe"}` → `{"name":"<module>","methods":[<MethodSig>,…]}`
//! - `{"op":"call","method":"<m>","args":[{"name":<str|null>,"value":<WireValue>}]}`
//!   → `{"ok":<WireValue>}` or `{"err":"<message>"}`
//!
//! A [`WireValue`] is an adjacently-tagged JSON encoding of a [`Value`]
//! (`{"type":"string","value":"…"}`). The plugin process is spawned once, queried
//! with `describe` at load time, and kept alive for the duration of the run; each
//! `call` is one request/response round-trip.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{Diagnostic, Error, Result};
use crate::eval::{FileEntry, Value};

use super::{MethodSig, Module, ModuleCx, ResolvedArg};

// ── Wire protocol ─────────────────────────────────────────────────────────────

/// A [`Value`] in its on-the-wire JSON form, e.g. `{"type":"string","value":"x"}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
enum WireValue {
    String(String),
    Bool(bool),
    List(Vec<WireValue>),
    Fileset(Vec<WireFile>),
}

/// A `FileSet` entry on the wire. Only `path` is required to reconstruct a
/// [`FileEntry`]; the derived fields are sent for the plugin's convenience.
#[derive(Debug, Serialize, Deserialize)]
struct WireFile {
    path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    stem: String,
    #[serde(default)]
    ext: String,
    #[serde(default)]
    dir: String,
}

impl WireValue {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::String(s) => WireValue::String(s.clone()),
            Value::Bool(b) => WireValue::Bool(*b),
            Value::List(items) => {
                WireValue::List(items.iter().map(WireValue::from_value).collect())
            }
            Value::FileSet(entries) => WireValue::Fileset(
                entries
                    .iter()
                    .map(|e| WireFile {
                        path: e.path.display().to_string(),
                        name: e.name.clone(),
                        stem: e.stem.clone(),
                        ext: e.ext.clone(),
                        dir: e.dir.display().to_string(),
                    })
                    .collect(),
            ),
        }
    }

    fn into_value(self) -> Value {
        match self {
            WireValue::String(s) => Value::String(s),
            WireValue::Bool(b) => Value::Bool(b),
            WireValue::List(items) => {
                Value::List(items.into_iter().map(WireValue::into_value).collect())
            }
            // Reconstruct each entry from its path; derived fields are recomputed.
            WireValue::Fileset(files) => Value::FileSet(
                files.into_iter().map(|f| FileEntry::from_path(PathBuf::from(f.path))).collect(),
            ),
        }
    }
}

/// A request sent to a plugin, one JSON object per line.
#[derive(Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Request<'a> {
    Describe,
    Call { method: &'a str, args: Vec<WireArg> },
}

#[derive(Serialize)]
struct WireArg {
    name: Option<String>,
    value: WireValue,
}

/// The `describe` response: the module's name and its method signatures.
#[derive(Deserialize)]
struct DescribeResponse {
    #[serde(default)]
    name: String,
    #[serde(default)]
    methods: Vec<MethodSig>,
}

/// The `call` response: success with a value, or a plugin-reported error message.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum CallResponse {
    Ok(WireValue),
    Err(String),
}

// ── Transport ─────────────────────────────────────────────────────────────────

/// One request/response round-trip over a line-delimited channel. Abstracted from
/// the process so the protocol can be unit-tested without spawning anything.
trait Transport: Send {
    /// Send `line` (a single JSON object, no trailing newline) and return the
    /// response line.
    fn round_trip(&mut self, line: &str) -> std::io::Result<String>;
}

/// Transport backed by a child process's piped stdin/stdout.
struct ChildTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Transport for ChildTransport {
    fn round_trip(&mut self, line: &str) -> std::io::Result<String> {
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;

        let mut response = String::new();
        // A zero-length read means the plugin closed stdout (e.g. it crashed or
        // exited) — surface that rather than treating it as an empty response.
        if self.stdout.read_line(&mut response)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "plugin closed its output stream",
            ));
        }
        Ok(response)
    }
}

impl Drop for ChildTransport {
    fn drop(&mut self) {
        // Best-effort shutdown: killing and reaping avoids leaking a zombie process.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── External module ───────────────────────────────────────────────────────────

/// A module whose methods are served by an external plugin process.
pub struct ExternalModule {
    name: String,
    methods: Vec<MethodSig>,
    transport: Mutex<Box<dyn Transport>>,
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
            method,
            args: args
                .iter()
                .map(|a| WireArg { name: a.name.clone(), value: WireValue::from_value(&a.value) })
                .collect(),
        };
        let line = serde_json::to_string(&request)
            .map_err(|e| cx.error(format!("plugin '{}': encoding call failed: {}", self.name, e)))?;

        let response = {
            let mut transport = self
                .transport
                .lock()
                .map_err(|_| cx.error(format!("plugin '{}' transport is poisoned", self.name)))?;
            transport
                .round_trip(&line)
                .map_err(|e| cx.error(format!("plugin '{}' call failed: {}", self.name, e)))?
        };

        match serde_json::from_str::<CallResponse>(response.trim()) {
            Ok(CallResponse::Ok(value)) => Ok(value.into_value()),
            Ok(CallResponse::Err(message)) => {
                Err(cx.error(format!("{}.{}: {}", self.name, method, message)))
            }
            Err(e) => Err(cx.error(format!(
                "plugin '{}' returned a malformed response: {}",
                self.name, e
            ))),
        }
    }
}

/// Spawn the plugin at `exe`, run `describe`, and build an [`ExternalModule`] keyed
/// by the discovered `name`.
pub(super) fn load(name: &str, exe: &Path) -> Result<ExternalModule> {
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Let the plugin's stderr pass through for diagnostics.
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| plugin_err(format!("failed to start plugin '{name}' ({}): {e}", exe.display())))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| plugin_err(format!("plugin '{name}': could not open stdin")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| plugin_err(format!("plugin '{name}': could not open stdout")))?;

    let mut transport = ChildTransport { child, stdin, stdout: BufReader::new(stdout) };

    let describe_line = serde_json::to_string(&Request::Describe).expect("Describe serializes");
    let response = transport
        .round_trip(&describe_line)
        .map_err(|e| plugin_err(format!("plugin '{name}': describe failed: {e}")))?;
    let describe: DescribeResponse = serde_json::from_str(response.trim())
        .map_err(|e| plugin_err(format!("plugin '{name}': malformed describe response: {e}")))?;

    // The discovered name is authoritative (it is what scripts import); a mismatched
    // self-reported name is tolerated but ignored.
    let _ = describe.name;

    Ok(ExternalModule {
        name: name.to_string(),
        methods: describe.methods,
        transport: Mutex::new(Box::new(transport)),
    })
}

fn plugin_err(msg: impl Into<String>) -> Error {
    Error::Eval(vec![Diagnostic::new(msg)])
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// A resolved map of plugin module name → executable path, gathered from a
/// project's plugin sources.
pub struct PluginIndex {
    entries: HashMap<String, PathBuf>,
}

impl PluginIndex {
    /// The executable path for `name`, if discovered.
    pub fn get(&self, name: &str) -> Option<&Path> {
        self.entries.get(name).map(PathBuf::as_path)
    }

    /// All discovered plugin names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

/// Discover plugins for the project rooted at `project_dir`, from two sources:
///
/// 1. `.mainstage/plugins/` — each executable file is a plugin named after the file;
///    one level of subdirectories provides namespaces (`acme/lint`).
/// 2. `plugins.toml` — a `[plugins]` table mapping names to executable paths
///    (relative to the project root or absolute). Manifest entries win over
///    convention so a project can override discovery explicitly.
///
/// Built-in module names are never consulted here, so plugins can never shadow them.
pub(super) fn discover(project_dir: &Path) -> PluginIndex {
    let mut entries = HashMap::new();
    scan_plugins_dir(&project_dir.join(".mainstage").join("plugins"), &mut entries);
    load_manifest(&project_dir.join("plugins.toml"), project_dir, &mut entries);
    PluginIndex { entries }
}

fn scan_plugins_dir(dir: &Path, entries: &mut HashMap<String, PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // One level of namespacing: <namespace>/<plugin>.
            let namespace = entry.file_name().to_string_lossy().into_owned();
            if let Ok(inner) = std::fs::read_dir(&path) {
                for sub in inner.flatten() {
                    let sub_path = sub.path();
                    if is_executable(&sub_path)
                        && let Some(stem) = plugin_name(&sub_path)
                    {
                        entries.insert(format!("{namespace}/{stem}"), sub_path);
                    }
                }
            }
        } else if is_executable(&path)
            && let Some(stem) = plugin_name(&path)
        {
            entries.insert(stem, path);
        }
    }
}

fn load_manifest(path: &Path, project_dir: &Path, entries: &mut HashMap<String, PathBuf>) {
    #[derive(Deserialize)]
    struct Manifest {
        #[serde(default)]
        plugins: HashMap<String, String>,
    }

    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(manifest) = toml::from_str::<Manifest>(&text) else {
        return;
    };
    for (name, rel) in manifest.plugins {
        let raw = PathBuf::from(&rel);
        let resolved = if raw.is_absolute() { raw } else { project_dir.join(raw) };
        entries.insert(name, resolved);
    }
}

/// The plugin name for an executable path: the file name, minus a `.exe` suffix.
fn plugin_name(path: &Path) -> Option<String> {
    let ext_is_exe = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let os = if ext_is_exe { path.file_stem() } else { path.file_name() };
    os.map(|s| s.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use crate::modules::ValueTy;

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    // ── Wire value round-trip ─────────────────────────────────────────────────

    fn round_trip(value: Value) -> Value {
        let json = serde_json::to_string(&WireValue::from_value(&value)).unwrap();
        let wire: WireValue = serde_json::from_str(&json).unwrap();
        wire.into_value()
    }

    #[test]
    fn wire_value_round_trips_scalars_and_lists() {
        assert!(matches!(round_trip(Value::String("x".into())), Value::String(s) if s == "x"));
        assert!(matches!(round_trip(Value::Bool(true)), Value::Bool(true)));
        match round_trip(Value::List(vec![Value::String("a".into()), Value::Bool(false)])) {
            Value::List(items) => assert_eq!(items.len(), 2),
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn wire_value_string_shape_is_tagged() {
        let json = serde_json::to_string(&WireValue::from_value(&Value::String("hi".into()))).unwrap();
        assert_eq!(json, r#"{"type":"string","value":"hi"}"#);
    }

    #[test]
    fn fileset_round_trips_via_path() {
        let value = Value::FileSet(vec![FileEntry::from_path(PathBuf::from("/src/main.rs"))]);
        match round_trip(value) {
            Value::FileSet(entries) => {
                assert_eq!(entries[0].name, "main.rs");
                assert_eq!(entries[0].ext, "rs");
            }
            other => panic!("expected fileset, got {other:?}"),
        }
    }

    // ── Protocol over a mock transport ────────────────────────────────────────

    /// A scripted transport: answers `describe` with a fixed module, and each
    /// `call` with the next queued response line.
    struct MockTransport {
        describe: String,
        call_responses: Vec<String>,
    }

    impl Transport for MockTransport {
        fn round_trip(&mut self, line: &str) -> std::io::Result<String> {
            if line.contains("\"describe\"") {
                Ok(self.describe.clone())
            } else if self.call_responses.is_empty() {
                Ok(r#"{"err":"no scripted response"}"#.to_string())
            } else {
                Ok(self.call_responses.remove(0))
            }
        }
    }

    fn module_with(call_responses: Vec<&str>) -> ExternalModule {
        let mut transport = MockTransport {
            describe: r#"{"name":"demo","methods":[
                {"name":"shout","params":[{"name":"s","ty":"string"}],"returns":"string"}
            ]}"#
            .to_string(),
            call_responses: call_responses.into_iter().map(String::from).collect(),
        };
        // Mirror `load`'s describe handshake.
        let describe_line = serde_json::to_string(&Request::Describe).unwrap();
        let response = transport.round_trip(&describe_line).unwrap();
        let describe: DescribeResponse = serde_json::from_str(response.trim()).unwrap();
        ExternalModule {
            name: "demo".to_string(),
            methods: describe.methods,
            transport: Mutex::new(Box::new(transport)),
        }
    }

    fn arg(v: &str) -> ResolvedArg {
        ResolvedArg { name: None, value: Value::String(v.to_string()) }
    }

    #[test]
    fn describe_populates_methods() {
        let module = module_with(vec![]);
        assert_eq!(module.name(), "demo");
        let sig = &module.methods()[0];
        assert_eq!(sig.name, "shout");
        assert_eq!(sig.returns, ValueTy::String);
        // A param with no explicit `required` defaults to required.
        assert!(sig.params[0].required);
    }

    #[test]
    fn call_ok_returns_value() {
        let module = module_with(vec![r#"{"ok":{"type":"string","value":"HELLO"}}"#]);
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: Path::new(".") };
        let result = module.call("shout", &[arg("hello")], &cx).unwrap();
        assert!(matches!(result, Value::String(s) if s == "HELLO"));
    }

    #[test]
    fn call_err_maps_to_eval_error_with_span() {
        let module = module_with(vec![r#"{"err":"boom"}"#]);
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: Path::new(".") };
        let err = module.call("shout", &[arg("x")], &cx).unwrap_err();
        match err {
            Error::Eval(diags) => {
                assert!(diags[0].message.contains("boom"));
                assert!(diags[0].span.is_some(), "error should carry the call span");
            }
            other => panic!("expected Error::Eval, got {other:?}"),
        }
    }

    #[test]
    fn malformed_response_errors() {
        let module = module_with(vec!["not json"]);
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: Path::new(".") };
        assert!(module.call("shout", &[arg("x")], &cx).is_err());
    }

    // ── Discovery ─────────────────────────────────────────────────────────────

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_plugin_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, body).unwrap();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn manifest_entries_are_discovered() {
        let dir = unique_dir("manifest");
        std::fs::write(
            dir.join("plugins.toml"),
            "[plugins]\nlint = \"tools/lint\"\n\"acme/fmt\" = \"/opt/fmt\"\n",
        )
        .unwrap();

        let index = discover(&dir);
        assert_eq!(index.get("lint"), Some(dir.join("tools/lint").as_path()));
        assert_eq!(index.get("acme/fmt"), Some(Path::new("/opt/fmt")));
        assert!(index.get("missing").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn plugins_dir_discovers_executables_and_namespaces() {
        let dir = unique_dir("dir");
        let plugins = dir.join(".mainstage").join("plugins");
        std::fs::create_dir_all(plugins.join("acme")).unwrap();
        write_executable(&plugins.join("lint"), "#!/bin/sh\n");
        write_executable(&plugins.join("acme").join("fmt"), "#!/bin/sh\n");
        // A non-executable file is not a plugin.
        std::fs::write(plugins.join("README"), "hi").unwrap();

        let index = discover(&dir);
        assert!(index.get("lint").is_some());
        assert!(index.get("acme/fmt").is_some());
        assert!(index.get("README").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── End-to-end with a real subprocess (unix) ──────────────────────────────

    #[cfg(unix)]
    #[test]
    fn real_subprocess_describe_and_call() {
        let dir = unique_dir("e2e");
        let exe = dir.join("echo-plugin");
        // A minimal line-oriented plugin: describe once, then echo an uppercase-ish
        // constant for any call. Validates spawn + describe + call end to end.
        write_executable(
            &exe,
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *describe*) printf '%s\n' '{"name":"echo","methods":[{"name":"ping","returns":"string"}]}' ;;
    *) printf '%s\n' '{"ok":{"type":"string","value":"pong"}}' ;;
  esac
done
"#,
        );

        let module = load("echo", &exe).expect("plugin should load");
        assert_eq!(module.name(), "echo");
        assert_eq!(module.methods()[0].name, "ping");

        let span = span();
        let cx = ModuleCx { span: &span, script_dir: &dir };
        let result = module.call("ping", &[], &cx).unwrap();
        assert!(matches!(result, Value::String(s) if s == "pong"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
