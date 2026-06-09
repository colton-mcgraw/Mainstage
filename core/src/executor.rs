//! Phase 5 — Step Executor.
//!
//! Executes the step sequences inside `steps {}`, `on_failure {}`, and `on_success {}` blocks.
//! Each step runs synchronously; the first failure short-circuits the sequence.

use std::path::{Path, PathBuf};

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result, Span},
    eval::{eval_condition, eval_expr, EvalContext, FileEntry, Value},
};

// ── Public API ─────────────────────────────────────────────────────────────────

/// Execute a sequence of steps in order, stopping at the first failure.
pub fn execute_steps(steps: &[Step], ctx: &EvalContext) -> Result<()> {
    for step in steps {
        execute_step(step, ctx)?;
    }
    Ok(())
}

/// Execute a single step.
pub fn execute_step(step: &Step, ctx: &EvalContext) -> Result<()> {
    match step {
        Step::Exec(s)   => exec_step(s, ctx),
        Step::Copy(s)   => copy_step(s, ctx),
        Step::Move(s)   => move_step(s, ctx),
        Step::Mkdir(s)  => mkdir_step(s, ctx),
        Step::Delete(s) => delete_step(s, ctx),
        Step::Write(s)  => write_step(s, ctx),
        Step::If(s)     => if_step(s, ctx),
        Step::For(s)    => for_step(s, ctx),
    }
}

// ── Step handlers ──────────────────────────────────────────────────────────────

fn exec_step(s: &ExecStep, ctx: &EvalContext) -> Result<()> {
    let command = interpolate_exec_command(&s.command, ctx, &s.span)?;
    let argv = tokenize_command(&command, &s.span)?;
    if argv.is_empty() {
        return Err(step_err("empty exec command", &s.span));
    }
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&ctx.script_dir)
        .status()
        .map_err(|e| step_err(format!("failed to run '{}': {}", argv[0], e), &s.span))?;
    if !status.success() {
        return Err(step_err(format!("'{}' exited with {}", argv[0], status), &s.span));
    }
    Ok(())
}

fn copy_step(s: &CopyStep, ctx: &EvalContext) -> Result<()> {
    let src  = eval_as_path(&s.src, ctx)?;
    let dest = eval_as_path(&s.dest, ctx)?;
    if src.is_dir() {
        copy_dir_recursive(&src, &dest, &s.span)
    } else {
        ensure_parent(&dest, &s.span)?;
        std::fs::copy(&src, &dest)
            .map(|_| ())
            .map_err(|e| step_err(format!("copy '{}' → '{}': {}", src.display(), dest.display(), e), &s.span))
    }
}

fn move_step(s: &MoveStep, ctx: &EvalContext) -> Result<()> {
    let src  = eval_as_path(&s.src, ctx)?;
    let dest = eval_as_path(&s.dest, ctx)?;
    ensure_parent(&dest, &s.span)?;
    // Try an atomic rename first; fall back to copy+delete across filesystems.
    if std::fs::rename(&src, &dest).is_err() {
        if src.is_dir() {
            copy_dir_recursive(&src, &dest, &s.span)?;
            std::fs::remove_dir_all(&src)
                .map_err(|e| step_err(format!("delete '{}': {}", src.display(), e), &s.span))?;
        } else {
            std::fs::copy(&src, &dest)
                .map_err(|e| step_err(format!("copy '{}' → '{}': {}", src.display(), dest.display(), e), &s.span))?;
            std::fs::remove_file(&src)
                .map_err(|e| step_err(format!("delete '{}': {}", src.display(), e), &s.span))?;
        }
    }
    Ok(())
}

fn mkdir_step(s: &MkdirStep, ctx: &EvalContext) -> Result<()> {
    let path = eval_as_path(&s.path, ctx)?;
    fs_create_dir_all(&path, &s.span)
}

fn delete_step(s: &DeleteStep, ctx: &EvalContext) -> Result<()> {
    let path = eval_as_path(&s.path, ctx)?;
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        std::fs::remove_dir_all(&path)
            .map_err(|e| step_err(format!("delete '{}': {}", path.display(), e), &s.span))
    } else {
        std::fs::remove_file(&path)
            .map_err(|e| step_err(format!("delete '{}': {}", path.display(), e), &s.span))
    }
}

fn write_step(s: &WriteStep, ctx: &EvalContext) -> Result<()> {
    let path = eval_as_path(&s.path, ctx)?;
    let content = match eval_expr(&Expr::String(s.content.clone()), ctx)? {
        Value::String(c) => c,
        _ => unreachable!("StringExpr always evaluates to Value::String"),
    };
    ensure_parent(&path, &s.span)?;
    std::fs::write(&path, content)
        .map_err(|e| step_err(format!("write '{}': {}", path.display(), e), &s.span))
}

fn if_step(s: &IfStep, ctx: &EvalContext) -> Result<()> {
    let branch = if eval_condition(&s.condition, ctx)? {
        &s.then_steps
    } else {
        &s.else_steps
    };
    execute_steps(branch, ctx)
}

