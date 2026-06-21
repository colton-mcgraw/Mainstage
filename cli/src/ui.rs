//! Live-run HUD (`mainstage ui`).
//!
//! A terminal UI that renders a pipeline run as it happens: one line per stage with a
//! spinner and live status, a rolling progress summary, and a tail of the most recent
//! output. It is drawn in a fixed-height *inline viewport* at the bottom of the terminal
//! (the scrollback above is untouched), and torn down into a clean, permanent summary when
//! the run finishes.
//!
//! The UI never reaches into the runner: it is a [`Reporter`] like any other. Lifecycle
//! events update a shared [`HudState`] that a render loop reads, and the runner's textual
//! output is captured via [`Reporter::flush_block`] (the same hook that normally writes to
//! stdout) so it never corrupts the screen. `wants_buffered` forces output capture even at
//! `--jobs 1`. The pipeline runs on a background thread while the main thread renders.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use console::style;
use mainstage_core::{
    AnalysisResult, AssertionResult, CancelToken, Error, EvalContext, Reporter, ReporterHandle,
    RunState, RunStatus, StageOutcome, StageState, StageStatus, StatusRecorder, TeeReporter,
    ast::Program, critical_path, plan_pipeline, run_pipeline_cancellable,
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    crossterm::{
        ExecutableCommand,
        event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
        terminal::{
            EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, size,
        },
    },
    layout::Constraint,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

/// How a stage stands in the HUD, mirroring the runner's lifecycle events.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HudStatus {
    Queued,
    Running,
    Passed,
    Failed,
    AllowedFailure,
    Skipped,
    Restored,
    Cancelled,
}

impl HudStatus {
    /// Whether the stage has reached a terminal state (no longer queued or running).
    fn settled(self) -> bool {
        !matches!(self, HudStatus::Queued | HudStatus::Running)
    }
}

/// One stage's live row in the HUD.
#[derive(Clone)]
struct HudStage {
    name: String,
    status: HudStatus,
    /// When the stage started running, for the live elapsed clock on a running row.
    started: Option<Instant>,
    elapsed: Option<Duration>,
    /// `(passed, failed)` assertion tallies for a `test` stage.
    tests: Option<(usize, usize)>,
    /// The most recent line of command output, shown live while the stage runs (Phase 54).
    last_output: Option<String>,
    /// On failure, the error message, shown inline after `failed`.
    error: Option<String>,
}

/// Shared state the render loop reads and the [`HudReporter`] writes. Cloned under the lock
/// for each frame so rendering never holds the lock during I/O.
#[derive(Clone)]
struct HudState {
    pipeline: String,
    started: Instant,
    stages: Vec<HudStage>,
    index: HashMap<String, usize>,
    /// Captured command output and `log` lines, newest last — the run's scrollback.
    scrollback: Vec<String>,
    /// Per-stage failure messages, in completion order.
    errors: Vec<(String, String)>,
    /// Undeclared-input-audit findings per stage (Phase 53).
    audit: Vec<(String, Vec<String>)>,
}

impl HudState {
    fn new(pipeline: &str, order: &[String]) -> Self {
        let stages: Vec<HudStage> = order
            .iter()
            .map(|name| HudStage {
                name: name.clone(),
                status: HudStatus::Queued,
                started: None,
                elapsed: None,
                tests: None,
                last_output: None,
                error: None,
            })
            .collect();
        let index = stages.iter().enumerate().map(|(i, s)| (s.name.clone(), i)).collect();
        Self {
            pipeline: pipeline.to_string(),
            started: Instant::now(),
            stages,
            index,
            scrollback: Vec::new(),
            errors: Vec::new(),
            audit: Vec::new(),
        }
    }

    fn set_status(&mut self, name: &str, status: HudStatus) {
        if let Some(&i) = self.index.get(name) {
            self.stages[i].status = status;
        }
    }

    fn set_elapsed(&mut self, name: &str, elapsed: Duration) {
        if let Some(&i) = self.index.get(name) {
            self.stages[i].elapsed = Some(elapsed);
        }
    }

