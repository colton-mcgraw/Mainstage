//! Phase 5 — Step Executor.
//!
//! Executes the step sequences inside `steps {}`, `on_failure {}`, and `on_success {}` blocks.
//! Each step runs synchronously; the first failure short-circuits the sequence.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result, Span},
    eval::{AssertionResult, EvalContext, FileEntry, Value, eval_condition, eval_expr},
};

// ── Public API ─────────────────────────────────────────────────────────────────

/// Execute a sequence of steps in order, stopping at the first failure.
///
/// A `let <ident> = <expr>;` step (Phase 44) binds a name for the steps that *follow* it
/// within this block: it extends the context lazily so each subsequent step — and any
/// nested block — resolves the binding. The extended context is local to this call, so the
/// binding falls out of scope when the block ends (and is rebuilt per iteration of a `for`).
pub fn execute_steps(steps: &[Step], ctx: &EvalContext) -> Result<()> {
    // `extended` holds the context once a local `let` has added a binding; until then we run
    // against the caller's `ctx` directly, cloning only when a block actually binds a local.
    let mut extended: Option<EvalContext> = None;
    for step in steps {
        let cur = extended.as_ref().unwrap_or(ctx);
        if let Step::Let(l) = step {
            let value = eval_expr(&l.value, cur)?;
            extended = Some(cur.with_local_let(l.name.clone(), value));
        } else {
            execute_step(step, cur)?;
        }
    }
    Ok(())
}

/// Execute a single step.
pub fn execute_step(step: &Step, ctx: &EvalContext) -> Result<()> {
    match step {
        Step::Exec(s) => exec_step(s, ctx),
        Step::Copy(s) => copy_step(s, ctx),
        Step::Move(s) => move_step(s, ctx),
        Step::Mkdir(s) => mkdir_step(s, ctx),
        Step::Delete(s) => delete_step(s, ctx),
        Step::Write(s) => write_step(s, ctx),
        Step::If(s) => if_step(s, ctx),
        Step::For(s) => for_step(s, ctx),
        Step::Try(s) => try_step(s, ctx),
        Step::Workdir(s) => workdir_step(s, ctx),
        Step::WithEnv(s) => with_env_step(s, ctx),
        Step::Expect(s) => expect_step(s, ctx),
        Step::Assert(s) => assert_step(s, ctx),
        Step::Log(s) => log_step(s, ctx),
        Step::Fail(s) => fail_step(s, ctx),
        // A local `let` only affects the steps that follow it within its block, which
        // `execute_steps` threads. Reached only on a direct single-step call: evaluate the
        // value (so its errors surface) and discard the binding, which has no successors here.
        Step::Let(s) => eval_expr(&s.value, ctx).map(|_| ()),
        // `use` steps are inlined into their stage before execution (Phase 46); reaching
        // one here means the template-lowering pass was skipped.
        Step::Use(_) => unreachable!("`use` steps are inlined before execution"),
    }
}

// ── Step handlers ──────────────────────────────────────────────────────────────

fn exec_step(s: &ExecStep, ctx: &EvalContext) -> Result<()> {
    let command = interpolate_exec_command(&s.command, ctx, &s.span)?;
    let argv = tokenize_command(&command, &s.span)?;
    if argv.is_empty() {
        return Err(step_err("empty exec command", &s.span));
    }
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(ctx.effective_cwd());
    cmd.envs(&ctx.env_overlay);

    // With an output sink (the parallel runner), capture stdout/stderr and append it
    // to the stage's buffer so concurrent stages never interleave on the terminal.
    // Without one (the sequential path), inherit the terminal and stream output live.
    let status = match &ctx.output {
        Some(sink) => {
            let out = cmd
                .output()
                .map_err(|e| step_err(format!("failed to run '{}': {}", argv[0], e), &s.span))?;
            sink.write(&out.stdout);
            sink.write(&out.stderr);
            out.status
        }
        None => cmd
            .status()
            .map_err(|e| step_err(format!("failed to run '{}': {}", argv[0], e), &s.span))?,
    };
    if !status.success() {
        return Err(step_err(format!("'{}' exited with {}", argv[0], status), &s.span));
    }
    Ok(())
}

fn copy_step(s: &CopyStep, ctx: &EvalContext) -> Result<()> {
    let src = eval_as_path(&s.src, ctx)?;
    let dest = eval_as_path(&s.dest, ctx)?;
    if src.is_dir() {
        copy_dir_recursive(&src, &dest, &s.span)
    } else {
        ensure_parent(&dest, &s.span)?;
        copy_file_force(&src, &dest, &s.span)
    }
}

