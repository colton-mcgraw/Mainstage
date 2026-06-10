//! Module System — trait-based registry.
//!
//! A [`Module`] exposes named methods callable from Mainstage scripts via
//! `import "<name>" as <alias>` declarations followed by `<alias>.<method>(...)`
//! calls. The [`ModuleRegistry`] resolves a raw module name to its implementation
//! and routes calls to it.
//!
//! Built-in modules (`env`, `git`, `str`, …) live in [`builtin`]; external
//! subprocess plugins live in [`external`]. Both implement the same [`Module`]
//! trait, so the evaluator and semantic analyzer never learn whether a module is
//! built-in or a plugin.
//!
//! This mirrors the [`Reporter`](crate::runner::Reporter) trait idiom: a small trait
//! with a registry that the rest of the crate is threaded through.

mod builtin;
mod external;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Diagnostic, Error, Result, Span};
use crate::eval::Value;

pub use builtin::{
    EnvModule, FsModule, GitModule, HashModule, JsonModule, PathModule, StrModule,
};
pub use external::{ExternalModule, PluginIndex};

// ── Resolved argument ─────────────────────────────────────────────────────────

/// A module-call argument whose expression has already been evaluated.
#[derive(Debug)]
pub struct ResolvedArg {
    /// `Some(name)` for keyword arguments (e.g. `short: true`); `None` for positional.
    pub name: Option<String>,
    pub value: Value,
}

// ── Value type tags ───────────────────────────────────────────────────────────

/// The runtime type of a [`Value`], used to declare and statically validate method
/// parameters and return values. Serialized in the plugin `describe` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueTy {
    String,
    Bool,
    List,
    FileSet,
    /// Matches any value type — for parameters that are intentionally untyped.
    Any,
}

impl Default for ValueTy {
    /// Untyped — the lenient default for plugin-declared signatures.
    fn default() -> Self {
        ValueTy::Any
    }
}

impl ValueTy {
    /// A human-readable name for diagnostics.
    pub fn describe(&self) -> &'static str {
        match self {
            ValueTy::String => "string",
            ValueTy::Bool => "bool",
            ValueTy::List => "list",
            ValueTy::FileSet => "fileset",
            ValueTy::Any => "any",
        }
    }

    /// Whether `value` is assignable to this declared type. `Any` matches everything.
    pub fn accepts(&self, value: &Value) -> bool {
        matches!(
            (self, value),
            (ValueTy::Any, _)
                | (ValueTy::String, Value::String(_))
                | (ValueTy::Bool, Value::Bool(_))
                | (ValueTy::List, Value::List(_))
                | (ValueTy::FileSet, Value::FileSet(_))
        )
    }
}

// ── Method signatures ─────────────────────────────────────────────────────────

/// A positional parameter in a [`MethodSig`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(default)]
    pub ty: ValueTy,
    /// `false` for trailing optional positionals (defines the minimum arity).
    /// Defaults to `true` so plugin signatures can omit it for required params.
    #[serde(default = "default_true")]
    pub required: bool,
}

/// A keyword (named) parameter in a [`MethodSig`], e.g. `default:` in `env.get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedParam {
    pub name: String,
    #[serde(default)]
    pub ty: ValueTy,
    /// Defaults to `false`: keyword arguments are optional unless declared required.
    #[serde(default)]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// The signature of a single module method: its positional and keyword parameters
/// and its return type.
///
/// Owned (no borrows) so that built-in and plugin modules — the latter deserialized
/// from a subprocess `describe` response — share one representation. The semantic
/// analyzer consumes these to validate calls, and tooling (LSP, `mainstage modules`)
/// renders them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodSig {
    pub name: String,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub named: Vec<NamedParam>,
    #[serde(default)]
    pub returns: ValueTy,
}

impl MethodSig {
    /// The minimum number of positional arguments (count of required positionals).
    pub fn min_positional(&self) -> usize {
        self.params.iter().filter(|p| p.required).count()
    }

    /// The maximum number of positional arguments accepted.
    pub fn max_positional(&self) -> usize {
        self.params.len()
    }

    /// Find a declared keyword parameter by name.
    pub fn named_param(&self, name: &str) -> Option<&NamedParam> {
        self.named.iter().find(|p| p.name == name)
    }
}

// ── Call context ──────────────────────────────────────────────────────────────

/// Context handed to a [`Module`] on each call: the source span of the call (for
/// diagnostics) and the script directory (the working directory for side effects
/// such as running `git`).
pub struct ModuleCx<'a> {
    pub span: &'a Span,
    pub script_dir: &'a Path,
}

