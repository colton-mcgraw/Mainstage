//! Phase 54 — Run-state persistence.
//!
//! A live, machine-readable record of a pipeline run, written to
//! `.mainstage/status.json` as the run progresses. It is the single backbone for the
//! phase's three frontends:
//!
//! - the `mainstage status` command renders the **last** run's table from it;
//! - the VS Code extension *watches* the file and surfaces the running stage in its
//!   status bar;
//! - the live HUD keeps its own in-memory state, but composing it with [`StatusRecorder`]
//!   (via [`TeeReporter`](crate::runner::TeeReporter)) means a `mainstage ui` run records
//!   state too.
//!
//! [`StatusRecorder`] is a [`Reporter`](crate::runner::Reporter): it turns lifecycle events
//! into a [`RunState`] and persists it atomically (a temp file renamed over the target, like
//! [`Cache::save`](crate::cache::Cache::save)) so a reader never sees a half-written file.
//! Frequent events (per-line output) are throttled; status transitions are flushed
//! immediately so a watcher reacts without lag.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cache::CACHE_DIR;
use crate::error::{Diagnostic, Error, Result};
use crate::runner::{Reporter, StageOutcome};

/// Run-state file name within [`CACHE_DIR`](crate::cache::CACHE_DIR).
pub const STATUS_FILE: &str = "status.json";

/// Minimum wall-clock gap between throttled persists (per-line output). Status transitions
/// bypass this and write immediately.
const THROTTLE: Duration = Duration::from_millis(100);

/// Overall outcome of a run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run is still in progress.
    Running,
    /// Every stage settled successfully.
    Succeeded,
    /// At least one (non-tolerated) stage failed.
    Failed,
}

/// Where a single stage stands, mirroring the runner's lifecycle events and the HUD's view.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    /// Not yet started (only seen when a frontend pre-seeds the stage list).
    Queued,
    /// Currently executing its steps.
    Running,
    /// Ran its steps successfully.
    Passed,
    /// Skipped because its inputs were unchanged and outputs present (a cache hit).
    Cached,
    /// Skipped, with its missing outputs restored from the content-addressed store.
    Restored,
    /// Its steps failed and the failure was not tolerated.
    Failed,
    /// Its steps failed but `allow_failure` tolerated it.
    AllowedFailure,
    /// Cancelled because a dependency failed (or the run was interrupted).
    Cancelled,
}

impl StageStatus {
    /// Map a runner [`StageOutcome`] (the settle event) to a status. `Passed` is left to the
    /// dedicated `stage_passed` event; the others come through `stage_finished`.
    fn from_outcome(outcome: StageOutcome) -> Self {
        match outcome {
            StageOutcome::Passed => StageStatus::Passed,
            StageOutcome::Skipped => StageStatus::Cached,
            StageOutcome::Restored => StageStatus::Restored,
            StageOutcome::Failed => StageStatus::Failed,
        }
    }

    /// Whether the stage has reached a terminal state.
    pub fn settled(self) -> bool {
        !matches!(self, StageStatus::Queued | StageStatus::Running)
    }
}

/// One stage's recorded state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageState {
    /// Stage name.
    pub name: String,
    /// Where the stage stands.
    pub status: StageStatus,
    /// Unix epoch millis when the stage started running, if it did.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub started_unix_ms: Option<u64>,
    /// Wall-clock duration once settled, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub elapsed_ms: Option<u64>,
    /// The most recent line of command output observed while running.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_output: Option<String>,
    /// On failure, the error message.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
    /// For a `test` stage, the `(passed, failed)` assertion tally.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tests: Option<(usize, usize)>,
}

impl StageState {
    fn new(name: &str, status: StageStatus) -> Self {
        Self {
            name: name.to_string(),
            status,
            started_unix_ms: None,
            elapsed_ms: None,
            last_output: None,
            error: None,
            tests: None,
        }
    }
}

/// The full record of a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunState {
    /// The pipeline that was run.
    pub pipeline: String,
    /// Unix epoch millis when the run started.
    pub started_unix_ms: u64,
    /// Overall outcome.
    pub status: RunStatus,
    /// Per-stage state, in the order stages were first observed (≈ execution order).
    pub stages: Vec<StageState>,
}

impl RunState {
    fn new(pipeline: &str) -> Self {
        Self {
            pipeline: pipeline.to_string(),
            started_unix_ms: now_unix_ms(),
            status: RunStatus::Running,
            stages: Vec::new(),
        }
    }