fn move_step(s: &MoveStep, ctx: &EvalContext) -> Result<()> {
    let src = eval_as_path(&s.src, ctx)?;
    let dest = eval_as_path(&s.dest, ctx)?;
    ensure_parent(&dest, &s.span)?;
    // Try an atomic rename first; fall back to copy+delete across filesystems.
    if std::fs::rename(&src, &dest).is_err() {
        if src.is_dir() {
            copy_dir_recursive(&src, &dest, &s.span)?;
            std::fs::remove_dir_all(&src)
                .map_err(|e| step_err(format!("delete '{}': {}", src.display(), e), &s.span))?;
        } else {
            std::fs::copy(&src, &dest).map_err(|e| {
                step_err(format!("copy '{}' → '{}': {}", src.display(), dest.display(), e), &s.span)
            })?;
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
    let branch = if eval_condition(&s.condition, ctx)? { &s.then_steps } else { &s.else_steps };
    execute_steps(branch, ctx)
}

fn for_step(s: &ForStep, ctx: &EvalContext) -> Result<()> {
    let entries = eval_as_fileset(&s.iterable, ctx, &s.span)?;
    for entry in entries {
        // Per-file incremental change detection (Phase 38): when the runner has marked an
        // input file unchanged since the last successful run — and the stage's declared
        // outputs are all present — its output is already up to date, so this iteration's
        // body is skipped. `skip_inputs` is empty on a full run, so this is a no-op then.
        if ctx.skip_inputs.contains(&entry.path) {
            continue;
        }
        let iter_ctx = ctx.with_for_var(s.var.clone(), entry);
        execute_steps(&s.steps, &iter_ctx)?;
    }
    Ok(())
}

fn try_step(s: &TryStep, ctx: &EvalContext) -> Result<()> {
    // Best-effort: run the inner steps and swallow a failure so the stage continues.
    // A failing step still short-circuits the remaining steps inside the block (as a
    // normal sequence would), but the error is not propagated past `try`.
    let _ = execute_steps(&s.steps, ctx);
    Ok(())
}

/// `workdir <path> { … }` — run the inner steps with the working directory set to `<path>`.
/// A relative `<path>` is resolved against the current effective directory, so nested
/// `workdir` blocks compose.
fn workdir_step(s: &WorkdirStep, ctx: &EvalContext) -> Result<()> {
    let raw = eval_as_path_raw(&s.path, ctx)?;
    let dir = resolve_against(ctx.effective_cwd(), &raw);
    let child = ctx.with_workdir(dir);
    execute_steps(&s.steps, &child)
}

/// `with_env { KEY: <value>, … } { … }` — run the inner steps with extra environment
/// variables set on spawned commands. Merges over any overlay already in scope, with the
/// inner keys winning.
fn with_env_step(s: &WithEnvStep, ctx: &EvalContext) -> Result<()> {
    let mut overlay = ctx.env_overlay.clone();
    for binding in &s.vars {
        let value = eval_expr(&binding.value, ctx)?.display_string();
        overlay.insert(binding.key.clone(), value);
    }
    let child = ctx.with_env_overlay(overlay);
    execute_steps(&s.steps, &child)
}

// ── Diagnostic & control-flow steps (Phase 43) ───────────────────────────────────

/// `log "<msg>"` — print an interpolated progress message. The message is rendered by the
/// run's reporter (so `--quiet` and styling are honored centrally) and routed to the same
/// destination as a `$` exec step's output: the per-stage buffer under the parallel runner,
/// or straight to the terminal on the sequential path. Without a reporter (library use) it
/// is a no-op, matching the `NoopReporter` default.
fn log_step(s: &LogStep, ctx: &EvalContext) -> Result<()> {
    let message = eval_string_expr(&s.message, ctx)?;
    let Some(handle) = &ctx.reporter else {
        return Ok(());
    };
    let mut line = Vec::new();
    handle.0.step_log(&mut line, &message);
    if line.is_empty() {
        // The reporter suppressed the line (e.g. `--quiet`).
        return Ok(());
    }
    match &ctx.output {
        // Buffered (parallel) path: append to the stage's sink so the line interleaves with
        // `$` output in execution order and is flushed atomically with the rest of the stage.
        Some(sink) => sink.write(&line),
        // Sequential path: a single worker streams output live, so write straight to stdout.
        None => {
            use std::io::Write as _;
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(&line);
            let _ = out.flush();
        }
    }
    Ok(())
}

/// `fail "<reason>"` — fail the enclosing stage deliberately. The interpolated reason becomes
/// a user-facing `Error::Eval` carrying the step span, so it behaves like any other failed
/// step: a `try` block swallows it and a stage's `on_failure` block fires.
fn fail_step(s: &FailStep, ctx: &EvalContext) -> Result<()> {
    let reason = eval_string_expr(&s.reason, ctx)?;
    Err(step_err(reason, &s.span))
}

// ── Test-harness steps (Phase 39) ────────────────────────────────────────────────

fn assert_step(s: &AssertStep, ctx: &EvalContext) -> Result<()> {
    let actual = eval_expr(&s.actual, ctx)?.display_string();
    let expected = eval_string_expr(&s.expected, ctx)?;
    let passed = match_passes(s.op, &actual, &expected, &s.span)?;
    let description = format!("assert \"{actual}\" {} \"{expected}\"", match_op_word(s.op));
    let detail = (!passed).then(|| match_detail(s.op, &actual, &expected));
    record_assertion(ctx, &s.span, description, passed, detail)
}

/// Whether `actual` satisfies `op` against `expected`. The only fallible case is `matches`,
/// whose expected value must be a valid (anchored) glob pattern.
fn match_passes(op: MatchOp, actual: &str, expected: &str, span: &Span) -> Result<bool> {
    Ok(match op {
        MatchOp::Equals => actual == expected,
        MatchOp::Contains => actual.contains(expected),
        MatchOp::NotContains => !actual.contains(expected),
        MatchOp::StartsWith => actual.starts_with(expected),
        MatchOp::EndsWith => actual.ends_with(expected),
        MatchOp::Matches => compile_glob(expected, span)?.matches(actual),
    })
}

/// Compile an anchored glob pattern for the `matches` matcher, mapping a bad pattern to a
/// step error (reusing the `glob` crate rather than pulling in a regex dependency).
fn compile_glob(pattern: &str, span: &Span) -> Result<glob::Pattern> {
    glob::Pattern::new(pattern)
        .map_err(|e| step_err(format!("invalid `matches` pattern \"{pattern}\": {e}"), span))
}

/// The human-readable reason a failed `assert` did not hold, shown in the test report.
fn match_detail(op: MatchOp, actual: &str, expected: &str) -> String {
    match op {
        MatchOp::Equals => format!("expected \"{expected}\", got \"{actual}\""),
        MatchOp::Contains => format!("\"{actual}\" does not contain \"{expected}\""),
        MatchOp::NotContains => format!("\"{actual}\" unexpectedly contains \"{expected}\""),
        MatchOp::StartsWith => format!("\"{actual}\" does not start with \"{expected}\""),
        MatchOp::EndsWith => format!("\"{actual}\" does not end with \"{expected}\""),
        MatchOp::Matches => format!("\"{actual}\" does not match \"{expected}\""),
    }
}

fn expect_step(s: &ExpectStep, ctx: &EvalContext) -> Result<()> {
    let command = interpolate_exec_command(&s.command, ctx, &s.span)?;
    let argv = tokenize_command(&command, &s.span)?;
    if argv.is_empty() {
        return Err(step_err("empty expect command", &s.span));
    }
    let timeout = s.timeout_secs.filter(|n| *n > 0).map(|n| Duration::from_secs(n as u64));

    // For an `output contains` check with a timeout, stop as soon as the marker appears so
    // a long-running (e.g. booting) process need not run out its full timeout.
    let stop_marker = match (&s.check, timeout) {
        (ExpectCheck::Output { op: MatchOp::Contains, expected }, Some(_)) => {
            Some(eval_string_expr(expected, ctx)?)
        }
        _ => None,
    };

    let cap = run_capture(
        &argv,
        ctx.effective_cwd(),
        &ctx.env_overlay,
        timeout,
        stop_marker.as_deref(),
        &s.span,
    )?;
    // Echo the command's captured output so it appears in the stage's (buffered) log,
    // matching the `$` exec step. The assertion is evaluated against the same bytes.
    if let Some(sink) = &ctx.output {
        sink.write(&cap.output);
    }
    let output = String::from_utf8_lossy(&cap.output);

    let timed = |n: i64| format!("command did not finish within {n}s");
    let (passed, detail) = match &s.check {
        ExpectCheck::Ok => {
            let ok = matches!(cap.status, Some(st) if st.success());
            let detail = (!ok).then(|| {
                if cap.timed_out {
                    timed(s.timeout_secs.unwrap_or(0))
                } else {
                    format!("command exited with {}", describe_status(&cap))
                }
            });
            (ok, detail)
        }
        ExpectCheck::Fails => {
            let failed = matches!(cap.status, Some(st) if !st.success());
            let detail = (!failed).then(|| {
                if cap.timed_out {
                    timed(s.timeout_secs.unwrap_or(0))
                } else {
                    "command unexpectedly succeeded".to_string()
                }
            });
            (failed, detail)
        }
        ExpectCheck::Output { op, expected } => {
            let want = eval_string_expr(expected, ctx)?;
            // Substring matchers scan the raw captured output; the "shape" matchers
            // (equals/starts_with/ends_with/matches) compare against trimmed output so a
            // trailing newline from the command doesn't defeat an otherwise-matching check.
            let subject = match op {
                MatchOp::Contains | MatchOp::NotContains => output.as_ref(),
                _ => output.trim(),
            };
            let ok = match_passes(*op, subject, &want, &s.span)?;
            let detail = (!ok).then(|| match op {
                MatchOp::Equals => format!("expected output \"{want}\", got \"{}\"", output.trim()),
                MatchOp::Contains => {
                    format!("output did not contain \"{want}\"{}", output_snippet(&output))
                }
                MatchOp::NotContains => {
                    format!("output unexpectedly contained \"{want}\"{}", output_snippet(&output))
                }
                MatchOp::StartsWith => {
                    format!("output did not start with \"{want}\"{}", output_snippet(&output))
                }
                MatchOp::EndsWith => {
                    format!("output did not end with \"{want}\"{}", output_snippet(&output))
                }
                MatchOp::Matches => {
                    format!("output did not match \"{want}\"{}", output_snippet(&output))
                }
            });
            (ok, detail)
        }
    };

    record_assertion(ctx, &s.span, describe_expect(s), passed, detail)
}

/// Record one assertion's outcome. In a `test` stage (the context carries a recorder) the
/// result is tallied and execution continues; otherwise a failed assertion aborts the step
/// like any other failure.
fn record_assertion(
    ctx: &EvalContext,
    span: &Span,
    description: String,
    passed: bool,
    detail: Option<String>,
) -> Result<()> {
    match &ctx.tests {
        Some(rec) => {
            rec.record(AssertionResult { description, passed, detail });
            Ok(())
        }
        None if passed => Ok(()),
        None => {
            let msg = match detail {
                Some(d) => format!("{description}: {d}"),
                None => description,
            };
            Err(step_err(msg, span))
        }
    }
}

/// Evaluate a `StringExpr` (resolving interpolation) to its string value.
fn eval_string_expr(s: &StringExpr, ctx: &EvalContext) -> Result<String> {
    match eval_expr(&Expr::String(s.clone()), ctx)? {
        Value::String(v) => Ok(v),
        _ => unreachable!("StringExpr always evaluates to Value::String"),
    }
}

fn match_op_word(op: MatchOp) -> &'static str {
    match op {
        MatchOp::Contains => "contains",
        MatchOp::Equals => "equals",
        MatchOp::NotContains => "not_contains",
        MatchOp::StartsWith => "starts_with",
        MatchOp::EndsWith => "ends_with",
        MatchOp::Matches => "matches",
    }
}

/// A one-line description of an `expect` step for the test report.
fn describe_expect(s: &ExpectStep) -> String {
    let check = match &s.check {
        ExpectCheck::Ok => "ok".to_string(),
        ExpectCheck::Fails => "fails".to_string(),
        ExpectCheck::Output { op, expected } => {
            format!("output {} \"{}\"", match_op_word(*op), stringexpr_preview(expected))
        }
    };
    let timeout = s.timeout_secs.map(|n| format!(" timeout {n}")).unwrap_or_default();
    format!("expect {check}{timeout} $ {}", s.command)
}

/// A literal-only preview of a `StringExpr` (interpolations shown as `${…}`), used only in
/// human-readable assertion descriptions.
fn stringexpr_preview(s: &StringExpr) -> String {
    let mut out = String::new();
    for part in &s.parts {
        match part {
            StringPart::Literal(t) => out.push_str(t),
            StringPart::Interpolation(_) => out.push_str("${…}"),
        }
    }
    out
}

fn describe_status(c: &Capture) -> String {
    match &c.status {
        Some(st) => st.to_string(),
        None => "no exit status".to_string(),
    }
}

/// A short, bounded tail of a command's captured output for an assertion-failure detail.
fn output_snippet(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return " (no output)".to_string();
    }
    let preview: String = trimmed.chars().take(200).collect();
    let ellipsis = if preview.chars().count() < trimmed.chars().count() { "…" } else { "" };
    format!("; got: {preview}{ellipsis}")
}