fn for_step(s: &ForStep, ctx: &EvalContext) -> Result<()> {
    let entries = eval_as_fileset(&s.iterable, ctx, &s.span)?;
    for entry in entries {
        let iter_ctx = ctx.with_for_var(s.var.clone(), entry);
        execute_steps(&s.steps, &iter_ctx)?;
    }
    Ok(())
}

// ── Exec helpers ───────────────────────────────────────────────────────────────

/// Substitute every `${ident}` or `${ident.field}` in `raw` with its evaluated value.
/// Interpolation is applied before tokenization so substituted values with spaces
/// are split normally unless the caller wraps them in quotes.
pub(crate) fn interpolate_exec_command(raw: &str, ctx: &EvalContext, span: &Span) -> Result<String> {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut inner = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '}' { closed = true; break; }
                inner.push(ch);
            }
            if !closed {
                return Err(step_err("unclosed `${` in exec command", span));
            }
            let expr = build_simple_expr(inner.trim(), span)?;
            let val  = eval_expr(&expr, ctx)?;
            result.push_str(&val.display_string());
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

/// Split a fully-interpolated command string into argv, respecting single and double quotes.
/// Does not perform shell expansion; quote characters are stripped.
pub(crate) fn tokenize_command(command: &str, span: &Span) -> Result<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            '"' => loop {
                match chars.next() {
                    Some('"') => break,
                    Some(ch)  => current.push(ch),
                    None      => return Err(step_err("unclosed double quote in exec command", span)),
                }
            },
            '\'' => loop {
                match chars.next() {
                    Some('\'') => break,
                    Some(ch)   => current.push(ch),
                    None       => return Err(step_err("unclosed single quote in exec command", span)),
                }
            },
            ch => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Build an `Expr` from a simple `ident` or `ident.field` string (used for exec interpolation).
fn build_simple_expr(s: &str, span: &Span) -> Result<Expr> {
    if s.is_empty() {
        return Err(step_err("empty interpolation '${}'", span));
    }
    if let Some((obj, field)) = s.split_once('.') {
        let obj   = obj.trim();
        let field = field.trim();
        if obj.is_empty() || field.is_empty() {
            return Err(step_err(format!("malformed interpolation expression '${{{}}}'", s), span));
        }
        Ok(Expr::MemberAccess(MemberAccessExpr {
            object: obj.to_string(),
            field:  field.to_string(),
            span:   span.clone(),
        }))
    } else {
        Ok(Expr::Ident(IdentExpr { name: s.to_string(), span: span.clone() }))
    }
}

// ── File-operation helpers ─────────────────────────────────────────────────────

fn eval_as_path(expr: &Expr, ctx: &EvalContext) -> Result<PathBuf> {
    match eval_expr(expr, ctx)? {
        Value::String(s) => Ok(PathBuf::from(s)),
        _ => Err(step_err("expected a string (path) value", expr.span())),
    }
}

fn eval_as_fileset(expr: &Expr, ctx: &EvalContext, span: &Span) -> Result<Vec<FileEntry>> {
    match eval_expr(expr, ctx)? {
        Value::FileSet(entries) => Ok(entries),
        _ => Err(step_err("`for` loop requires a fileset expression", span)),
    }
}

/// Create `path` and all its parent directories; succeeds silently if already present.
fn fs_create_dir_all(path: &Path, span: &Span) -> Result<()> {
    std::fs::create_dir_all(path)
        .map_err(|e| step_err(format!("mkdir '{}': {}", path.display(), e), span))
}

/// Ensure the parent directory of `path` exists.
fn ensure_parent(path: &Path, span: &Span) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs_create_dir_all(parent, span)?;
        }
    }
    Ok(())
}

/// Recursively copy directory `src` into `dest`, creating `dest` if absent.
fn copy_dir_recursive(src: &Path, dest: &Path, span: &Span) -> Result<()> {
    fs_create_dir_all(dest, span)?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| step_err(format!("read dir '{}': {}", src.display(), e), span))?
    {
        let entry = entry
            .map_err(|e| step_err(format!("read entry in '{}': {}", src.display(), e), span))?;
        let ft = entry
            .file_type()
            .map_err(|e| step_err(format!("file type '{}': {}", entry.path().display(), e), span))?;
        let dst = dest.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &dst, span)?;
        } else {
            std::fs::copy(entry.path(), &dst).map_err(|e| {
                step_err(format!("copy '{}' → '{}': {}", entry.path().display(), dst.display(), e), span)
            })?;
        }
    }
    Ok(())
}

// ── Error helper ───────────────────────────────────────────────────────────────

