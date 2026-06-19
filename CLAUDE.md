# CLAUDE.md — orientation for AI agents

Fast-start notes for working in this repo. Read [`CONTRIBUTING.md`](CONTRIBUTING.md)
for the canonical workspace layout, build/test commands, and CI gates — this file
covers the **mental model, the change recipes, and the gotchas** that aren't obvious
from a first read.

## What Mainstage is

A build-orchestration language (`.ms` files) with its own interpreter. A script declares
`project`, `let`, `import`, `stage`, and `pipeline` items; the runtime builds a stage
dependency DAG and runs it with change detection and parallelism. Think "Make + a real
language", implemented in Rust.

## Architecture in one picture

Source text flows through a fixed pipeline of passes. **Put language logic in `core`** —
the CLI and LSP are thin front ends that both call the same `core` entry points, so
behavior never diverges between command line and editor.

```
.ms source
  └─ parser.rs (+ grammar.pest, pest PEG)  → ast.rs (typed AST, every node has a Span)
       └─ matrix.rs::expand  (lowers `matrix {}` stages into concrete variants)
            └─ sema.rs::analyze_with(registry) → AnalysisResult { dependency_graph }
                 └─ eval.rs::eval_program_with(registry) → EvalContext (lets, project, …)
                      └─ runner.rs (schedules the DAG; parallel by readiness)
                           └─ executor.rs (runs each step: $, copy, write, expect, …)
                                └─ modules/ (env, git, str, path, fs, json, shell, …)
```

Cross-cutting:
- **`cache.rs`** — change detection (`.mainstage/cache.json`): per-file mtime/size fast
  path + SHA-256, whole-stage and per-output incremental.
- **`format.rs` + `trivia.rs`** — `mainstage format`. `trivia.rs` re-attaches comments
  (the parser discards them) so formatting round-trips them losslessly.
- **`error.rs`** — `Error::{Parse,Semantic,Eval,Io}`, each carrying `Vec<Diagnostic>`
  with `Span`s. Diagnostics accumulate; passes don't abort on the first error.

The single source of truth is `cli/src/commands.rs::prepare()` — it does
`parse → expand_matrix → analyze_with → eval_program_with`, sharing **one**
`ModuleRegistry` between analysis and evaluation. Mirror this order anywhere you drive
the language (the tests in `core/tests/runner.rs` show the minimal version).

## The workflow this repo runs on

Development is **phase-driven** by [`docs/ROADMAP.md`](docs/ROADMAP.md). Each phase is a
checklist; "implement the next phase" means find the first phase with unchecked `[ ]`
boxes, implement every item, then flip them to `[x]`. Commit messages reference the
phase (e.g. "(Phase 39)"). Match the surrounding code's heavy comment density and the
existing idioms — this codebase documents the *why* on almost every block.

## Recipe: adding a language construct (step, stage field, expression)

A new surface-syntax feature touches the same files in the same order almost every time.
Using "add a step" as the template (Phase 39's `expect`/`assert` and Phase 36's `try`
are worked examples to copy from):