// ── Command capture ──────────────────────────────────────────────────────────────

/// The result of running a command under [`run_capture`].
struct Capture {
    /// Combined stdout then stderr.
    output: Vec<u8>,
    /// Exit status, or `None` if the command was killed (early stop on marker or timeout).
    status: Option<std::process::ExitStatus>,
    /// `true` if the command was killed because it exceeded its timeout.
    timed_out: bool,
}

/// Run `argv` in `cwd`, capturing combined stdout/stderr.
///
/// With no `timeout`, the command runs to completion. With a timeout, it is killed if it
/// does not finish in time (`timed_out = true`); when a `stop_marker` is given as well, the
/// command is also stopped early as soon as the captured output contains the marker, so a
/// never-exiting boot-smoke process is not forced to wait out the full timeout.
fn run_capture(
    argv: &[String],
    cwd: &Path,
    envs: &HashMap<String, String>,
    timeout: Option<Duration>,
    stop_marker: Option<&str>,
    span: &Span,
) -> Result<Capture> {
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(cwd);
    cmd.envs(envs);

    // Fast path: run to completion and let `output()` collect both streams.
    let Some(timeout) = timeout else {
        let out = cmd
            .output()
            .map_err(|e| step_err(format!("failed to run '{}': {}", argv[0], e), span))?;
        let mut buf = out.stdout;
        buf.extend_from_slice(&out.stderr);
        return Ok(Capture { output: buf, status: Some(out.status), timed_out: false });
    };

    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child =
        cmd.spawn().map_err(|e| step_err(format!("failed to run '{}': {}", argv[0], e), span))?;

    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let found = Arc::new(AtomicBool::new(false));
    let marker = stop_marker.filter(|m| !m.is_empty()).map(|m| m.as_bytes().to_vec());

    let readers = [
        child.stdout.take().map(|p| spawn_reader(p, &buf, &found, &marker)),
        child.stderr.take().map(|p| spawn_reader(p, &buf, &found, &marker)),
    ];

    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut timed_out = false;
    let mut killed = false;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                status = Some(st);
                break;
            }
            Ok(None) => {}
            Err(e) => return Err(step_err(format!("waiting on '{}': {}", argv[0], e), span)),
        }
        if found.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            killed = true;
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            killed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    if killed {
        // The child was killed (early-stop or timeout). Do *not* join the readers: a killed
        // process may have left a grandchild that still holds the pipe's write end open, so
        // a reader could block indefinitely. Give them a brief moment to flush, then take a
        // snapshot of what was captured and let the detached readers exit on their own EOF.
        std::thread::sleep(Duration::from_millis(50));
        drop(readers);
        let output = buf.lock().unwrap().clone();
        return Ok(Capture { output, status, timed_out });
    }

    // Normal exit: the pipes are closed, so the readers reach EOF and finish promptly.
    for handle in readers.into_iter().flatten() {
        let _ = handle.join();
    }
    let output = std::mem::take(&mut *buf.lock().unwrap());
    Ok(Capture { output, status, timed_out })
}