    /// `(done, running, total)` across all stages, where "done" counts every settled stage.
    fn counts(&self) -> (usize, usize, usize) {
        let total = self.stages.len();
        let running = self.stages.iter().filter(|s| s.status == HudStatus::Running).count();
        let done = self.stages.iter().filter(|s| s.status.settled()).count();
        (done, running, total)
    }
}

/// A [`Reporter`] that funnels lifecycle events into a shared [`HudState`].
struct HudReporter {
    state: Arc<Mutex<HudState>>,
}

impl HudReporter {
    fn push_lines(&self, text: &str) {
        let mut s = self.state.lock().unwrap();
        for line in text.lines() {
            s.scrollback.push(line.to_string());
        }
    }
}

impl Reporter for HudReporter {
    // Force output capture even with a single worker, so command output flows through
    // `flush_block` into our scrollback rather than onto the terminal we are drawing on.
    fn wants_buffered(&self) -> bool {
        true
    }

    // The runner's per-stage output block: captured into scrollback instead of stdout.
    fn flush_block(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.push_lines(&String::from_utf8_lossy(bytes));
    }

    fn step_log(&self, _out: &mut dyn std::io::Write, message: &str) {
        self.push_lines(&format!("› {message}"));
    }

    fn stage_start(&self, _out: &mut dyn std::io::Write, stage: &str) {
        let mut s = self.state.lock().unwrap();
        s.set_status(stage, HudStatus::Running);
        if let Some(&i) = s.index.get(stage) {
            s.stages[i].started = Some(Instant::now());
        }
    }

    fn stage_output_line(&self, stage: &str, line: &str) {
        if line.is_empty() {
            return;
        }
        let mut s = self.state.lock().unwrap();
        if let Some(&i) = s.index.get(stage) {
            s.stages[i].last_output = Some(line.to_string());
        }
    }

    fn stage_skipped(&self, _out: &mut dyn std::io::Write, stage: &str) {
        self.state.lock().unwrap().set_status(stage, HudStatus::Skipped);
    }

    fn stage_restored(&self, _out: &mut dyn std::io::Write, stage: &str) {
        self.state.lock().unwrap().set_status(stage, HudStatus::Restored);
    }

    fn stage_passed(&self, _out: &mut dyn std::io::Write, stage: &str) {
        self.state.lock().unwrap().set_status(stage, HudStatus::Passed);
    }

    fn stage_failed(
        &self,
        _out: &mut dyn std::io::Write,
        stage: &str,
        error: &Error,
        allow_failure: bool,
    ) {
        let mut s = self.state.lock().unwrap();
        let status = if allow_failure { HudStatus::AllowedFailure } else { HudStatus::Failed };
        s.set_status(stage, status);
        if !allow_failure {
            if let Some(&i) = s.index.get(stage) {
                s.stages[i].error = Some(error.to_string());
            }
            s.errors.push((stage.to_string(), error.to_string()));
        }
    }

    fn stage_cancelled(&self, _out: &mut dyn std::io::Write, stage: &str) {
        self.state.lock().unwrap().set_status(stage, HudStatus::Cancelled);
    }

    fn stage_tests(&self, _out: &mut dyn std::io::Write, stage: &str, results: &[AssertionResult]) {
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;
        let mut s = self.state.lock().unwrap();
        if let Some(&i) = s.index.get(stage) {
            s.stages[i].tests = Some((passed, failed));
        }
    }

    fn stage_finished(
        &self,
        _out: &mut dyn std::io::Write,
        stage: &str,
        _outcome: StageOutcome,
        elapsed: Duration,
    ) {
        self.state.lock().unwrap().set_elapsed(stage, elapsed);
    }

    fn stage_input_audit(
        &self,
        _out: &mut dyn std::io::Write,
        stage: &str,
        undeclared: &[std::path::PathBuf],
    ) {
        let files = undeclared.iter().map(|p| p.display().to_string()).collect();
        self.state.lock().unwrap().audit.push((stage.to_string(), files));
    }
}

