//! Plugin linting — validate a subprocess plugin against the wire protocol.
//!
//! Powers `mainstage plugin check`. The plugin is spawned and asked to `describe`
//! itself exactly as discovery would; the response is then checked against the rules
//! a plugin must satisfy to be usable from a script: a non-empty module name, valid
//! type tags, callable method and parameter names, no duplicates, and sane parameter
//! ordering. Findings are split into hard **errors** (the plugin will not work) and
//! **warnings** (it works, but violates a convention).
//!
//! The check is read-only: only `describe` is sent, never `call`, so linting never
//! triggers a plugin's side effects.

use std::path::Path;

use super::protocol::{DescribeResponse, Request, WireMethodSig};
use super::{PluginProcess, is_executable};
use crate::modules::ModuleRegistry;

/// The severity of a single [`LintFinding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    /// The plugin will not load or cannot be called from a script.
    Error,
    /// The plugin works but breaks a naming or protocol convention.
    Warning,
}

/// One issue found while linting a plugin.
#[derive(Debug, Clone)]
pub struct LintFinding {
    pub level: LintLevel,
    pub message: String,
}

impl LintFinding {
    fn error(message: impl Into<String>) -> Self {
        LintFinding { level: LintLevel::Error, message: message.into() }
    }

    fn warning(message: impl Into<String>) -> Self {
        LintFinding { level: LintLevel::Warning, message: message.into() }
    }
}

/// The result of linting one plugin executable.
#[derive(Debug, Clone)]
pub struct LintReport {
    /// The self-reported module name from `describe`, if the plugin responded.
    pub module_name: Option<String>,
    /// The number of methods the plugin declared.
    pub method_count: usize,
    pub findings: Vec<LintFinding>,
}

impl LintReport {
    fn new() -> Self {
        LintReport { module_name: None, method_count: 0, findings: Vec::new() }
    }

    fn error(mut self, message: impl Into<String>) -> Self {
        self.findings.push(LintFinding::error(message));
        self
    }

    /// Whether any finding is an [`LintLevel::Error`] — i.e. the plugin is broken.
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.level == LintLevel::Error)
    }

    /// Whether the plugin passed with no findings at all.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Lint the plugin executable at `exe`, spawning it with `script_dir` as its working
/// directory (matching how discovery runs it). Never returns `Err`: every problem —
/// including a failure to spawn or describe — is reported as a finding so the caller
/// renders one consistent report.
pub fn lint_plugin(exe: &Path, script_dir: &Path) -> LintReport {
    let report = LintReport::new();

    if !exe.exists() {
        return report.error(format!("executable not found: {}", exe.display()));
    }
    if !is_executable(exe) {
        return report.error(format!(
            "not executable: {} (on Unix, set the execute bit with `chmod +x`)",
            exe.display()
        ));
    }

    let mut process = match PluginProcess::spawn(exe, script_dir) {
        Ok(p) => p,
        Err(e) => return report.error(format!("failed to start plugin: {e}")),
    };
    let describe: DescribeResponse = match process.request(&Request::Describe) {
        Ok(d) => d,
        Err(e) => return report.error(format!("`describe` failed: {e}")),
    };

    check_describe(describe, report)
}

/// Validate a parsed `describe` response, accumulating findings into `report`.
fn check_describe(describe: DescribeResponse, mut report: LintReport) -> LintReport {
    report.module_name = Some(describe.name.clone());
    report.method_count = describe.methods.len();

    check_module_name(&describe.name, &mut report);

    if describe.methods.is_empty() {
        report.findings.push(LintFinding::warning(
            "plugin declares no methods — it cannot be called from a script",
        ));
    }

    let mut seen = std::collections::HashSet::new();
    for method in &describe.methods {
        check_method(method, &mut seen, &mut report);
    }

    report
}

/// Validate the self-reported module name. The name is advisory (discovery decides the
/// authoritative name), so issues here are warnings, not errors.
fn check_module_name(name: &str, report: &mut LintReport) {
    if name.trim().is_empty() {
        report
            .findings
            .push(LintFinding::warning("module name is empty — `describe` should report a `name`"));
        return;
    }
    // A namespaced name (`acme/lint`) is the recommended convention for third-party
    // plugins; each `/`-separated segment must itself be a usable identifier.
    for segment in name.split('/') {
        if !is_ident(segment) {
            report.findings.push(LintFinding::warning(format!(
                "module name '{name}' is not a valid identifier (segment '{segment}'); use \
                 letters, digits, and underscores, with '/' to namespace (e.g. 'acme/lint')"
            )));
            break;
        }
    }
    if ModuleRegistry::is_reserved_name(name) {
        report.findings.push(LintFinding::warning(format!(
            "module name '{name}' collides with a built-in module — discovery will refuse to load \
             a plugin under that name; choose a distinct (ideally namespaced) name"
        )));
    }
}

/// Validate one declared method: its name, parameter names and types, and ordering.
fn check_method(
    method: &WireMethodSig,
    seen: &mut std::collections::HashSet<String>,
    report: &mut LintReport,
) {
    let m = &method.name;
    if m.trim().is_empty() {
        report.findings.push(LintFinding::error("a method has an empty name"));
        return;
    }
    if !is_ident(m) {
        report.findings.push(LintFinding::error(format!(
            "method '{m}' is not a valid identifier — it could never be called from a script"
        )));
    }
    if !seen.insert(m.clone()) {
        report
            .findings
            .push(LintFinding::error(format!("method '{m}' is declared more than once")));
    }

    // Type tags must be ones the host understands; `into_sig` validates every tag.
    if let Err(e) = method.clone().into_sig() {
        report.findings.push(LintFinding::error(format!("method '{m}': {e}")));
    }

    // Parameter names must be unique and callable.
    let mut param_names = std::collections::HashSet::new();
    let mut required_after_optional = false;
    let mut saw_optional = false;
    for p in &method.params {
        check_param_name(m, &p.name, &mut param_names, report);
        if p.required && saw_optional {
            required_after_optional = true;
        }
        if !p.required {
            saw_optional = true;
        }
    }
    if required_after_optional {
        report.findings.push(LintFinding::warning(format!(
            "method '{m}': a required positional parameter follows an optional one — optional \
             positionals should come last"
        )));
    }
    for p in &method.named {
        check_param_name(m, &p.name, &mut param_names, report);
    }
}

/// Validate a single parameter name and record it for duplicate detection.
fn check_param_name(
    method: &str,
    name: &str,
    seen: &mut std::collections::HashSet<String>,
    report: &mut LintReport,
) {
    if !is_ident(name) {
        report.findings.push(LintFinding::error(format!(
            "method '{method}': parameter '{name}' is not a valid identifier"
        )));
    }
    if !seen.insert(name.to_string()) {
        report.findings.push(LintFinding::warning(format!(
            "method '{method}': parameter '{name}' is declared more than once"
        )));
    }
}

/// Whether `s` is a valid Mainstage identifier: a leading letter or underscore
/// followed by letters, digits, or underscores (mirrors the `ident` grammar rule).
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
