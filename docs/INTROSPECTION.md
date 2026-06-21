# Build Graph Query & Explain

Mainstage can make its dependency graph and its change-detection decisions
inspectable, so you can see *why* a stage ran, what depends on it, and where the
time went. Three commands cover this, all reading the same analysis and cache the
runner uses — so what they report always matches what a real run does:

- **`mainstage query`** — print the stage dependency graph and its reverse edges.
- **`mainstage explain <stage>`** — say why a stage would run or be skipped.
- **`mainstage profile`** (or the `--profile` flag) — per-stage timings and the
  critical path.
- **`mainstage status`** — show the **last** run's per-stage statuses and timings.

These are introspection only: `query`, `explain`, and `status` execute no steps and
never write the cache. For the language itself see [`GRAMMAR.md`](GRAMMAR.md).

---

## Table of Contents

1. [`mainstage query`](#mainstage-query)
2. [`mainstage explain`](#mainstage-explain)
3. [`mainstage profile`](#mainstage-profile)
4. [`mainstage status`](#mainstage-status)

---

## `mainstage query`

Prints the stage dependency graph: for each stage, the stages it **depends on**
(its inputs/outputs and `depends_on` edges) and the stages that **depend on it**
(its reverse edges). Nodes are listed in topological order, so a dependency always
appears before the stages that need it.

```console
$ mainstage query
dependency graph: all stages
gen
  depends on (none)
  required by a, b
a
  depends on gen
  required by combine
b
  depends on gen
  required by combine
combine
  depends on a, b
```

### Filtering by pipeline

By default `query` spans every declared stage. Pass `--pipeline <name>` to restrict
the graph to one pipeline's members; edges to stages outside that pipeline are
dropped, so the view matches what that pipeline would actually run.

```console
$ mainstage query --pipeline release
```

### Export formats

`--format` selects the output form for external tooling:

| Value          | Output                                                         |
|----------------|----------------------------------------------------------------|
| `text` (default) | The indented, human-readable listing above.                 |
| `dot`          | A Graphviz DOT digraph. Edges point in execution order (`dependency -> stage`). Pipe to Graphviz: `mainstage query --format dot \| dot -Tpng -o graph.png`. |
| `json`         | A JSON document with a `pipeline` field and a `nodes` array (each node has `name`, `depends_on`, and `dependents`), for scripts and dashboards. |

```console
$ mainstage query --format dot
digraph mainstage {
  rankdir=LR;
  node [shape=box];
  "gen";
  "gen" -> "a";
  "gen" -> "b";
  ...
}
```

---

## `mainstage explain`

Explains why a single stage would run or be skipped on its next invocation, by
replaying the runner's change-detection decision against the current cache and the
current state of the tree. The verdict is one of:

| Verdict | Meaning |
|---------|---------|
| **would run — no prior successful run is recorded** | The stage has never completed successfully (nothing cached yet). |
| **would run — inputs changed since the last run** | One or more input files changed. The changed files are listed. |
| **would run — never cached (`<reason>`)** | The stage is `always_run`, a `test` stage, or has no declared `inputs` — the Phase 7 "always runs" default. |
| **would skip — inputs unchanged and outputs present** | A local cache hit: nothing to do. |
| **would skip — inputs unchanged; outputs would be restored from the cache** | Inputs match, but some declared outputs are missing; they would be restored from the content-addressed output store rather than rebuilt. |

The explanation also reports, when relevant:

- **changed inputs** — the specific input files whose content changed.
- **missing outputs** — declared outputs not currently present in the tree.
- **incremental note** — when a rerun would be *per-output* rather than
  whole-stage: a prior run exists, all outputs are present, only a subset of
  inputs changed, **and** the stage iterates its inputs with `for file in inputs`
  (so the unchanged iterations are skipped).
- **depends on / required by** — the stage's direct dependencies and dependents.

```console
$ mainstage explain a
▶ a would run — inputs changed since the last run
  note: only the changed inputs' outputs would be rebuilt (incremental)
  changed inputs:
    src/main.rs
  depends on: gen
  required by: combine
```

---

## `mainstage profile`

Runs a pipeline (like `mainstage run`) and, when it finishes, prints the per-stage
timing summary followed by the **critical path** — the chain of stages with the
greatest cumulative duration, i.e. the longest sequential bottleneck through the
dependency graph. Shortening the critical path is what actually makes a parallel
build faster.

```console
$ mainstage profile release
...
timing summary
  ✓ gen      12ms
  ✓ a        450ms
  ✓ b        80ms
  ✓ combine  30ms

critical path (492ms total)
  gen → a → combine
```

The same breakdown can be appended to any run with the global `--profile` flag
(`mainstage --profile`, `mainstage --profile run release`). Both honor `--quiet`
(which suppresses the summary) and `--jobs` (which changes the timings but not the
graph). Only stages that actually ran this invocation are considered, so a partial
run still profiles cleanly.

---

## `mainstage status`

Shows the **last** run's per-stage statuses and timings — including which stages were
cache hits or restored from the output store — without re-running anything. It reads a
run-state file the runner writes as it goes, `.mainstage/status.json`, so it reflects
whatever the most recent `mainstage run`, `mainstage ui`, or `mainstage profile` did.

On a terminal it opens an interactive table (dismiss with `q`, `Esc`, or `Enter`); when
its output is piped or redirected (CI, `| cat`) it prints the same table as plain styled
text instead:

```console
$ mainstage status
last run: ✓ succeeded  pipeline 'release'
  ✓ compile  passed     1.5s
  • assets   cached
  ↻ docs     restored
  ✓ test     passed     420ms  tests: 12 passed, 0 failed
```

Per-stage status is one of `passed`, `cached` (skipped on a cache hit), `restored`
(missing outputs restored from the content-addressed store), `failed` (with the error),
`failed (allowed)`, `cancelled`, or — for a live file written by a run still in flight —
`running` (with its latest output line). If no run has been recorded yet, `status` says so.

### The run-state file

`.mainstage/status.json` is a small JSON document updated atomically during a run (it sits
next to the change-detection cache, so `mainstage clean` removes it). Its shape:

```json
{
  "pipeline": "release",
  "started_unix_ms": 1717000000000,
  "status": "running",
  "stages": [
    { "name": "compile", "status": "running",
      "started_unix_ms": 1717000000020, "last_output": "Compiling app v0.1.0" }
  ]
}
```

This is the same file the VS Code extension watches to show the running stage in its
status bar (see [`TOOLING.md`](TOOLING.md)).
