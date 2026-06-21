import { test } from "node:test";
import assert from "node:assert/strict";

import { fmtMillis, formatStatusBar, parseRunState, RunState } from "../runStatus";

// ── parseRunState ────────────────────────────────────────────────────────────────

test("parseRunState rejects malformed or partial JSON", () => {
  assert.equal(parseRunState("{ not json"), undefined);
  assert.equal(parseRunState("null"), undefined);
  assert.equal(parseRunState("[]"), undefined);
  // Missing required keys (e.g. caught mid-write) is rejected.
  assert.equal(parseRunState(JSON.stringify({ pipeline: "x" })), undefined);
});

test("parseRunState accepts a well-formed run state", () => {
  const json = JSON.stringify({ pipeline: "dev", started_unix_ms: 0, status: "running", stages: [] });
  const state = parseRunState(json);
  assert.equal(state?.pipeline, "dev");
  assert.equal(state?.status, "running");
});

// ── fmtMillis ────────────────────────────────────────────────────────────────────

test("fmtMillis formats compactly", () => {
  assert.equal(fmtMillis(250), "250ms");
  assert.equal(fmtMillis(1500), "1.5s");
  assert.equal(fmtMillis(65_000), "1m 5s");
});

// ── formatStatusBar ──────────────────────────────────────────────────────────────

test("a running stage shows a spinner, elapsed time, and last output", () => {
  const state: RunState = {
    pipeline: "release",
    started_unix_ms: 1000,
    status: "running",
    stages: [
      { name: "compile", status: "passed", elapsed_ms: 120 },
      { name: "test", status: "running", started_unix_ms: 1000, last_output: "running 42 tests" },
    ],
  };
  const view = formatStatusBar(state, 3500);
  assert.equal(view?.text, "$(sync~spin) test (2.5s) : running 42 tests");
});

test("a running pipeline with no active stage falls back to the pipeline name", () => {
  const state: RunState = {
    pipeline: "dev",
    started_unix_ms: 0,
    status: "running",
    stages: [{ name: "a", status: "queued" }],
  };
  assert.equal(formatStatusBar(state, 0)?.text, "$(sync~spin) dev…");
});

test("a succeeded run shows a check and the pipeline name", () => {
  const state: RunState = {
    pipeline: "dev",
    started_unix_ms: 0,
    status: "succeeded",
    stages: [{ name: "a", status: "passed" }],
  };
  assert.equal(formatStatusBar(state, 0)?.text, "$(check) dev succeeded");
});

test("a failed run names the failing stage", () => {
  const state: RunState = {
    pipeline: "ci",
    started_unix_ms: 0,
    status: "failed",
    stages: [
      { name: "build", status: "passed" },
      { name: "test", status: "failed", error: "boom" },
    ],
  };
  const view = formatStatusBar(state, 0);
  assert.equal(view?.text, "$(error) ci failed: test");
  assert.match(view?.tooltip ?? "", /boom/);
});
