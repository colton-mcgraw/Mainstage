/**
 * Phase 54 — live run status in the status bar.
 *
 * The Mainstage CLI writes a run-state file at `<project>/.mainstage/status.json` as a
 * pipeline runs. This module owns the *pure* part of surfacing it: parsing that JSON and
 * formatting a one-line status-bar label. The wiring (the watcher and the `StatusBarItem`)
 * lives in {@link ../extension}. Keeping the formatting pure makes it unit-testable without
 * a VS Code host (see `src/test/runStatus.test.ts`).
 */

/** Overall outcome of a run, mirroring the CLI's `RunStatus`. */
export type RunStatus = "running" | "succeeded" | "failed";

/** Where a single stage stands, mirroring the CLI's `StageStatus`. */
export type StageStatus =
  | "queued"
  | "running"
  | "passed"
  | "cached"
  | "restored"
  | "failed"
  | "allowed_failure"
  | "cancelled";

export interface StageState {
  name: string;
  status: StageStatus;
  started_unix_ms?: number;
  elapsed_ms?: number;
  last_output?: string;
  error?: string;
}

export interface RunState {
  pipeline: string;
  started_unix_ms: number;
  status: RunStatus;
  stages: StageState[];
}

/** A rendered status-bar entry. */
export interface StatusBarView {
  /** The label shown in the status bar (may contain `$(icon)` codicons). */
  text: string;
  /** A longer hover tooltip. */
  tooltip: string;
}

/**
 * Parse the run-state JSON defensively. Returns `undefined` for anything that does not look
 * like a `RunState` — including a partially written file caught mid-rename — so a malformed
 * read is simply ignored until the next watch event.
 */
export function parseRunState(text: string): RunState | undefined {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const v = value as Record<string, unknown>;
  if (typeof v.pipeline !== "string" || typeof v.status !== "string" || !Array.isArray(v.stages)) {
    return undefined;
  }
  return value as RunState;
}

/** Format a millisecond duration compactly: `ms`, `s` (one decimal), or `m s`. */
export function fmtMillis(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)}ms`;
  }
  if (ms < 60_000) {
    return `${(ms / 1000).toFixed(1)}s`;
  }
  const secs = Math.floor(ms / 1000);
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

/** Truncate `s` to at most `max` characters, adding an ellipsis when shortened. */
function truncate(s: string, max: number): string {
  return s.length <= max ? s : `${s.slice(0, Math.max(0, max - 1))}…`;
}

/**
 * Build the status-bar view for a run, or `undefined` when there is nothing to show.
 *
 * - While running, the currently executing stage is shown with its live elapsed time and
 *   the last line of its output: `$(sync~spin) <stage> (<elapsed>) : <last line>`.
 * - A settled run shows a terminal summary: `$(check) <pipeline> succeeded` /
 *   `$(error) <pipeline> failed: <stage>`.
 *
 * `nowMs` is injected (rather than read from the clock) so the running elapsed time is
 * deterministic in tests.
 */
export function formatStatusBar(state: RunState, nowMs: number): StatusBarView | undefined {
  if (state.status === "running") {
    const running = state.stages.find((s) => s.status === "running");
    if (!running) {
      return {
        text: `$(sync~spin) ${state.pipeline}…`,
        tooltip: `Mainstage: running pipeline '${state.pipeline}'`,
      };
    }
    const elapsed = running.started_unix_ms
      ? ` (${fmtMillis(Math.max(0, nowMs - running.started_unix_ms))})`
      : "";
    const tail = running.last_output ? ` : ${truncate(running.last_output, 40)}` : "";
    return {
      text: `$(sync~spin) ${running.name}${elapsed}${tail}`,
      tooltip:
        `Mainstage: running '${running.name}' in pipeline '${state.pipeline}'` +
        (running.last_output ? `\n${running.last_output}` : ""),
    };
  }

  if (state.status === "succeeded") {
    return {
      text: `$(check) ${state.pipeline} succeeded`,
      tooltip: `Mainstage: pipeline '${state.pipeline}' succeeded`,
    };
  }

  // failed
  const failed = state.stages.find(
    (s) => s.status === "failed" || s.status === "cancelled",
  );
  const where = failed ? `: ${failed.name}` : "";
  return {
    text: `$(error) ${state.pipeline} failed${where}`,
    tooltip:
      `Mainstage: pipeline '${state.pipeline}' failed` +
      (failed?.error ? `\n${failed.name}: ${failed.error}` : ""),
  };
}