impl ModuleCx<'_> {
    /// Build an eval [`Error`] carrying this call's span.
    pub fn error(&self, msg: impl Into<String>) -> Error {
        Error::Eval(vec![Diagnostic::new(msg).with_span(self.span.clone())])
    }
}

// ── Module trait ──────────────────────────────────────────────────────────────

/// A callable module. Implemented by built-in modules and, in a later phase, by
/// external subprocess plugins.
///
/// `Send + Sync` so a [`ModuleRegistry`] (and thus an [`EvalContext`](crate::eval::EvalContext))
/// can be shared across threads.
pub trait Module: Send + Sync {
    /// The canonical module name as written in `import "<name>"` — not the local alias.
    fn name(&self) -> &str;

    /// The method signatures this module exposes, for validation and tooling.
    fn methods(&self) -> &[MethodSig];

    /// Route an already-evaluated call to one of this module's methods.
    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value>;
}

// ── Registry ──────────────────────────────────────────────────────────────────

/// Resolves module names to their [`Module`] implementations and routes calls.
///
/// `Arc`-backed, so cloning (as happens when an [`EvalContext`](crate::eval::EvalContext)
/// is cloned per stage / loop iteration) is cheap and shares one underlying table.
#[derive(Clone)]
pub struct ModuleRegistry {
    modules: Arc<HashMap<String, Arc<dyn Module>>>,
    /// Names of the built-in standard modules, which plugins may never shadow.
    builtins: Arc<HashSet<String>>,
}

impl ModuleRegistry {
    /// The registry of built-in standard-library modules.
    pub fn standard() -> Self {
        let mods: Vec<Arc<dyn Module>> = vec![
            Arc::new(EnvModule),
            Arc::new(GitModule),
            Arc::new(StrModule),
            Arc::new(PathModule),
            Arc::new(HashModule),
            Arc::new(FsModule),
            Arc::new(JsonModule),
        ];
        Self::from_modules(mods)
    }

    fn from_modules(mods: Vec<Arc<dyn Module>>) -> Self {
        let builtins = mods.iter().map(|m| m.name().to_string()).collect();
        let map = mods.into_iter().map(|m| (m.name().to_string(), m)).collect();
        Self { modules: Arc::new(map), builtins: Arc::new(builtins) }
    }

    /// Look up a module by its raw name.
    pub fn get(&self, name: &str) -> Option<&dyn Module> {
        self.modules.get(name).map(|m| m.as_ref())
    }

    /// Whether a module with this raw name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// Whether `name` is a built-in standard module (which plugins may not shadow).
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains(name)
    }

    /// Look up the signature of `module.method`, if both exist.
    pub fn method_sig(&self, module: &str, method: &str) -> Option<&MethodSig> {
        self.get(module)?.methods().iter().find(|m| m.name == method)
    }

    /// Route a call to `module.method`. `args` must already be evaluated.
    pub fn dispatch(
        &self,
        module: &str,
        method: &str,
        args: &[ResolvedArg],
        cx: &ModuleCx,
    ) -> Result<Value> {
        match self.get(module) {
            Some(m) => m.call(method, args, cx),
            None => Err(cx.error(format!("unknown module '{}'", module))),
        }
    }

    /// Register a plugin module. Errors if its name shadows a built-in or collides
    /// with an already-registered module. Call before the registry is shared/cloned.
    pub fn register_plugin(&mut self, module: Arc<dyn Module>) -> std::result::Result<(), String> {
        let name = module.name().to_string();
        if self.is_builtin(&name) {
            return Err(format!(
                "plugin '{name}' may not shadow the built-in module of the same name"
            ));
        }
        // Arc refcount is 1 before the registry is shared, so this mutates in place.
        let map = Arc::make_mut(&mut self.modules);
        if map.contains_key(&name) {
            return Err(format!("a plugin named '{name}' is already registered"));
        }
        map.insert(name, module);
        Ok(())
    }

    /// Discover and load the plugins named in `names` (typically the program's
    /// non-built-in `import` names) from `project_dir`, registering each.
    ///
    /// Built-in names and already-registered names are skipped, as are names with no
    /// discovered executable — those are left for semantic analysis to report as
    /// unknown modules. A discovered-but-unloadable plugin (failed spawn, bad
    /// `describe`) is a hard error.
    pub fn load_plugins(&mut self, names: &[&str], project_dir: &Path) -> Result<()> {
        let wanted: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !self.is_builtin(n) && !self.contains(n))
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }

        let index = external::discover(project_dir);
        for name in wanted {
            if let Some(path) = index.get(name) {
                let module = external::load(name, path)?;
                self.register_plugin(Arc::new(module))
                    .map_err(|e| Error::Eval(vec![Diagnostic::new(e)]))?;
            }
        }
        Ok(())
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `EvalContext` derives `Debug` and is printed by `mainstage eval`; render
        // the registry as its sorted module names rather than opaque trait objects.
        let mut names: Vec<&str> = self.modules.keys().map(String::as_str).collect();
        names.sort_unstable();
        f.debug_struct("ModuleRegistry").field("modules", &names).finish()
    }
}

