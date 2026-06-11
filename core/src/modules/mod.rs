//! Module System — trait-based registry.
//!
//! A [`Module`] exposes named methods callable from Mainstage scripts via
//! `import "<name>" as <alias>` declarations followed by `<alias>.<method>(...)`
//! calls. The [`ModuleRegistry`] resolves a raw module name to its implementation
//! and routes calls to it.
//!
//! Built-in modules (`env`, `git`) live in [`builtin`]. External subprocess
//! plugins will be added in a later phase behind this same [`Module`] trait, so the
//! evaluator and semantic analyzer never learn whether a module is built-in or not.
//!
//! This mirrors the [`Reporter`](crate::runner::Reporter) trait idiom: a small trait
//! with a registry that the rest of the crate is threaded through.

mod builtin;
mod permissions;
pub mod plugin;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Diagnostic, Error, Result, Span};
use crate::eval::Value;

pub use builtin::{
    EnvModule, FsModule, GitModule, HashModule, HttpModule, JsonModule, PathModule, ShellModule,
    StrModule, TimeModule,
};
pub use permissions::{Capability, Permissions};
pub use plugin::ExternalModule;

// ── Resolved argument ─────────────────────────────────────────────────────────

/// A module-call argument whose expression has already been evaluated.
#[derive(Debug)]
pub struct ResolvedArg {
    /// `Some(name)` for keyword arguments (e.g. `short: true`); `None` for positional.
    pub name: Option<String>,
    pub value: Value,
}

// ── Value type tags ───────────────────────────────────────────────────────────

/// The runtime type of a [`Value`], used to declare — and, in a later phase,
/// statically validate — method parameters and return values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueTy {
    String,
    Int,
    Bool,
    List,
    FileSet,
    /// Matches any value type — for parameters that are intentionally untyped.
    Any,
}

impl ValueTy {
    /// A human-readable name for diagnostics.
    pub fn describe(&self) -> &'static str {
        match self {
            ValueTy::String => "string",
            ValueTy::Int => "int",
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
                | (ValueTy::Int, Value::Int(_))
                | (ValueTy::Bool, Value::Bool(_))
                | (ValueTy::List, Value::List(_))
                | (ValueTy::FileSet, Value::FileSet(_))
        )
    }
}

// ── Method signatures ─────────────────────────────────────────────────────────

/// A positional parameter in a [`MethodSig`].
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: ValueTy,
    /// `false` for trailing optional positionals (defines the minimum arity).
    pub required: bool,
}

/// A keyword (named) parameter in a [`MethodSig`], e.g. `default:` in `env.get`.
#[derive(Debug, Clone)]
pub struct NamedParam {
    pub name: String,
    pub ty: ValueTy,
    pub required: bool,
}

/// The signature of a single module method: its positional and keyword parameters
/// and its return type.
///
/// Owned (no borrows) so that built-in and plugin modules — the latter
/// deserialized from a subprocess `describe` response in a later phase — share one
/// representation. The semantic analyzer will consume these to validate calls, and
/// tooling (LSP, `mainstage modules`) will render them.
#[derive(Debug, Clone)]
pub struct MethodSig {
    pub name: String,
    pub params: Vec<Param>,
    pub named: Vec<NamedParam>,
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

    /// Render this signature in call form for documentation and tooling, e.g.
    /// `get(var: string, default?: string) -> string`. Positional parameters are
    /// listed first, then keyword parameters (which are always passed by name at the
    /// call site); optional parameters are suffixed with `?`.
    pub fn signature(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(self.params.len() + self.named.len());
        for p in &self.params {
            let opt = if p.required { "" } else { "?" };
            parts.push(format!("{}{}: {}", p.name, opt, p.ty.describe()));
        }
        for p in &self.named {
            let opt = if p.required { "" } else { "?" };
            parts.push(format!("{}{}: {}", p.name, opt, p.ty.describe()));
        }
        format!("{}({}) -> {}", self.name, parts.join(", "), self.returns.describe())
    }
}

// ── Call context ──────────────────────────────────────────────────────────────

