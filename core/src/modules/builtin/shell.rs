//! `shell` module — run an external command and capture its stdout.
//!
//! Gated on the [`Run`](crate::modules::Capability::Run) capability: a script may only
//! spawn a process when the user grants it with `--allow-run` or a manifest
//! `[permissions]` block. The command string is tokenized into argv exactly like the
//! `$` exec step (no shell is involved); a non-zero exit is reported as an error.

use std::sync::LazyLock;

use crate::error::Result;
use crate::eval::Value;
use crate::executor::tokenize_command;
use crate::modules::{
    require_positional_string, Capability, MethodSig, Module, ModuleCx, Param, ResolvedArg, ValueTy,
};

/// `shell.run("git rev-parse HEAD")` → captured stdout (trailing newline trimmed).
pub struct ShellModule;

static METHODS: LazyLock<Vec<MethodSig>> = LazyLock::new(|| {
    vec![MethodSig {
        name: "run".to_string(),
        params: vec![Param { name: "command".to_string(), ty: ValueTy::String, required: true }],
        named: vec![],
        returns: ValueTy::String,
    }]
});

impl Module for ShellModule {
    fn name(&self) -> &str {
        "shell"
    }

    fn methods(&self) -> &[MethodSig] {
        &METHODS
    }

    fn call(&self, method: &str, args: &[ResolvedArg], cx: &ModuleCx) -> Result<Value> {
        match method {
            "run" => {
                cx.require(Capability::Run)?;
                let command = require_positional_string(args, 0, "shell.run", cx)?;
                let argv = tokenize_command(&command, cx.span)?;
                if argv.is_empty() {
                    return Err(cx.error("shell.run: empty command"));
                }

                let output = std::process::Command::new(&argv[0])
                    .args(&argv[1..])
                    .current_dir(cx.script_dir)
                    .output()
                    .map_err(|e| cx.error(format!("shell.run '{}': {}", argv[0], e)))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(cx.error(format!(
                        "shell.run '{}' exited with {}: {}",
                        argv[0],
                        output.status,
                        stderr.trim()
                    )));
                }
                Ok(Value::String(String::from_utf8_lossy(&output.stdout).trim_end().to_string()))
            }
            _ => Err(cx.error(format!("shell has no method '{}'", method))),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use crate::modules::Permissions;
    use std::path::{Path, PathBuf};

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    fn call_with(method: &str, arg: &str, perms: Permissions) -> Result<Value> {
        let span = span();
        let cx = ModuleCx { span: &span, script_dir: Path::new("."), permissions: perms };
        let args = vec![ResolvedArg { name: None, value: Value::String(arg.to_string()) }];
        ShellModule.call(method, &args, &cx)
    }

    #[test]
    fn run_captures_stdout_when_allowed() {
        // `echo` prints its arguments back; argv tokenization splits the line. On
        // Windows `echo` is a `cmd` builtin rather than a standalone program, so route
        // through `cmd /C` there. Either way the trailing newline is trimmed.
        let command =
            if cfg!(windows) { "cmd /C echo hello world" } else { "echo hello world" };
        let out = call_with("run", command, Permissions::all()).unwrap();
        assert!(matches!(out, Value::String(s) if s == "hello world"));
    }

    #[test]
    fn run_denied_without_capability() {
        // Default permissions deny `run`; the gate fires before any process spawns.
        assert!(call_with("run", "echo hi", Permissions::default()).is_err());
    }

    #[test]
    fn run_nonzero_exit_errors() {
        assert!(call_with("run", "false", Permissions::all()).is_err());
    }

    #[test]
    fn unknown_method_errors() {
        assert!(call_with("nope", "x", Permissions::all()).is_err());
    }
}