/// Spinner frames cycled while a stage runs.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Run `pipeline` under the live HUD. Returns the process exit code. Computes the stage
/// order up front (surfacing any plan error before taking over the terminal), then renders
/// the run in an inline viewport while it executes on a background thread.
pub fn run_hud(
    program: &Program,
    pipeline: Option<&str>,
    ctx: &EvalContext,
    analysis: &AnalysisResult,
    jobs: Option<usize>,
    cancel: &CancelToken,
) -> i32 {
    // Resolve the stage order before touching the terminal so a plan error prints normally.
    let plan = match plan_pipeline(program, pipeline, ctx, analysis) {
        Ok(p) => p,
        Err(e) => return crate::commands::report_error(e),
    };
    let order: Vec<String> = plan.stages().map(|s| s.name.clone()).collect();
    let pipeline_label = plan.pipeline.clone();

    let state = Arc::new(Mutex::new(HudState::new(&pipeline_label, &order)));
    // Compose the HUD reporter (drives the live screen) with a `StatusRecorder` (persists
    // `.mainstage/status.json`) so a `mainstage ui` run is also recorded for `mainstage
    // status` and the VS Code status bar (Phase 54). Both receive the live per-line output.
    let recorder = StatusRecorder::new(ctx.script_dir.clone(), &pipeline_label);
    let reporter: Arc<dyn Reporter> =
        Arc::new(TeeReporter::new(HudReporter { state: state.clone() }, recorder));
    let run_ctx = ctx.with_reporter(ReporterHandle(reporter.clone()));
    let jobs =
        jobs.unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));

    let height = viewport_height(order.len());
    let mut terminal = match enter_terminal(height) {
        Ok(t) => t,
        // If the terminal can't be set up, fall back to a plain run rather than failing.
        Err(_) => {
            return run_plain(
                program,
                pipeline,
                &run_ctx,
                analysis,
                reporter.as_ref(),
                jobs,
                cancel,
            );
        }
    };

    let finished = Arc::new(AtomicBool::new(false));
    let result: Arc<Mutex<Option<mainstage_core::Result<()>>>> = Arc::new(Mutex::new(None));

    let mut tick: usize = 0;
    std::thread::scope(|scope| {
        // The pipeline runs on a background thread; the main thread renders.
        let finished_bg = finished.clone();
        let result_bg = result.clone();
        let run_ctx = &run_ctx;
        let rep = reporter.as_ref();
        scope.spawn(move || {
            let r =
                run_pipeline_cancellable(program, pipeline, run_ctx, analysis, rep, jobs, cancel);
            *result_bg.lock().unwrap() = Some(r);
            finished_bg.store(true, Ordering::SeqCst);
        });

        loop {
            // Drain input: q / Esc / Ctrl-C request cancellation (the run then winds down).
            if let Ok(true) = event::poll(Duration::from_millis(90))
                && let Ok(Event::Key(key)) = event::read()
                && key.kind == KeyEventKind::Press
            {
                let ctrl_c =
                    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                if ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    cancel.cancel();
                }
            }

            let snapshot = state.lock().unwrap().clone();
            let _ = terminal.draw(|frame| render(frame, &snapshot, tick));
            tick = tick.wrapping_add(1);

            if finished.load(Ordering::SeqCst) {
                // One last frame so the final statuses/timings are shown before teardown.
                let snapshot = state.lock().unwrap().clone();
                let _ = terminal.draw(|frame| render(frame, &snapshot, tick));
                break;
            }
        }
    });

    // Tear down the live viewport, then print a clean, permanent summary in its place.
    let _ = terminal.clear();
    let _ = disable_raw_mode();
    drop(terminal);

    let run_result = result.lock().unwrap().take().unwrap_or(Ok(()));
    let final_state = state.lock().unwrap().clone();
    print_summary(&final_state, &run_result, &analysis.dependency_graph);

    if run_result.is_ok() { 0 } else { 1 }
}