/// Context handed to a [`Module`] on each call: the source span of the call (for
/// diagnostics) and the script directory (the working directory for side effects
/// such as running `git`).
pub struct ModuleCx<'a> {
    pub span: &'a Span,
    pub script_dir: &'a Path,
    /// Capabilities granted to this run; consulted by side-effecting modules via
    /// [`ModuleCx::require`]. Defaults (everything denied) unless the user opts in.
    pub permissions: Permissions,
}

impl ModuleCx<'_> {
    /// Build an eval [`Error`] carrying this call's span.
    pub fn error(&self, msg: impl Into<String>) -> Error {
        Error::Eval(vec![Diagnostic::new(msg).with_span(self.span.clone())])
    }

    /// Ensure `cap` is granted, or fail with a diagnostic that names the flag and
    /// manifest key the user can add to grant it. Side-effecting modules call this
    /// before performing the gated operation.
    pub fn require(&self, cap: Capability) -> Result<()> {
        if self.permissions.grants(cap) {
            Ok(())
        } else {
            Err(self.error(format!(
                "permission denied: this operation needs the '{}' capability — re-run with {} \
                 or add `{} = true` under [permissions] in {}",
                cap.name(),
                cap.flag(),
                cap.name(),
                plugin::MANIFEST,
            )))
        }
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
    /// Capabilities granted to this run, threaded into every [`ModuleCx`] the
    /// evaluator builds. Defaults to all-denied; set via [`with_permissions`].
    ///
    /// [`with_permissions`]: ModuleRegistry::with_permissions
    permissions: Permissions,
}

impl ModuleRegistry {
    /// The registry of built-in standard-library modules.
    pub fn standard() -> Self {
        Self::from_modules(Self::standard_modules())
    }

    /// The standard registry extended with any external plugins discovered under
    /// `script_dir` (see [`plugin::discover`]). Built-in modules always take
    /// precedence — a plugin may never shadow one — and plugin processes are
    /// spawned here and kept alive for the lifetime of the returned registry.
    ///
    /// Pass the resulting registry to both [`analyze_with`](crate::sema::analyze_with)
    /// and [`eval_program_with`](crate::eval::eval_program_with) so plugin calls are
    /// validated identically to built-in ones.
    pub fn with_plugins(script_dir: &Path) -> Result<Self> {
        let mut mods: Vec<Arc<dyn Module>> = Self::standard_modules();
        let reserved: HashSet<String> = mods.iter().map(|m| m.name().to_string()).collect();
        for plugin in plugin::discover(script_dir, &reserved)? {
            mods.push(Arc::new(plugin));
        }
        Ok(Self::from_modules(mods))
    }

    /// The built-in standard-library modules, as trait objects.
    fn standard_modules() -> Vec<Arc<dyn Module>> {
        vec![
            Arc::new(EnvModule),
            Arc::new(GitModule),
            Arc::new(StrModule),
            Arc::new(PathModule),
            Arc::new(HashModule),
            Arc::new(FsModule),
            Arc::new(JsonModule),
            Arc::new(ShellModule),
            Arc::new(HttpModule),
            Arc::new(TimeModule),
        ]
    }

    fn from_modules(mods: Vec<Arc<dyn Module>>) -> Self {
        let map = mods.into_iter().map(|m| (m.name().to_string(), m)).collect();
        // Capabilities default to denied; the CLI opts in via `with_permissions`.
        Self { modules: Arc::new(map), permissions: Permissions::default() }
    }

    /// Return this registry with `permissions` granted. Threaded into every
    /// [`ModuleCx`] so side-effecting modules can gate on the granted capabilities.
    pub fn with_permissions(mut self, permissions: Permissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// The capabilities granted to this run.
    pub fn permissions(&self) -> Permissions {
        self.permissions
    }

    /// Look up a module by its raw name.
    pub fn get(&self, name: &str) -> Option<&dyn Module> {
        self.modules.get(name).map(|m| m.as_ref())
    }

    /// Whether a module with this raw name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// The names of every registered module (built-in and plugin), sorted. Used by
    /// the `mainstage modules` command and other tooling to enumerate capabilities.
    pub fn module_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.modules.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
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