1. **`core/src/grammar.pest`** — add the rule; register it in the `step` (or
   `stage_field`) alternation. Order matters in a PEG: put more-specific alternatives
   first. New contextual keywords usually need **no** change to the `keyword` rule
   (don't reserve words unless they must be rejected as identifiers).
2. **`core/src/ast.rs`** — add the AST node/enum variant and a `span` arm. Add any new
   field to `StageBlock`/etc.
3. **`core/src/parser.rs`** — a `build_*` method walking the pest pairs. Silent rules
   (`_{…}`) pass their inner concrete rules straight through (see how `step`/`item` are
   matched by `as_rule()`). Add the variant to the `build_step`/`build_stage` dispatch.
4. **`core/src/sema.rs`** — resolve names in any sub-expressions (`resolve_step` arm) and
   add semantic validation (mutual-exclusion checks, type checks via `infer_type`).
5. **`core/src/eval.rs`** / **`core/src/executor.rs`** — runtime behavior. Steps live in
   `executor.rs::execute_step`; expressions in `eval.rs`. **If you add an `EvalContext`
   field, update every literal**: `eval_program_with`, `clone_base`, and the test
   helpers in `eval.rs` and `executor.rs` (the compiler will point you at them).
6. **`core/src/runner.rs`** — only if the feature affects scheduling, caching, or
   reporting (e.g. a new stage flavor → `change_detection_inputs`, a new `Reporter`
   trait method).
7. **`core/src/format.rs`** — render the new node (matches on AST exhaustively, **will
   not compile until you add the arm**). Keep output canonical and idempotent.
8. **`lsp/src/navigation.rs`** (`walk_steps` is an exhaustive match — add the arm),
   and `index.rs` / `hover.rs` / `completion.rs` if the feature is editor-visible.
9. **`core/src/lib.rs`** — re-export any new public types.
10. **`cli/src/commands.rs`** — new flags/output (e.g. a `Reporter` impl method).
11. **Docs**: `docs/GRAMMAR.md` (field tables + the EBNF block at the bottom),
    `docs/MODULES.md` for modules, `docs/TOOLING.md` for LSP features.
12. **Tests**: a `#[test]` next to the code + an integration test in `core/tests/`, and
    often a committed `.ms` under `tests/` or `examples/`.

**Exhaustive matches are your safety net** — `Step`, `Item`, `Expr` are matched without
`_ =>` in `format.rs`, `navigation.rs`, and `ast.rs::span()`, so a missing case is a
compile error, not a silent bug. Lean on `cargo build` to find every site.

### Recipe: adding a module / module method

Edit the module in `core/src/modules/builtin/<name>.rs`: add a `MethodSig` to its
`METHODS` (params, named params, `returns: ValueTy`) and a match arm in `call`. Helpers
(`require_positional_string`, `require_positional_list`, `named_string`, `resolve_path`)
live in `core/src/modules/mod.rs`. `sema.rs` validates calls against `MethodSig`
automatically and `mainstage modules` lists them — no extra wiring. A brand-new module is
registered in `ModuleRegistry::standard()` (`modules/mod.rs`), re-exported from
`modules/builtin/mod.rs`, and (if user-facing) documented in `docs/MODULES.md`.

## Invariants & gotchas

- **Every CI gate must pass** (see CONTRIBUTING.md): `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings` (**warnings fail the build**), tests, and
  `mainstage format --check` over every committed `.ms` script. After editing any `.ms`
  file (including `main.ms` and `tests/*.ms`), run `mainstage format <file>` and, if you
  add a new committed script, add it to the format list in `.github/workflows/ci.yml`
  **and** the fixture lists in `core/tests/format.rs` / `trivia.rs`.
- **Two formatters, don't confuse them**: `cargo fmt` (Rust) vs `mainstage format`
  (`.ms`). `clippy::type_complexity` is on — factor gnarly tuple types into a `type` alias.
- **Spans are 1-based** `(line, col)` start/end in `core`; the LSP converts to 0-based
  UTF-16 in `lsp/src/convert.rs`.
- **Reporter trait** (`runner.rs`): new methods need a default no-op body so existing
  impls (`NoopReporter`, the CLI's `TermReporter`, test reporters) keep compiling.
- **Spawning processes with capture/timeout**: see `executor.rs::run_capture` — after a
  kill, do **not** join reader threads (an orphaned grandchild can hold the pipe open
  forever); snapshot the buffer and detach.
- **Tests use temp dirs** keyed by nanos + tag; unix-only behaviors (real shells,
  `sleep`) are gated with `#[cfg(unix)]`. Follow that pattern.

## Remote-environment notes

Sessions run in an ephemeral container; commit and push to keep work. GitHub access is
via `mcp__github__*` MCP tools (no `gh` CLI), scoped to one repo. Don't open a PR unless
asked. Develop on the assigned `claude/*` branch.