/// Spawn a thread that drains `reader` into the shared `buf`, flipping `found` once the
/// accumulated output contains `marker` (if any).
fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    buf: &Arc<Mutex<Vec<u8>>>,
    found: &Arc<AtomicBool>,
    marker: &Option<Vec<u8>>,
) -> std::thread::JoinHandle<()> {
    let buf = Arc::clone(buf);
    let found = Arc::clone(found);
    let marker = marker.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut b = buf.lock().unwrap();
                    b.extend_from_slice(&chunk[..n]);
                    if let Some(m) = &marker
                        && contains_subslice(&b, m)
                    {
                        found.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
    })
}

/// Whether `haystack` contains `needle` as a contiguous subslice.
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || (needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle))
}

// ── Exec helpers ───────────────────────────────────────────────────────────────

/// Substitute every `${ident}` or `${ident.field}` in `raw` with its evaluated value.
/// Interpolation is applied before tokenization so substituted values with spaces
/// are split normally unless the caller wraps them in quotes.
pub(crate) fn interpolate_exec_command(
    raw: &str,
    ctx: &EvalContext,
    span: &Span,
) -> Result<String> {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut inner = String::new();
            let mut closed = false;
            for ch in chars.by_ref() {
                if ch == '}' {
                    closed = true;
                    break;
                }
                inner.push(ch);
            }
            if !closed {
                return Err(step_err("unclosed `${` in exec command", span));
            }
            let expr = build_simple_expr(inner.trim(), span)?;
            let val = eval_expr(&expr, ctx)?;
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
                    Some(ch) => current.push(ch),
                    None => return Err(step_err("unclosed double quote in exec command", span)),
                }
            },
            '\'' => loop {
                match chars.next() {
                    Some('\'') => break,
                    Some(ch) => current.push(ch),
                    None => return Err(step_err("unclosed single quote in exec command", span)),
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
        let obj = obj.trim();
        let field = field.trim();
        if obj.is_empty() || field.is_empty() {
            return Err(step_err(format!("malformed interpolation expression '${{{}}}'", s), span));
        }
        Ok(Expr::MemberAccess(MemberAccessExpr {
            object: obj.to_string(),
            field: field.to_string(),
            span: span.clone(),
        }))
    } else {
        Ok(Expr::Ident(IdentExpr { name: s.to_string(), span: span.clone() }))
    }
}