    /// Load the last run's recorded state from `<project_dir>/.mainstage/status.json`.
    /// Returns `None` when no (readable, well-formed) record exists.
    pub fn load(project_dir: &Path) -> Option<Self> {
        let path = status_path(project_dir);
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Find a stage by name, creating it (appended, in first-seen order) if absent.
    fn entry(&mut self, name: &str, default: StageStatus) -> &mut StageState {
        if let Some(i) = self.stages.iter().position(|s| s.name == name) {
            &mut self.stages[i]
        } else {
            self.stages.push(StageState::new(name, default));
            self.stages.last_mut().unwrap()
        }
    }
}

/// Absolute path to the run-state file for the project rooted at `project_dir`.
pub fn status_path(project_dir: &Path) -> PathBuf {
    project_dir.join(CACHE_DIR).join(STATUS_FILE)
}

/// A [`Reporter`] that records a run into `.mainstage/status.json`.
///
/// It owns the [`RunState`] behind a mutex (so it is `Sync` and shareable across worker
/// threads) and persists after each event — immediately for status transitions, throttled
/// for the high-frequency per-line output so the file is not rewritten on every line.
pub struct StatusRecorder {
    project_dir: PathBuf,
    state: Mutex<RunState>,
    /// Last time the file was written, to throttle per-line persists.
    last_write: Mutex<Instant>,
}

impl StatusRecorder {
    /// A recorder for `pipeline`, writing under `project_dir`.
    pub fn new(project_dir: PathBuf, pipeline: &str) -> Self {
        Self {
            project_dir,
            state: Mutex::new(RunState::new(pipeline)),
            // Far enough in the past that the first throttled write always lands.
            last_write: Mutex::new(Instant::now() - THROTTLE),
        }
    }

    /// A snapshot of the current run state (used by tests).
    pub fn snapshot(&self) -> RunState {
        self.state.lock().unwrap().clone()
    }

    /// Persist the current state. `force` bypasses the throttle (used for status
    /// transitions); otherwise the write is skipped if one happened within [`THROTTLE`].
    /// Failures are swallowed: a missing status file degrades the UI but never the build.
    fn persist(&self, force: bool) {
        if !force {
            let mut last = self.last_write.lock().unwrap();
            if last.elapsed() < THROTTLE {
                return;
            }
            *last = Instant::now();
        } else {
            *self.last_write.lock().unwrap() = Instant::now();
        }
        let snapshot = self.state.lock().unwrap().clone();
        let _ = write_atomic(&self.project_dir, &snapshot);
    }
}

impl Reporter for StatusRecorder {
    fn stage_start(&self, _out: &mut dyn Write, stage: &str) {
        {
            let mut s = self.state.lock().unwrap();
            let e = s.entry(stage, StageStatus::Running);
            e.status = StageStatus::Running;
            e.started_unix_ms = Some(now_unix_ms());
        }
        self.persist(true);
    }

    fn stage_output_line(&self, stage: &str, line: &str) {
        if line.is_empty() {
            return;
        }
        {
            let mut s = self.state.lock().unwrap();
            s.entry(stage, StageStatus::Running).last_output = Some(line.to_string());
        }
        // High-frequency event: throttle so the file is not rewritten per line.
        self.persist(false);
    }

    fn stage_skipped(&self, _out: &mut dyn Write, stage: &str) {
        self.state.lock().unwrap().entry(stage, StageStatus::Cached).status = StageStatus::Cached;
        self.persist(true);
    }

    fn stage_restored(&self, _out: &mut dyn Write, stage: &str) {
        self.state.lock().unwrap().entry(stage, StageStatus::Restored).status =
            StageStatus::Restored;
        self.persist(true);
    }

    fn stage_passed(&self, _out: &mut dyn Write, stage: &str) {
        self.state.lock().unwrap().entry(stage, StageStatus::Passed).status = StageStatus::Passed;
        self.persist(true);
    }

    fn stage_failed(&self, _out: &mut dyn Write, stage: &str, error: &Error, allow_failure: bool) {
        {
            let mut s = self.state.lock().unwrap();
            let status =
                if allow_failure { StageStatus::AllowedFailure } else { StageStatus::Failed };
            let e = s.entry(stage, status);
            e.status = status;
            e.error = Some(error.to_string());
        }
        self.persist(true);
    }

    fn stage_cancelled(&self, _out: &mut dyn Write, stage: &str) {
        self.state.lock().unwrap().entry(stage, StageStatus::Cancelled).status =
            StageStatus::Cancelled;
        self.persist(true);
    }

    fn stage_tests(&self, _out: &mut dyn Write, stage: &str, results: &[crate::AssertionResult]) {
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;
        self.state.lock().unwrap().entry(stage, StageStatus::Running).tests =
            Some((passed, failed));
        self.persist(true);
    }

    fn stage_finished(
        &self,
        _out: &mut dyn Write,
        stage: &str,
        outcome: StageOutcome,
        elapsed: Duration,
    ) {
        {
            let mut s = self.state.lock().unwrap();
            let mapped = StageStatus::from_outcome(outcome);
            let e = s.entry(stage, mapped);
            e.elapsed_ms = Some(elapsed.as_millis() as u64);
            // Don't clobber a more specific status already set (e.g. AllowedFailure, or a
            // Cached/Restored skip): only fill in when still Running/Queued.
            if !e.status.settled() {
                e.status = mapped;
            }
        }
        self.persist(true);
    }