fn step_err(msg: impl Into<String>, span: &Span) -> Error {
    Error::Eval(vec![Diagnostic::new(msg).with_span(span.clone())])
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{EvalContext, Value};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn span() -> Span {
        Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
    }

    fn ctx_with(lets: &[(&str, &str)]) -> EvalContext {
        EvalContext {
            script_dir: PathBuf::from("."),
            platform: "linux".to_string(),
            let_values: lets.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect(),
            project_fields: HashMap::new(),
            for_vars: HashMap::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: std::collections::HashSet::new(),
            stage_output_refs: HashMap::new(),
        }
    }

    // ── tokenize_command ──────────────────────────────────────────────────────

    #[test]
    fn tokenize_simple() {
        assert_eq!(
            tokenize_command("cargo build --release", &span()).unwrap(),
            &["cargo", "build", "--release"]
        );
    }

    #[test]
    fn tokenize_double_quoted() {
        assert_eq!(
            tokenize_command(r#"echo "hello world""#, &span()).unwrap(),
            &["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_single_quoted() {
        assert_eq!(
            tokenize_command("echo 'hello world'", &span()).unwrap(),
            &["echo", "hello world"]
        );
    }

    #[test]
    fn tokenize_extra_whitespace() {
        assert_eq!(
            tokenize_command("  cargo   test  ", &span()).unwrap(),
            &["cargo", "test"]
        );
    }

    #[test]
    fn tokenize_unclosed_double_quote_errors() {
        assert!(tokenize_command(r#"echo "unterminated"#, &span()).is_err());
    }

    #[test]
    fn tokenize_unclosed_single_quote_errors() {
        assert!(tokenize_command("echo 'unterminated", &span()).is_err());
    }

    // ── interpolate_exec_command ──────────────────────────────────────────────

    #[test]
    fn interpolate_ident() {
        let ctx = ctx_with(&[("target", "release")]);
        assert_eq!(
            interpolate_exec_command("cargo build --profile ${target}", &ctx, &span()).unwrap(),
            "cargo build --profile release"
        );
    }

    #[test]
    fn interpolate_no_placeholders() {
        assert_eq!(
            interpolate_exec_command("cargo test", &ctx_with(&[]), &span()).unwrap(),
            "cargo test"
        );
    }

    #[test]
    fn interpolate_multiple() {
        let ctx = ctx_with(&[("cmd", "build"), ("flag", "--release")]);
        assert_eq!(
            interpolate_exec_command("cargo ${cmd} ${flag}", &ctx, &span()).unwrap(),
            "cargo build --release"
        );
    }

    #[test]
    fn interpolate_unclosed_brace_errors() {
        assert!(interpolate_exec_command("run ${unclosed", &ctx_with(&[]), &span()).is_err());
    }

    // ── build_simple_expr ─────────────────────────────────────────────────────

    #[test]
    fn build_ident_expr() {
        assert!(matches!(build_simple_expr("platform", &span()).unwrap(), Expr::Ident(_)));
    }

    #[test]
    fn build_member_expr() {
        assert!(matches!(
            build_simple_expr("project.name", &span()).unwrap(),
            Expr::MemberAccess(_)
        ));
    }

    #[test]
    fn build_empty_errors() {
        assert!(build_simple_expr("", &span()).is_err());
    }

    // ── file-system steps ─────────────────────────────────────────────────────

    #[test]
    fn mkdir_and_delete() {
        let tmp = std::env::temp_dir().join("ms_test_mkdir_delete");
        let ctx = EvalContext {
            script_dir: std::env::temp_dir(),
            platform: "linux".to_string(),
            let_values: vec![("p".to_string(), Value::String(tmp.display().to_string()))],
            project_fields: HashMap::new(),
            for_vars: HashMap::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: std::collections::HashSet::new(),
            stage_output_refs: HashMap::new(),
        };
        let span = span();
        let path_expr = Expr::Ident(IdentExpr { name: "p".to_string(), span: span.clone() });

        // mkdir
        execute_step(&Step::Mkdir(MkdirStep { path: path_expr.clone(), span: span.clone() }), &ctx)
            .expect("mkdir should succeed");
        assert!(tmp.is_dir());

        // delete
        execute_step(&Step::Delete(DeleteStep { path: path_expr.clone(), span: span.clone() }), &ctx)
            .expect("delete should succeed");
        assert!(!tmp.exists());

        // delete on missing path is a no-op
        execute_step(&Step::Delete(DeleteStep { path: path_expr, span }), &ctx)
            .expect("delete of missing path should be a no-op");
    }

    #[test]
    fn write_creates_file() {
        let tmp = std::env::temp_dir().join("ms_test_write.txt");
        let _ = std::fs::remove_file(&tmp);

        let ctx = EvalContext {
            script_dir: std::env::temp_dir(),
            platform: "linux".to_string(),
            let_values: vec![("p".to_string(), Value::String(tmp.display().to_string()))],
            project_fields: HashMap::new(),
            for_vars: HashMap::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: std::collections::HashSet::new(),
            stage_output_refs: HashMap::new(),
        };
        let span = span();
        let path_expr = Expr::Ident(IdentExpr { name: "p".to_string(), span: span.clone() });
        let content = StringExpr {
            parts: vec![StringPart::Literal("hello phase5".to_string())],
            span: span.clone(),
        };

        execute_step(&Step::Write(WriteStep { path: path_expr, content, span }), &ctx)
            .expect("write should succeed");

        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello phase5");
        let _ = std::fs::remove_file(&tmp);
    }
}