/// Set up raw mode and an inline-viewport terminal of `height` rows.
fn enter_terminal(height: u16) -> std::io::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    Terminal::with_options(backend, TerminalOptions { viewport: Viewport::Inline(height) })
}

/// Fall back to a non-interactive run (used when the terminal can't be set up). Keeps the
/// HUD reporter so output is still captured, then dumps it.
fn run_plain(
    program: &Program,
    pipeline: Option<&str>,
    ctx: &EvalContext,
    analysis: &AnalysisResult,
    reporter: &dyn Reporter,
    jobs: usize,
    cancel: &CancelToken,
) -> i32 {
    match run_pipeline_cancellable(program, pipeline, ctx, analysis, reporter, jobs, cancel) {
        Ok(()) => 0,
        Err(e) => crate::commands::report_error(e),
    }
}

/// Choose the inline viewport height: a header, a windowed stage list, a summary, and a
/// short output tail — clamped to what the terminal can show.
fn viewport_height(total: usize) -> u16 {
    let rows = size().map(|(_, r)| r as usize).unwrap_or(24);
    let avail = rows.saturating_sub(1).max(4);
    const CHROME: usize = 2; // header + summary
    const LOG: usize = 2; // output tail
    let stage_rows = total.clamp(1, avail.saturating_sub(CHROME + LOG).max(1));
    (CHROME + stage_rows + LOG).min(avail) as u16
}