// ── Shared argument helpers ───────────────────────────────────────────────────
//
// Used by built-in modules to extract typed arguments. Kept here so the same
// extraction (and error wording) is shared across modules.

/// Return the `idx`-th positional (unnamed) argument as a `String`, or error.
pub(crate) fn require_positional_string(
    args: &[ResolvedArg],
    idx: usize,
    fn_name: &str,
    cx: &ModuleCx,
) -> Result<String> {
    let positional: Vec<&ResolvedArg> = args.iter().filter(|a| a.name.is_none()).collect();
    match positional.get(idx) {
        Some(a) => match &a.value {
            Value::String(s) => Ok(s.clone()),
            _ => Err(cx.error(format!("{}: argument {} must be a string", fn_name, idx + 1))),
        },
        None => Err(cx.error(format!(
            "{} requires at least {} positional argument(s)",
            fn_name,
            idx + 1
        ))),
    }
}

/// Resolve `p` against `script_dir` when relative; absolute paths are returned as-is.
/// Used by I/O modules so script-relative paths behave consistently with `glob`.
pub(crate) fn resolve_path(script_dir: &Path, p: &str) -> PathBuf {
    let raw = PathBuf::from(p);
    if raw.is_absolute() { raw } else { script_dir.join(raw) }
}

/// Return the `idx`-th positional (unnamed) argument as a `List`, or error.
pub(crate) fn require_positional_list(
    args: &[ResolvedArg],
    idx: usize,
    fn_name: &str,
    cx: &ModuleCx,
) -> Result<Vec<Value>> {
    let positional: Vec<&ResolvedArg> = args.iter().filter(|a| a.name.is_none()).collect();
    match positional.get(idx) {
        Some(a) => match &a.value {
            Value::List(items) => Ok(items.clone()),
            _ => Err(cx.error(format!("{}: argument {} must be a list", fn_name, idx + 1))),
        },
        None => Err(cx.error(format!(
            "{} requires at least {} positional argument(s)",
            fn_name,
            idx + 1
        ))),
    }
}

/// Return the value of a named `String` argument, or `None` if absent or wrong type.
pub(crate) fn named_string(args: &[ResolvedArg], name: &str) -> Option<String> {
    args.iter()
        .find(|a| a.name.as_deref() == Some(name))
        .and_then(|a| match &a.value {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
}

/// Return the value of a named `Bool` argument, or `None` if absent or wrong type.
pub(crate) fn named_bool(args: &[ResolvedArg], name: &str) -> Option<bool> {
    args.iter()
        .find(|a| a.name.as_deref() == Some(name))
        .and_then(|a| match a.value {
            Value::Bool(b) => Some(b),
            _ => None,
        })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal in-process module for testing registry behavior without a plugin.
    struct Fake(&'static str);

    impl Module for Fake {
        fn name(&self) -> &str {
            self.0
        }
        fn methods(&self) -> &[MethodSig] {
            &[]
        }
        fn call(&self, _method: &str, _args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
            Err(cx.error("fake"))
        }
    }

    #[test]
    fn register_plugin_refuses_to_shadow_a_builtin() {
        let mut reg = ModuleRegistry::standard();
        let err = reg.register_plugin(Arc::new(Fake("str"))).unwrap_err();
        assert!(err.contains("shadow"), "got: {err}");
        // The built-in is untouched.
        assert!(reg.is_builtin("str"));
    }

    #[test]
    fn register_plugin_adds_then_rejects_duplicates() {
        let mut reg = ModuleRegistry::standard();
        reg.register_plugin(Arc::new(Fake("lint"))).unwrap();
        assert!(reg.contains("lint"));
        assert!(!reg.is_builtin("lint"));
        assert!(reg.register_plugin(Arc::new(Fake("lint"))).is_err());
    }

    #[test]
    fn load_plugins_skips_builtin_and_undiscovered_names() {
        // An empty project directory has no plugin sources, so loading is a no-op and
        // unknown names are simply left unregistered.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_loadplugins_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut reg = ModuleRegistry::standard();
        reg.load_plugins(&["str", "no_such_plugin"], &dir).unwrap();
        assert!(!reg.contains("no_such_plugin"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