    fn pipeline_finished(&self, _out: &mut dyn Write, _pipeline: &str, failed_stage: Option<&str>) {
        {
            let mut s = self.state.lock().unwrap();
            s.status =
                if failed_stage.is_some() { RunStatus::Failed } else { RunStatus::Succeeded };
        }
        self.persist(true);
    }

    // Swallow output blocks: the recorder captures live output via `stage_output_line`, and
    // must never write to stdout itself — doing so would double-print under the terminal
    // reporter or corrupt the live HUD's screen when composed via `TeeReporter`.
    fn flush_block(&self, _bytes: &[u8]) {}
}

/// Atomically write `state` to `<project_dir>/.mainstage/status.json`, creating the
/// directory if needed. Mirrors [`Cache::save`](crate::cache::Cache::save): write to a
/// uniquely-named temp file, then rename over the target, so a watcher never reads a
/// truncated file.
fn write_atomic(project_dir: &Path, state: &RunState) -> Result<()> {
    let dir = project_dir.join(CACHE_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| status_err(format!("create state dir '{}': {}", dir.display(), e)))?;
    let path = dir.join(STATUS_FILE);
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| status_err(format!("serialize run state: {}", e)))?;

    let tmp = dir.join(format!("{}.{}.tmp", STATUS_FILE, uuid::Uuid::new_v4()));
    std::fs::write(&tmp, text)
        .map_err(|e| status_err(format!("write run state '{}': {}", tmp.display(), e)))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        status_err(format!("replace run state '{}': {}", path.display(), e))
    })
}

/// Current wall-clock time as Unix epoch milliseconds (0 if the clock predates the epoch).
fn now_unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn status_err(msg: impl Into<String>) -> Error {
    Error::Eval(vec![Diagnostic::new(msg)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Diagnostic;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("mainstage-status-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_and_round_trips_a_run() {
        let dir = temp_dir("roundtrip");
        let rec = StatusRecorder::new(dir.clone(), "build");
        let mut out: Vec<u8> = Vec::new();

        rec.stage_start(&mut out, "compile");
        rec.stage_output_line("compile", "linking…");
        rec.stage_finished(&mut out, "compile", StageOutcome::Passed, Duration::from_millis(120));
        rec.stage_passed(&mut out, "compile");
        rec.stage_skipped(&mut out, "assets");
        rec.stage_finished(&mut out, "assets", StageOutcome::Skipped, Duration::from_millis(2));
        rec.pipeline_finished(&mut out, "build", None);

        let loaded = RunState::load(&dir).expect("status file written");
        assert_eq!(loaded.pipeline, "build");
        assert_eq!(loaded.status, RunStatus::Succeeded);
        assert_eq!(loaded.stages.len(), 2);

        let compile = &loaded.stages[0];
        assert_eq!(compile.name, "compile");
        assert_eq!(compile.status, StageStatus::Passed);
        assert_eq!(compile.elapsed_ms, Some(120));
        assert_eq!(compile.last_output.as_deref(), Some("linking…"));

        let assets = &loaded.stages[1];
        assert_eq!(assets.status, StageStatus::Cached);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_records_error_and_run_status() {
        let dir = temp_dir("failure");
        let rec = StatusRecorder::new(dir.clone(), "build");
        let mut out: Vec<u8> = Vec::new();

        rec.stage_start(&mut out, "test");
        let err = Error::Eval(vec![Diagnostic::new("boom")]);
        rec.stage_failed(&mut out, "test", &err, false);
        rec.stage_finished(&mut out, "test", StageOutcome::Failed, Duration::from_millis(5));
        rec.pipeline_finished(&mut out, "build", Some("test"));

        let loaded = RunState::load(&dir).unwrap();
        assert_eq!(loaded.status, RunStatus::Failed);
        assert_eq!(loaded.stages[0].status, StageStatus::Failed);
        assert!(loaded.stages[0].error.as_deref().unwrap().contains("boom"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finished_does_not_clobber_allowed_failure() {
        let dir = temp_dir("allowed");
        let rec = StatusRecorder::new(dir.clone(), "build");
        let mut out: Vec<u8> = Vec::new();

        let err = Error::Eval(vec![Diagnostic::new("nope")]);
        rec.stage_failed(&mut out, "lint", &err, true);
        rec.stage_finished(&mut out, "lint", StageOutcome::Failed, Duration::from_millis(3));

        let loaded = RunState::load(&dir).unwrap();
        assert_eq!(loaded.stages[0].status, StageStatus::AllowedFailure);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