// ── File-operation helpers ─────────────────────────────────────────────────────

/// Evaluate `expr` to a raw path string, without working-directory resolution.
fn eval_as_path_raw(expr: &Expr, ctx: &EvalContext) -> Result<PathBuf> {
    match eval_expr(expr, ctx)? {
        Value::String(s) => Ok(PathBuf::from(s)),
        _ => Err(step_err("expected a string (path) value", expr.span())),
    }
}

/// Evaluate `expr` to a path, resolving a relative path against the active working
/// directory (Phase 42 `workdir`; the script directory by default).
fn eval_as_path(expr: &Expr, ctx: &EvalContext) -> Result<PathBuf> {
    let raw = eval_as_path_raw(expr, ctx)?;
    Ok(resolve_against(ctx.effective_cwd(), &raw))
}

/// Join `p` onto `base` unless `p` is already absolute.
fn resolve_against(base: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
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
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs_create_dir_all(parent, span)?;
    }
    Ok(())
}

/// Copy a single file, force-overwriting the destination — like `cp -f`. If the
/// destination exists as a file it is removed first, so a read-only target (e.g. a
/// firmware variable store copied from a read-only source, whose permission bits the
/// previous copy carried over) can still be replaced instead of failing to open.
fn copy_file_force(src: &Path, dest: &Path, span: &Span) -> Result<()> {
    if dest.is_file() {
        let _ = std::fs::remove_file(dest);
    }
    std::fs::copy(src, dest).map(|_| ()).map_err(|e| {
        step_err(format!("copy '{}' → '{}': {}", src.display(), dest.display(), e), span)
    })
}