/// Draw one HUD frame into the inline viewport.
fn render(frame: &mut ratatui::Frame, state: &HudState, tick: usize) {
    let area = frame.area();
    let h = area.height as usize;
    let (done, running, total) = state.counts();

    // Layout: header (1) + stages + summary (1) + output tail.
    let log_rows = h.saturating_sub(2).min(2);
    let stage_rows = h.saturating_sub(2 + log_rows);

    let mut lines: Vec<Line> = Vec::with_capacity(h);

    // Header.
    let elapsed = fmt_millis(state.started.elapsed().as_millis());
    lines.push(Line::from(vec![
        Span::styled("▶ ", Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("running {}", state.pipeline),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("   {elapsed}"), dim()),
        Span::styled("   [q] quit", dim()),
    ]));

    // Stage rows, windowed around the first unfinished stage when they don't all fit.
    for stage in window(&state.stages, stage_rows) {
        lines.push(stage_line(stage, tick, area.width));
    }

    // Summary.
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{done}/{total} done"), Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {running} running"), dim()),
    ]));

    // Output tail: the most recent captured lines.
    if log_rows > 0 {
        let start = state.scrollback.len().saturating_sub(log_rows);
        for line in &state.scrollback[start..] {
            lines.push(Line::from(Span::styled(
                format!("  │ {}", truncate(line, area.width)),
                dim(),
            )));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// The slice of stages to display: all of them when they fit, otherwise a window anchored on
/// the first unfinished stage so the action stays in view.
fn window(stages: &[HudStage], rows: usize) -> &[HudStage] {
    if rows == 0 || stages.len() <= rows {
        return stages;
    }
    let anchor =
        stages.iter().position(|s| !s.status.settled()).unwrap_or(stages.len().saturating_sub(1));
    let start = anchor.saturating_sub(1).min(stages.len() - rows);
    &stages[start..start + rows]
}

/// Render a single stage row in the Phase 54 format:
/// `[glyph] <name> (<elapsed>) <status>`, where a running row shows its live elapsed clock
/// and a tail of the last output line, a failed row shows its error, and a skipped/restored
/// row reads `cached` / `restored`.
fn stage_line(stage: &HudStage, tick: usize, width: u16) -> Line<'static> {
    let (glyph, glyph_style, status, status_style) = match stage.status {
        HudStatus::Queued => ("·".to_string(), dim(), "queued".to_string(), dim()),
        HudStatus::Running => (
            SPINNER[tick % SPINNER.len()].to_string(),
            Style::default().fg(Color::Cyan),
            "running…".to_string(),
            Style::default().fg(Color::Cyan),
        ),
        HudStatus::Passed => {
            ("✓".to_string(), Style::default().fg(Color::Green), "done".to_string(), dim())
        }
        HudStatus::Failed => (
            "✗".to_string(),
            Style::default().fg(Color::Red),
            "failed".to_string(),
            Style::default().fg(Color::Red),
        ),
        HudStatus::AllowedFailure => (
            "!".to_string(),
            Style::default().fg(Color::Yellow),
            "failed (allowed)".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        HudStatus::Skipped => ("•".to_string(), dim(), "cached".to_string(), dim()),
        HudStatus::Restored => {
            ("↻".to_string(), Style::default().fg(Color::Cyan), "restored".to_string(), dim())
        }
        HudStatus::Cancelled => (
            "⊘".to_string(),
            Style::default().fg(Color::Yellow),
            "cancelled".to_string(),
            Style::default().fg(Color::Yellow),
        ),
    };

    let mut spans = vec![
        Span::raw("  "),
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        Span::raw(format!("{:<20}", truncate(&stage.name, 20usize))),
    ];
    // Elapsed clock in parens: the final duration once settled, or the live clock while
    // running.
    if let Some(elapsed) = elapsed_of(stage) {
        spans.push(Span::styled(format!("({}) ", fmt_millis(elapsed.as_millis())), dim()));
    }
    spans.push(Span::styled(status, status_style));

    // A trailing detail after the status: the last output line while running, or the error
    // on failure. Budget the remaining row width so it never wraps the inline viewport.
    let detail = match stage.status {
        HudStatus::Running => stage.last_output.as_deref(),
        HudStatus::Failed => stage.error.as_deref(),
        _ => None,
    };
    if let Some(text) = detail {
        let one_line = text.lines().next().unwrap_or("").trim();
        if !one_line.is_empty() {
            let budget = (width as usize).saturating_sub(34).max(8);
            let st = if stage.status == HudStatus::Failed {
                Style::default().fg(Color::Red)
            } else {
                dim()
            };
            spans.push(Span::styled(format!(": {}", truncate(one_line, budget)), st));
        }
    }

    if let Some((passed, failed)) = stage.tests {
        let tests = format!("  tests: {passed} passed, {failed} failed");
        let st = if failed > 0 { Style::default().fg(Color::Red) } else { dim() };
        spans.push(Span::styled(tests, st));
    }
    Line::from(spans)
}

/// A stage's elapsed time: the recorded final duration once settled, otherwise the live
/// time since it started running (for the running row's clock). `None` before it starts.
fn elapsed_of(stage: &HudStage) -> Option<Duration> {
    stage.elapsed.or_else(|| stage.started.map(|s| s.elapsed()))
}

/// The dim style used for secondary text (timings, queued rows, output tail).
fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Truncate `s` to at most `max` display columns (best-effort, by char count), adding an
/// ellipsis when shortened.
fn truncate(s: &str, max: impl Into<usize>) -> String {
    let max = max.into();
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let take = max.saturating_sub(1);
        format!("{}…", s.chars().take(take).collect::<String>())
    }
}

/// Print the permanent summary after the live viewport is torn down: the final per-stage
/// status with timings, the critical path, and — on failure — the captured output and
/// errors. Mirrors the terminal reporter's end-of-run look so the HUD and `run` agree.
fn print_summary(
    state: &HudState,
    result: &mainstage_core::Result<()>,
    dep_graph: &HashMap<String, Vec<String>>,
) {
    println!();
    let width = state.stages.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for stage in &state.stages {
        let (glyph, status) = summary_row(stage);
        println!("  {glyph} {:<width$}  {status}", stage.name, width = width);
    }

    // Critical path, when more than one stage was timed.
    let durations: HashMap<String, Duration> =
        state.stages.iter().filter_map(|s| s.elapsed.map(|d| (s.name.clone(), d))).collect();
    let path = critical_path(dep_graph, &durations);
    if path.len() > 1 {
        let total: Duration = path.iter().filter_map(|s| durations.get(s)).sum();
        println!("\n{} ({} total)", style("critical path").bold(), fmt_millis(total.as_millis()));
        println!("  {}", path.join(&format!(" {} ", style("→").dim())));
    }

    // Audit findings (Phase 53), when present.
    for (stage, files) in &state.audit {
        println!(
            "\n{} {} read {} undeclared file(s):",
            style("audit:").yellow().bold(),
            style(stage).bold(),
            files.len()
        );
        for f in files {
            println!("  {} {f}", style("?").yellow());
        }
    }

    match result {
        Ok(()) => {
            let elapsed = fmt_millis(state.started.elapsed().as_millis());
            println!(
                "\n{} {}",
                style("✓").green().bold(),
                style(format!("pipeline '{}' succeeded in {elapsed}", state.pipeline)).green()
            );
        }
        Err(e) => {
            // Surface the captured output so a failure is debuggable, then the per-stage
            // errors (more specific than the pipeline-level message), then the conclusion.
            if !state.scrollback.is_empty() {
                println!("\n{}", style("── output ──").dim());
                for line in &state.scrollback {
                    println!("{line}");
                }
            }
            for (stage, message) in &state.errors {
                println!("\n{} {} {message}", style("✗").red().bold(), style(stage).bold());
            }
            // The core `Error` Display already carries an "error:" prefix, so don't add one.
            println!("\n{} {e}", style("✗").red().bold());
        }
    }
}

/// The glyph and status text for a stage in the permanent summary.
fn summary_row(stage: &HudStage) -> (console::StyledObject<&'static str>, String) {
    match stage.status {
        HudStatus::Passed => (
            style("✓").green(),
            stage.elapsed.map(|d| fmt_millis(d.as_millis())).unwrap_or_default(),
        ),
        HudStatus::Failed => (style("✗").red(), style("failed").red().to_string()),
        HudStatus::AllowedFailure => {
            (style("!").yellow(), style("failed (allowed)").yellow().to_string())
        }
        HudStatus::Skipped => (style("•").dim(), style("up to date").dim().to_string()),
        HudStatus::Restored => (style("↻").cyan(), style("restored from cache").dim().to_string()),
        HudStatus::Cancelled => (style("⊘").yellow(), style("cancelled").yellow().to_string()),
        HudStatus::Running | HudStatus::Queued => {
            (style("·").dim(), style("did not run").dim().to_string())
        }
    }
}

/// Format a millisecond duration compactly: `ms`, `s` (one decimal), or `m s`.
fn fmt_millis(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

// ── status command (Phase 54) ────────────────────────────────────────────────────

/// One row of the `mainstage status` table: the stage's name, where it settled, its
/// duration, and a short note (error, test tally, or last output). Pure data, so the
/// row-builder is unit-testable without a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRow {
    pub name: String,
    pub status: StageStatus,
    /// Pre-formatted elapsed label (`""` when the stage never ran).
    pub elapsed: String,
    /// A short trailing note: the error on failure, the test tally for a `test` stage, or
    /// the last output line for a still-running stage.
    pub note: String,
}

/// Build the table rows for a recorded run. Each stage maps to exactly one row, in the order
/// it was recorded (≈ execution order).
pub fn status_rows(state: &RunState) -> Vec<StatusRow> {
    state.stages.iter().map(status_row).collect()
}

fn status_row(s: &StageState) -> StatusRow {
    let elapsed = s.elapsed_ms.map(|ms| fmt_millis(ms as u128)).unwrap_or_default();
    let note = if let Some(err) = &s.error {
        err.lines().next().unwrap_or("").trim().to_string()
    } else if let Some((passed, failed)) = s.tests {
        format!("tests: {passed} passed, {failed} failed")
    } else if s.status == StageStatus::Running {
        s.last_output.clone().unwrap_or_default()
    } else {
        String::new()
    };
    StatusRow { name: s.name.clone(), status: s.status, elapsed, note }
}

/// The glyph + label + color for a stage status, shared by the TUI table and the static
/// fallback.
fn status_label(status: StageStatus) -> (&'static str, &'static str, Color) {
    match status {
        StageStatus::Queued => ("·", "queued", Color::DarkGray),
        StageStatus::Running => ("…", "running", Color::Cyan),
        StageStatus::Passed => ("✓", "passed", Color::Green),
        StageStatus::Cached => ("•", "cached", Color::DarkGray),
        StageStatus::Restored => ("↻", "restored", Color::Cyan),
        StageStatus::Failed => ("✗", "failed", Color::Red),
        StageStatus::AllowedFailure => ("!", "failed (allowed)", Color::Yellow),
        StageStatus::Cancelled => ("⊘", "cancelled", Color::Yellow),
    }
}

/// Render the last run's status. When stdout is a terminal, draw an interactive ratatui
/// table (dismissed with `q` / `Esc` / `Enter`); otherwise print a static styled table
/// (piped output / CI). Returns the process exit code (`1` when the recorded run failed).
pub fn run_status(state: &RunState) -> i32 {
    let exit = if state.status == RunStatus::Failed { 1 } else { 0 };
    if std::io::stdout().is_terminal() && draw_status_table(state).is_ok() {
        return exit;
    }
    print_status_static(state);
    exit
}

/// Draw the status table in a fullscreen alternate screen and wait for a dismiss key.
fn draw_status_table(state: &RunState) -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let _ = terminal.draw(|frame| render_status(frame, state));

    // Block until the user dismisses the view.
    loop {
        if let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            let ctrl_c =
                key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            if ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter) {
                break;
            }
        }
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Draw one frame of the status table.
fn render_status(frame: &mut ratatui::Frame, state: &RunState) {
    let header = Row::new(["", "stage", "status", "time", "note"].map(Cell::from))
        .style(Style::default().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = status_rows(state)
        .into_iter()
        .map(|r| {
            let (glyph, label, color) = status_label(r.status);
            Row::new(vec![
                Cell::from(glyph).style(Style::default().fg(color)),
                Cell::from(r.name),
                Cell::from(label).style(Style::default().fg(color)),
                Cell::from(r.elapsed).style(dim()),
                Cell::from(truncate(&r.note, frame.area().width.saturating_sub(40))).style(dim()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(24),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Percentage(40),
    ];

    let (run_glyph, run_text, run_color) = run_summary(state);
    let title = format!(" {run_glyph} {run_text}  ·  pipeline '{}' ", state.pipeline);
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(Span::styled(
            title,
            Style::default().fg(run_color).add_modifier(Modifier::BOLD),
        )))
        .column_spacing(1);

    frame.render_widget(table, frame.area());

    let hint = Line::from(Span::styled("  press q / Esc / Enter to dismiss", dim()));
    let area = frame.area();
    if area.height > 2 {
        let foot = ratatui::layout::Rect { y: area.height - 1, height: 1, ..area };
        frame.render_widget(Paragraph::new(hint), foot);
    }
}

/// Print the status table as plain styled text (no raw mode) — the piped/CI fallback.
fn print_status_static(state: &RunState) {
    let (run_glyph, run_text, _) = run_summary(state);
    let conclusion = match state.status {
        RunStatus::Succeeded => style(format!("{run_glyph} {run_text}")).green().to_string(),
        RunStatus::Failed => style(format!("{run_glyph} {run_text}")).red().to_string(),
        RunStatus::Running => style(format!("{run_glyph} {run_text}")).cyan().to_string(),
    };
    println!("{} {}  pipeline '{}'", style("last run:").bold(), conclusion, state.pipeline);

    let rows = status_rows(state);
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    for r in &rows {
        let (glyph, label, _) = status_label(r.status);
        let note = if r.note.is_empty() {
            String::new()
        } else {
            format!("  {}", style(truncate(&r.note, 60usize)).dim())
        };
        println!(
            "  {} {:<width$}  {:<16} {}{}",
            glyph,
            r.name,
            label,
            style(&r.elapsed).dim(),
            note,
            width = width,
        );
    }
}

/// The glyph, label, and color summarizing the overall run outcome.
fn run_summary(state: &RunState) -> (&'static str, &'static str, Color) {
    match state.status {
        RunStatus::Running => ("▶", "running", Color::Cyan),
        RunStatus::Succeeded => ("✓", "succeeded", Color::Green),
        RunStatus::Failed => ("✗", "failed", Color::Red),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(order: &[&str]) -> HudState {
        HudState::new("build", &order.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn counts_track_settled_and_running() {
        let mut s = state_with(&["a", "b", "c"]);
        s.set_status("a", HudStatus::Passed);
        s.set_status("b", HudStatus::Running);
        assert_eq!(s.counts(), (1, 1, 3));
        s.set_status("b", HudStatus::Failed);
        assert_eq!(s.counts(), (2, 0, 3));
    }

    #[test]
    fn window_anchors_on_first_unfinished_stage() {
        let mut s = state_with(&["a", "b", "c", "d", "e"]);
        s.set_status("a", HudStatus::Passed);
        s.set_status("b", HudStatus::Passed);
        s.set_status("c", HudStatus::Running);
        // A 2-row window should keep the running stage `c` in view.
        let shown: Vec<&str> = window(&s.stages, 2).iter().map(|x| x.name.as_str()).collect();
        assert!(shown.contains(&"c"), "running stage must be visible, got {shown:?}");
    }

    #[test]
    fn window_returns_all_when_they_fit() {
        let s = state_with(&["a", "b"]);
        assert_eq!(window(&s.stages, 5).len(), 2);
    }

    #[test]
    fn truncate_adds_ellipsis_when_long() {
        assert_eq!(truncate("hello", 10u16), "hello");
        assert_eq!(truncate("hello world", 5u16), "hell…");
        assert_eq!(truncate("x", 0u16), "");
    }

    fn stage(name: &str, status: StageStatus) -> StageState {
        StageState {
            name: name.to_string(),
            status,
            started_unix_ms: None,
            elapsed_ms: None,
            last_output: None,
            error: None,
            tests: None,
        }
    }

    #[test]
    fn status_rows_map_status_elapsed_and_notes() {
        let mut compile = stage("compile", StageStatus::Passed);
        compile.elapsed_ms = Some(1_500);
        let mut test = stage("test", StageStatus::Failed);
        test.elapsed_ms = Some(42);
        test.error = Some("assertion failed\nsecond line".to_string());
        let mut lint = stage("lint", StageStatus::Running);
        lint.last_output = Some("checking crate…".to_string());
        let cached = stage("assets", StageStatus::Cached);
        let mut suite = stage("suite", StageStatus::Passed);
        suite.tests = Some((3, 1));

        let state = RunState {
            pipeline: "release".to_string(),
            started_unix_ms: 0,
            status: RunStatus::Failed,
            stages: vec![compile, test, lint, cached, suite],
        };
        let rows = status_rows(&state);

        assert_eq!(rows[0].elapsed, "1.5s");
        assert_eq!(rows[0].note, "");
        // The error note is collapsed to its first line.
        assert_eq!(rows[1].note, "assertion failed");
        assert_eq!(rows[1].elapsed, "42ms");
        // A running stage surfaces its last output line.
        assert_eq!(rows[2].note, "checking crate…");
        // A cache hit has no note and no elapsed.
        assert_eq!(rows[3].status, StageStatus::Cached);
        assert_eq!(rows[3].note, "");
        // A test stage shows its tally even when passed.
        assert_eq!(rows[4].note, "tests: 3 passed, 1 failed");
    }

    #[test]
    fn run_status_exit_code_reflects_outcome() {
        // Non-TTY in tests, so this exercises the static path and the exit code.
        let ok = RunState {
            pipeline: "dev".to_string(),
            started_unix_ms: 0,
            status: RunStatus::Succeeded,
            stages: vec![stage("a", StageStatus::Passed)],
        };
        assert_eq!(run_status(&ok), 0);
        let bad = RunState { status: RunStatus::Failed, ..ok };
        assert_eq!(run_status(&bad), 1);
    }
}