/// Recursively copy directory `src` into `dest`, creating `dest` if absent. Existing
/// files in `dest` are force-overwritten; files present only in `dest` are left in place
/// (use an explicit `delete` before `copy` for a clean replacement).
fn copy_dir_recursive(src: &Path, dest: &Path, span: &Span) -> Result<()> {
    fs_create_dir_all(dest, span)?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| step_err(format!("read dir '{}': {}", src.display(), e), span))?
    {
        let entry = entry
            .map_err(|e| step_err(format!("read entry in '{}': {}", src.display(), e), span))?;
        let ft = entry.file_type().map_err(|e| {
            step_err(format!("file type '{}': {}", entry.path().display(), e), span)
        })?;
        let dst = dest.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &dst, span)?;
        } else {
            copy_file_force(&entry.path(), &dst, span)?;
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
    use crate::modules::ModuleRegistry;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.ms"),
            line_start: 1,
            col_start: 1,
            line_end: 1,
            col_end: 1,
        }
    }

    fn ctx_with(lets: &[(&str, &str)]) -> EvalContext {
        EvalContext {
            script_dir: PathBuf::from("."),
            platform: "linux".to_string(),
            let_values: lets
                .iter()
                .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
                .collect(),
            project_fields: HashMap::new(),
            for_vars: HashMap::new(),
            matrix_vars: HashMap::new(),
            skip_inputs: std::collections::HashSet::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: std::collections::HashSet::new(),
            stage_output_refs: HashMap::new(),
            registry: ModuleRegistry::standard(),
            output: None,
            tests: None,
            cwd_override: None,
            env_overlay: HashMap::new(),
            reporter: None,
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
        assert_eq!(tokenize_command("  cargo   test  ", &span()).unwrap(), &["cargo", "test"]);
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

    // ── match_passes (Phase 45 matchers) ──────────────────────────────────────

    #[test]
    fn match_passes_all_matchers() {
        let p = |op, a, e| match_passes(op, a, e, &span()).unwrap();
        // equals / contains (the original pair).
        assert!(p(MatchOp::Equals, "demo", "demo"));
        assert!(!p(MatchOp::Equals, "demo", "demos"));
        assert!(p(MatchOp::Contains, "release-demo", "demo"));
        assert!(!p(MatchOp::Contains, "release", "demo"));
        // not_contains asserts absence.
        assert!(p(MatchOp::NotContains, "all clear", "ERROR"));
        assert!(!p(MatchOp::NotContains, "saw ERROR here", "ERROR"));
        // prefix / suffix.
        assert!(p(MatchOp::StartsWith, "1.2.0", "1."));
        assert!(!p(MatchOp::StartsWith, "1.2.0", "2."));
        assert!(p(MatchOp::EndsWith, "app.tar.gz", ".gz"));
        assert!(!p(MatchOp::EndsWith, "app.tar.gz", ".zip"));
        // anchored glob — the whole value must match.
        assert!(p(MatchOp::Matches, "1.2.0", "1.*.0"));
        assert!(p(MatchOp::Matches, "build-42", "build-??"));
        assert!(!p(MatchOp::Matches, "1.2.0-rc1", "1.*.0"));
    }

    #[test]
    fn match_passes_invalid_glob_errors() {
        // An unterminated character class is not a valid glob pattern.
        let err = match_passes(MatchOp::Matches, "x", "a[b", &span()).unwrap_err();
        assert!(format!("{err}").contains("invalid `matches` pattern"), "got: {err}");
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
            matrix_vars: HashMap::new(),
            skip_inputs: std::collections::HashSet::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: std::collections::HashSet::new(),
            stage_output_refs: HashMap::new(),
            registry: ModuleRegistry::standard(),
            output: None,
            tests: None,
            cwd_override: None,
            env_overlay: HashMap::new(),
            reporter: None,
        };
        let span = span();
        let path_expr = Expr::Ident(IdentExpr { name: "p".to_string(), span: span.clone() });

        // mkdir
        execute_step(&Step::Mkdir(MkdirStep { path: path_expr.clone(), span: span.clone() }), &ctx)
            .expect("mkdir should succeed");
        assert!(tmp.is_dir());

        // delete
        execute_step(
            &Step::Delete(DeleteStep { path: path_expr.clone(), span: span.clone() }),
            &ctx,
        )
        .expect("delete should succeed");
        assert!(!tmp.exists());

        // delete on missing path is a no-op
        execute_step(&Step::Delete(DeleteStep { path: path_expr, span }), &ctx)
            .expect("delete of missing path should be a no-op");
    }

    #[test]
    fn try_swallows_failure_and_stops_block() {
        // A failing step inside `try` does not propagate, but it does short-circuit the
        // remaining steps within the same block.
        let dir = std::env::temp_dir().join(format!("ms_try_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("after");
        let _ = std::fs::remove_file(&marker);

        let ctx = ctx_with(&[("m", marker.to_str().unwrap())]);
        let sp = span();
        let inner = vec![
            // A program that does not exist → the step fails.
            Step::Exec(ExecStep { command: "ms_no_such_binary_zzz".to_string(), span: sp.clone() }),
            // Should NOT run, because the failing step short-circuits the block.
            Step::Write(WriteStep {
                path: Expr::Ident(IdentExpr { name: "m".to_string(), span: sp.clone() }),
                content: StringExpr {
                    parts: vec![StringPart::Literal("x".to_string())],
                    span: sp.clone(),
                },
                span: sp.clone(),
            }),
        ];

        // The try step itself succeeds (failure swallowed)...
        execute_step(&Step::Try(TryStep { steps: inner, span: sp }), &ctx)
            .expect("try must not propagate the inner failure");
        // ...but the post-failure write inside the block did not run.
        assert!(!marker.exists(), "steps after a failure inside try are skipped");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_force_overwrites_readonly_destination() {
        let dir = std::env::temp_dir().join(format!("ms_copyf_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        let dest = dir.join("dest.bin");
        std::fs::write(&src, "new").unwrap();
        std::fs::write(&dest, "old").unwrap();

        // Make the destination read-only — a plain open-for-write would fail.
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&dest, perms).unwrap();

        let ctx = ctx_with(&[("s", src.to_str().unwrap()), ("d", dest.to_str().unwrap())]);
        let sp = span();
        execute_step(
            &Step::Copy(CopyStep {
                src: Expr::Ident(IdentExpr { name: "s".to_string(), span: sp.clone() }),
                dest: Expr::Ident(IdentExpr { name: "d".to_string(), span: sp.clone() }),
                span: sp,
            }),
            &ctx,
        )
        .expect("copy must force-overwrite a read-only destination");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn for_loop_skips_inputs_marked_unchanged() {
        // Phase 38: a file listed in `skip_inputs` has its loop iteration skipped, while
        // others run normally.
        let dir = std::env::temp_dir().join(format!("ms_incr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();

        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();
        // `b` is unchanged → skip its iteration.
        ctx.skip_inputs.insert(b.clone());

        let sp = span();
        // for f in inputs { write "<dir>/${f.stem}.out" content: "ran" }
        let for_step = Step::For(ForStep {
            var: "f".to_string(),
            iterable: Expr::Ident(IdentExpr { name: "inputs".to_string(), span: sp.clone() }),
            steps: vec![Step::Write(WriteStep {
                path: Expr::String(StringExpr {
                    parts: vec![
                        StringPart::Literal(format!("{}/", dir.display())),
                        StringPart::Interpolation(Box::new(Expr::MemberAccess(MemberAccessExpr {
                            object: "f".to_string(),
                            field: "stem".to_string(),
                            span: sp.clone(),
                        }))),
                        StringPart::Literal(".out".to_string()),
                    ],
                    span: sp.clone(),
                }),
                content: StringExpr {
                    parts: vec![StringPart::Literal("ran".to_string())],
                    span: sp.clone(),
                },
                span: sp.clone(),
            })],
            span: sp.clone(),
        });

        ctx.stage_inputs = Some(Value::FileSet(vec![
            FileEntry::from_path(a.clone()),
            FileEntry::from_path(b.clone()),
        ]));

        execute_step(&for_step, &ctx).expect("for loop should succeed");
        assert!(dir.join("a.out").exists(), "changed input's iteration ran");
        assert!(!dir.join("b.out").exists(), "unchanged input's iteration was skipped");

        let _ = std::fs::remove_dir_all(&dir);
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
            matrix_vars: HashMap::new(),
            skip_inputs: std::collections::HashSet::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: std::collections::HashSet::new(),
            stage_output_refs: HashMap::new(),
            registry: ModuleRegistry::standard(),
            output: None,
            tests: None,
            cwd_override: None,
            env_overlay: HashMap::new(),
            reporter: None,
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

    // ── workdir / with_env (Phase 42) ─────────────────────────────────────────

    /// A string-literal `Expr` for building step ASTs in tests.
    fn str_expr(s: &str) -> Expr {
        Expr::String(StringExpr { parts: vec![StringPart::Literal(s.to_string())], span: span() })
    }

    /// A `write <path> content: "<text>"` step.
    fn write(path: &str, text: &str) -> Step {
        Step::Write(WriteStep {
            path: str_expr(path),
            content: StringExpr {
                parts: vec![StringPart::Literal(text.to_string())],
                span: span(),
            },
            span: span(),
        })
    }

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ms_exec_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn workdir_resolves_relative_file_step_against_script_dir() {
        let dir = unique_dir("workdir");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();

        // workdir "sub" { write "f.txt" content: "x" }
        let step = Step::Workdir(WorkdirStep {
            path: str_expr("sub"),
            steps: vec![write("f.txt", "x")],
            span: span(),
        });
        execute_step(&step, &ctx).expect("workdir step should succeed");
        assert!(dir.join("sub").join("f.txt").exists(), "relative write lands in the workdir");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_workdir_composes() {
        let dir = unique_dir("workdir_nested");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();

        // workdir "a" { workdir "b" { write "f.txt" content: "x" } }
        let step = Step::Workdir(WorkdirStep {
            path: str_expr("a"),
            steps: vec![Step::Workdir(WorkdirStep {
                path: str_expr("b"),
                steps: vec![write("f.txt", "x")],
                span: span(),
            })],
            span: span(),
        });
        execute_step(&step, &ctx).expect("nested workdir should succeed");
        assert!(
            dir.join("a").join("b").join("f.txt").exists(),
            "the inner relative workdir resolves against the outer one"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absolute_path_ignores_workdir() {
        let dir = unique_dir("workdir_abs");
        let outside = unique_dir("workdir_abs_target");
        let target = outside.join("f.txt");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();

        // workdir "sub" { write "<absolute>/f.txt" content: "x" }
        let step = Step::Workdir(WorkdirStep {
            path: str_expr("sub"),
            steps: vec![write(target.to_str().unwrap(), "x")],
            span: span(),
        });
        execute_step(&step, &ctx).expect("workdir step should succeed");
        assert!(target.exists(), "an absolute path is unaffected by the active workdir");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn with_env_sets_command_environment() {
        use crate::eval::OutputSink;
        let dir = unique_dir("with_env");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();
        let sink = Arc::new(OutputSink::default());
        ctx.output = Some(Arc::clone(&sink));

        // with_env { MS_TEST_VAR: "hello42" } { $ sh -c "echo $MS_TEST_VAR" }
        let step = Step::WithEnv(WithEnvStep {
            vars: vec![EnvBinding {
                key: "MS_TEST_VAR".to_string(),
                value: str_expr("hello42"),
                span: span(),
            }],
            steps: vec![Step::Exec(ExecStep {
                command: "sh -c \"echo $MS_TEST_VAR\"".to_string(),
                span: span(),
            })],
            span: span(),
        });
        execute_step(&step, &ctx).expect("with_env step should succeed");
        let out = String::from_utf8_lossy(&sink.take()).to_string();
        assert!(out.contains("hello42"), "the env var reaches the command; got: {out}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn nested_with_env_merges_inner_overriding_outer() {
        use crate::eval::OutputSink;
        let dir = unique_dir("with_env_nested");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();
        let sink = Arc::new(OutputSink::default());
        ctx.output = Some(Arc::clone(&sink));

        // with_env { A: "1", B: "2" } { with_env { A: "9" } { $ sh -c "echo $A $B" } }
        let inner = Step::WithEnv(WithEnvStep {
            vars: vec![EnvBinding { key: "A".to_string(), value: str_expr("9"), span: span() }],
            steps: vec![Step::Exec(ExecStep {
                command: "sh -c \"echo $A $B\"".to_string(),
                span: span(),
            })],
            span: span(),
        });
        let step = Step::WithEnv(WithEnvStep {
            vars: vec![
                EnvBinding { key: "A".to_string(), value: str_expr("1"), span: span() },
                EnvBinding { key: "B".to_string(), value: str_expr("2"), span: span() },
            ],
            steps: vec![inner],
            span: span(),
        });
        execute_step(&step, &ctx).expect("nested with_env should succeed");
        let out = String::from_utf8_lossy(&sink.take()).to_string();
        // Inner `A` overrides outer; outer `B` is preserved through the merge.
        assert!(out.contains("9 2"), "inner overlay wins and outer keys survive; got: {out}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── log / fail (Phase 43) ──────────────────────────────────────────────────

    /// A reporter that renders `log` messages as `LOG:<msg>` so a test can observe routing.
    struct EchoReporter;
    impl crate::runner::Reporter for EchoReporter {
        fn step_log(&self, out: &mut dyn std::io::Write, message: &str) {
            let _ = write!(out, "LOG:{message}");
        }
    }

    /// A `log "<msg>"` step.
    fn log(message: &str) -> Step {
        Step::Log(LogStep {
            message: StringExpr {
                parts: vec![StringPart::Literal(message.to_string())],
                span: span(),
            },
            span: span(),
        })
    }

    #[test]
    fn log_routes_through_reporter_into_the_stage_sink() {
        use crate::eval::{OutputSink, ReporterHandle};
        let sink = Arc::new(OutputSink::default());
        let mut ctx = ctx_with(&[("who", "world")]);
        ctx.output = Some(Arc::clone(&sink));
        ctx.reporter = Some(ReporterHandle(Arc::new(EchoReporter)));

        // log "hi ${who}" → routed through the reporter, captured in the per-stage buffer.
        let step = Step::Log(LogStep {
            message: StringExpr {
                parts: vec![
                    StringPart::Literal("hi ".to_string()),
                    StringPart::Interpolation(Box::new(Expr::Ident(IdentExpr {
                        name: "who".to_string(),
                        span: span(),
                    }))),
                ],
                span: span(),
            },
            span: span(),
        });
        execute_step(&step, &ctx).expect("log step should succeed");
        assert_eq!(String::from_utf8_lossy(&sink.take()), "LOG:hi world");
    }

    #[test]
    fn log_with_no_reporter_is_a_silent_noop() {
        use crate::eval::OutputSink;
        let sink = Arc::new(OutputSink::default());
        let mut ctx = ctx_with(&[]);
        ctx.output = Some(Arc::clone(&sink));
        // No reporter installed (library use): the line is suppressed entirely.
        execute_step(&log("nothing prints"), &ctx).expect("log is a no-op without a reporter");
        assert!(sink.take().is_empty(), "log writes nothing when no reporter is present");
    }

    #[test]
    fn log_suppressed_when_reporter_writes_nothing() {
        use crate::eval::{OutputSink, ReporterHandle};
        // A quiet-style reporter renders nothing; the empty render must not reach the sink.
        struct QuietReporter;
        impl crate::runner::Reporter for QuietReporter {}
        let sink = Arc::new(OutputSink::default());
        let mut ctx = ctx_with(&[]);
        ctx.output = Some(Arc::clone(&sink));
        ctx.reporter = Some(ReporterHandle(Arc::new(QuietReporter)));
        execute_step(&log("hush"), &ctx).expect("log should succeed");
        assert!(sink.take().is_empty(), "a no-op reporter render produces no output");
    }

    #[test]
    fn fail_returns_an_error_with_the_interpolated_reason() {
        let ctx = ctx_with(&[("kind", "release")]);
        // fail "bad ${kind}" → an Eval error carrying the interpolated reason.
        let step = Step::Fail(FailStep {
            reason: StringExpr {
                parts: vec![
                    StringPart::Literal("bad ".to_string()),
                    StringPart::Interpolation(Box::new(Expr::Ident(IdentExpr {
                        name: "kind".to_string(),
                        span: span(),
                    }))),
                ],
                span: span(),
            },
            span: span(),
        });
        let err = execute_step(&step, &ctx).expect_err("fail must error");
        match err {
            Error::Eval(diags) => assert_eq!(diags[0].message, "bad release"),
            other => panic!("expected an Eval error, got {other:?}"),
        }
    }

    #[test]
    fn fail_inside_try_does_not_propagate() {
        let ctx = ctx_with(&[]);
        let step = Step::Try(TryStep { steps: vec![log_fail("boom")], span: span() });
        execute_step(&step, &ctx).expect("try swallows a fail step like any other failure");
    }

    /// A `fail "<reason>"` step.
    fn log_fail(reason: &str) -> Step {
        Step::Fail(FailStep {
            reason: StringExpr {
                parts: vec![StringPart::Literal(reason.to_string())],
                span: span(),
            },
            span: span(),
        })
    }

    // ── block-scoped local `let` (Phase 44) ────────────────────────────────────

    /// A `let <name> = "<text>";` step binding a string literal.
    fn local_let(name: &str, text: &str) -> Step {
        Step::Let(LetStep { name: name.to_string(), value: str_expr(text), span: span() })
    }

    #[test]
    fn local_let_is_visible_to_following_steps() {
        // let msg = "phase44"; write "<dir>/out.txt" content: "${msg}"
        let dir = unique_dir("local_let");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();
        let target = dir.join("out.txt");
        let write_using_local = Step::Write(WriteStep {
            path: str_expr(target.to_str().unwrap()),
            content: StringExpr {
                parts: vec![StringPart::Interpolation(Box::new(Expr::Ident(IdentExpr {
                    name: "msg".to_string(),
                    span: span(),
                })))],
                span: span(),
            },
            span: span(),
        });
        execute_steps(&[local_let("msg", "phase44"), write_using_local], &ctx)
            .expect("a following step should see the local binding");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "phase44");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_let_does_not_leak_out_of_the_block() {
        // The caller's context is untouched: a local bound inside `execute_steps` is not
        // visible to a sibling step list run against the same `ctx`.
        let ctx = ctx_with(&[]);
        execute_steps(&[local_let("scoped", "x")], &ctx).expect("inner block succeeds");
        // Evaluating `scoped` against the original ctx must fail — the binding fell out of scope.
        let ident = Expr::Ident(IdentExpr { name: "scoped".to_string(), span: span() });
        assert!(eval_expr(&ident, &ctx).is_err(), "the local must not leak into the outer scope");
    }

    #[test]
    fn local_let_shadows_an_earlier_local_via_reverse_lookup() {
        // sema forbids shadowing, but the evaluator's reverse `let` lookup must still resolve
        // the most-recent binding so a re-bound name reads as its latest value.
        let dir = unique_dir("local_let_rebind");
        let mut ctx = ctx_with(&[]);
        ctx.script_dir = dir.clone();
        let target = dir.join("v.txt");
        let write_v = Step::Write(WriteStep {
            path: str_expr(target.to_str().unwrap()),
            content: StringExpr {
                parts: vec![StringPart::Interpolation(Box::new(Expr::Ident(IdentExpr {
                    name: "v".to_string(),
                    span: span(),
                })))],
                span: span(),
            },
            span: span(),
        });
        execute_steps(&[local_let("v", "first"), local_let("v", "second"), write_v], &ctx)
            .expect("rebinding then reading succeeds");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
