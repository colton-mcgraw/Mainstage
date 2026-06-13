# Mainstage Roadmap

This roadmap outlines the planned features and milestones for Mainstage. It is subject to change based on user feedback, development progress, and other factors.

---

## Goal 1: Core Language & Runtime

Delivers a fully functional Mainstage interpreter: the complete V1 grammar defined in `docs/GRAMMAR.md`, a working CLI, and a build runtime with change detection and pipeline orchestration.

---

### Phase 1: Lexer, Parser & AST

Build the language foundation. Output: a parser that turns `.ms` source into a typed AST with source locations on every node.

- [x] Define all AST node types in Rust (`Program`, `ImportDecl`, `LetDecl`, `ProjectBlock`, `StageBlock`, `PipelineBlock`, expression variants, step variants, condition variants)
- [x] Implement the lexer — tokenize `.ms` source into a token stream with file, line, and column spans
- [x] Implement a recursive-descent parser matching the EBNF in `docs/GRAMMAR.md`
- [x] Attach source spans to every AST node for downstream error reporting
- [x] Wire a `mainstage parse <file>` CLI subcommand that prints the AST (debug tool)

---

### Phase 2: Semantic Analysis

Validate the AST before execution. Output: a fully resolved, dependency-linked AST, or a set of user-facing errors with source locations.

- [x] Name resolution: `let` bindings, stage names, import aliases, `project.<field>` access
- [x] Forward reference enforcement: a `let` binding may not reference a binding declared after it
- [x] Resolve `<stage>.outputs` references — link each reference to its declaring `stage` node
- [x] Build the stage dependency graph from `inputs` / `outputs` / `<stage>.outputs` references
- [x] Uniqueness checks: stage names unique, pipeline names unique, at most one `default pipeline`
- [x] Type compatibility: both branches of an `if/else` expression must produce the same type

---

### Phase 3: Expression Evaluator & Built-in Variables

Evaluate expressions at script load time and within step argument positions.

- [x] String literals
- [x] String interpolation — evaluate `${expr}` embedded in strings
- [x] Boolean literals
- [x] List literal evaluation
- [x] `if/else` conditional expression — evaluate condition, return the matching branch
- [x] `platform` built-in variable — resolved from the host OS at startup
- [x] `project.<field>` access — available after the `project` block is evaluated
- [x] `glob(pattern, ...)` — evaluate glob patterns relative to the script directory; return a `fileset`
- [x] `fileset` type with per-file item properties: `.path`, `.name`, `.stem`, `.ext`, `.dir`

---

### Phase 4: Module System

Support `import` declarations and the built-in modules callable in expressions and conditions.

- [x] Module registry — resolve `import "<name>" as <alias>` to a Rust module implementation
- [x] `env` module: `env.get("VAR")`, `env.get("VAR", default: "...")`, `env("VAR")` condition form
- [x] `git` module: `git.tag()`, `git.sha()`, `git.sha(short: true)`
- [x] Named (keyword) argument support in module calls — `git.sha(short: true)`

---

### Phase 5: Step Executor

Execute individual steps inside `steps {}`, `on_failure {}`, and `on_success {}` blocks.

- [x] `$` exec step — tokenize the line into argv, resolve the program on `PATH`, run without a shell; apply string interpolation before tokenization
- [x] `copy <src> to <dest>` — cross-platform file or directory copy; create destination directory if absent
- [x] `move <src> to <dest>` — cross-platform file or directory move
- [x] `mkdir <path>` — create the full directory tree
- [x] `delete <path>` — remove a file or directory recursively; no-op if the path does not exist
- [x] `write <path> content: <string>` — write a string to a file, creating or overwriting it
- [x] `if/else` conditional steps
- [x] `for <var> in <fileset>` loop — bind `file.*` properties per iteration and run the body steps
- [x] Context variables inside step blocks: `inputs` (resolved fileset), `outputs` (declared output paths)

---

### Phase 6: Pipeline Runner & Failure Handling

Orchestrate stages in dependency order and handle failures per the propagation rules in `docs/GRAMMAR.md`.

- [x] Topological sort of the stage dependency graph
- [x] Sequential stage execution in DAG order
- [x] Stage-level `on_failure` block — run when that stage's steps fail, before cancellation propagates
- [x] `allow_failure: true` — treat a failed stage as succeeded; do not cancel downstream stages or trigger pipeline `on_failure`
- [x] Failure propagation — cancel all stages that depend (directly or transitively) on a failed stage's outputs
- [x] Pipeline `on_failure` and `on_success` hooks; bind `failed_stage` variable inside `on_failure`

---

### Phase 7: Change Detection

Skip stages whose inputs have not changed and whose declared outputs are already present.

- [x] Hash all files in a stage's resolved `inputs` set (SHA-256 per file, combined into a single digest)
- [x] Persist input digest and output path list to a local cache (`.mainstage/cache.json`)
- [x] On stage entry: if the input digest matches the cache and all declared output paths exist, skip the stage
- [x] Invalidate cache entries when output files are missing or deleted between runs
- [x] `mainstage clean` CLI subcommand — delete the cache and force a full rebuild

---

### Phase 8: CLI

Wire the CLI subcommands to the runtime and produce clear terminal output.

- [x] `mainstage` — run the `default pipeline`; user-facing error if none is declared
- [x] `mainstage run <name>` — run a named pipeline
- [x] `mainstage list` — list all declared pipelines with their stage names
- [x] `mainstage parse <file>` — print the parsed AST (from Phase 1, promoted to a stable debug tool)
- [x] `mainstage clean` — clear the change-detection cache (from Phase 7)
- [x] Structured terminal output: stage start/skip/pass/fail indicators, step output, failure summaries
- [x] Exit code propagation: exit non-zero when a pipeline fails

---

## Goal 2: Module System — Standard Library & Extensibility

Turns the hardcoded two-module dispatch into a trait-based registry, grows a real standard library, validates module calls at analysis time, and lets users add their own modules without forking or recompiling the core.

Today the module system is a hardcoded `match` in `core/src/modules.rs` that routes only `env` and `git`; there is no trait, no registry, and no validation — `import "bogus" as b;` passes semantic analysis and fails only at eval time, and method names, argument arity, and argument types are never checked before execution. This goal closes that gap and makes Mainstage's capabilities both growable (standard library) and user-extensible (plugins).

**Design decisions:**

- **Extensibility:** subprocess plugins — external executables on a search path that speak a newline-delimited JSON protocol over stdio. Cross-platform, no `unsafe`, language-agnostic, sandboxable, no ABI concerns. (Native dynamic libraries and WASM were considered and deferred.)
- **V1 standard library:** core essentials — `str`, `path`, `json`, and `fs` alongside the existing `env` and `git`, plus a `hash` helper reusing the Phase 7 SHA-256. `http`, `shell`, and `time` are deferred.
- **`json` (V1):** opaque-string form with path getters (`json.parse`, `json.get(text, "a.b.0")` returning strings), avoiding an extension of the `Value` enum and the ripple it would cause across interpolation and `if/else` type compatibility. A richer JSON value type is a possible later extension.
- **Network / `http`:** out of V1 — deferred until a permission/capability model exists.
- **Internals:** `MethodSig` is an owned type shared by built-in and plugin modules; plugin processes are long-lived for the duration of a single `mainstage` run; the registry is threaded through additive `eval_program_with` / `analyze_with` variants so existing signatures and tests are preserved; standard-library module names may never be shadowed by plugins.

---

### Phase 9: Registry Refactor (no behavior change)

Replace the hardcoded `dispatch` match with a `Module` trait and a `ModuleRegistry`. A pure refactor — `env` and `git` behave identically and no user-visible features change. Mirrors the existing `Reporter` trait idiom in `core/src/runner.rs`.

- [x] Define the `Module` trait and `MethodSig` / `Param` / `NamedParam` / `ValueTy` / `ModuleCx` / `ResolvedArg` in `core/src/modules/mod.rs`
- [x] Implement `ModuleRegistry` (`standard`, `get`, `method_sig`, `dispatch`) — `Arc`-backed and cheaply clonable
- [x] Port `env` → `EnvModule` and `git` → `GitModule`, with their unit tests, into `core/src/modules/builtin/`
- [x] Thread `ModuleRegistry` through `EvalContext` (and `clone_base`) and `eval_program_with`
- [x] Pass the same registry into `analyze_with`; construct it once in `cli/src/commands.rs::prepare`
- [x] Update `core/src/lib.rs` re-exports (`ModuleRegistry`, `Module`)

---

### Phase 10: Semantic Call Validation

Validate module names, method names, and argument arity and types during semantic analysis instead of at eval time.

- [x] Validate the `import "<name>"` string against the registry — `import "bogus" as b;` now errors at analysis time
- [x] Per `ModuleCall`: method exists; positional count within min/max; named arguments are recognized and required ones present; literal argument types match the declared `ValueTy`
- [x] Emit precise diagnostics carrying the call and argument `Span`
- [x] Keep eval-time errors as a defensive fallback — validated calls should never reach them

---

### Phase 11: Pure Standard Library

Add the deterministic, low-risk standard-library modules.

- [x] `str` — `upper`, `lower`, `trim`, `replace`, `split`, `join`, `contains`, `starts_with`, `ends_with`, `len`
- [x] `path` — `join`, `dir`, `base`, `stem`, `ext`, `with_ext`, `abs` (relative to the script directory)
- [x] `hash` — `sha256`, `sha256_file`, reusing the Phase 7 hasher
- [x] `env.has("VAR")` addition

---

### Phase 12: Read-only I/O Standard Library

Add side-effecting but read-only modules. File mutation stays in the existing `write` / `copy` / `move` / `delete` step layer.

- [x] `fs` — `exists`, `read`, `is_dir`, `is_file`, `size`, `list`
- [x] `json` — `parse`, `get(text, "a.b.0")`, `stringify` (opaque-string / path-getter form, no `Value` enum change)

---

### Phase 13: External Plugin Mechanism

Let users add modules via subprocess plugins that speak JSON over stdio — no core recompile required.

- [x] `describe` / `call` JSON protocol with `Value` and `MethodSig` (de)serialization
- [x] `ExternalModule` implementing the `Module` trait — runs `describe` at load, keeps a long-lived process for `call`
- [x] Plugin discovery: built-in registry first (no shadowing), then `.mainstage/plugins/<name>`, then a `plugins.toml` manifest; support namespaced names like `"acme/lint"`
- [x] Registry loads discovered plugins so semantic analysis validates plugin calls identically to built-ins
- [x] Error mapping (plugin `err` → `Error::Eval` with the call span) and failure modes (missing executable, malformed JSON, non-zero exit)

---

### Phase 14: Permissioned I/O Modules

Introduce a capability model, then the modules that require it.

- [x] Permission model — `--allow-run` / `--allow-net` (and `--allow-all`) flags **and** a manifest `[permissions]` block; the granted set is their union, defaulting to all-denied
- [x] `shell` module (`run`, capturing stdout), gated on the `run` capability
- [x] `http` module (`get`, `download`), gated on the `net` capability
- [x] `time` module (`now`, `unix`, `format`), ungated, with a determinism-vs-change-detection note in the module docs

---

### Phase 15: Docs, Grammar, and Testing

Make the module system discoverable and tool-assisted.

- [x] Add support for integer and boolean literal types in the `ValueTy` system, then update the grammar and docs to reflect them.
- [x] Document every standard-library module and the plugin protocol in `docs/GRAMMAR.md` and a new `docs/MODULES.md`
- [x] Update the `import_decl` grammar notes if namespaced plugin names require lexer changes
- [x] `mainstage modules` CLI subcommand listing available modules and their signatures (built-in and plugin)
- [x] Add example scripts to `tests/` that use the new standard library modules and a test plugin, covering both successful calls and validation errors.

## Goal 3: IDE Integration & Developer Experience

Brings Mainstage into the editor: a Language Server (the scaffolded `lsp` crate) that surfaces the analyzer's diagnostics live, offers completion, hover, and signature help driven by the module registry, and supports navigation — plus a `mainstage format` formatter for consistent, comment-preserving code style.

The language server is a thin protocol shell over `core`: it reuses the same `parse` / `analyze_with` pipeline and `ModuleRegistry` the CLI already builds, so editor behavior never diverges from command-line behavior. The formatter is the one piece that needs new groundwork — the grammar discards comments today — so a trivia-preserving syntax layer is added before the formatter is built on top of it.

**Design decisions:**

- **LSP stack:** the `lsp` crate runs a `tower-lsp` (0.20) server over stdio on a `tokio` runtime (both already wired into the workspace; the crate is a stub today). Editors launch it via a new `mainstage lsp` subcommand.
- **Single source of truth:** the server calls `core`'s `parse` and `analyze_with` with the same `ModuleRegistry` as the CLI — no duplicated language logic. Completion, hover, and signature help read `Module::methods()` and `MethodSig::signature()`; diagnostics are the existing `Vec<Diagnostic>` carried by `Error::Parse` / `Error::Semantic`.
- **Position mapping:** core `Span`s are 1-based `(line, col)` start/end pairs; the server converts them to 0-based LSP `Range`s with UTF-16 column semantics.
- **Document sync:** full-document sync in V1; incremental sync is deferred.
- **Formatter needs trivia:** `COMMENT` is a silent pest rule, so comments never reach the AST. The formatter is therefore built on a comment/trivia-preserving layer (Phase 20) rather than the lossy AST, guaranteeing comments survive a format.

---

### Phase 16: Language Server Foundation & Document Sync

Stand up the server and the analysis loop every later feature builds on. Output: an editor can connect, open a `.ms` file, and the server keeps an up-to-date parsed view of it.

- [x] Replace the `lsp` stub with a `tower-lsp` server over stdio on a `tokio` runtime — implement `initialize` (advertising server capabilities), `initialized`, and `shutdown`
- [x] In-memory document store keyed by document URI — handle `didOpen` / `didChange` (full sync) / `didClose`
- [x] Shared analysis entry point: a non-panicking helper that takes document text and script directory, runs `parse` → `analyze_with(registry)`, and returns the `Program` plus collected diagnostics for reuse by every feature
- [x] `Span` → LSP `Range` conversion (1-based core spans to 0-based, UTF-16 columns), with unit tests
- [x] `mainstage lsp` CLI subcommand that launches the server (the editor entry point)

---

### Phase 17: Live Diagnostics

Surface parse and semantic errors in the editor as the user types. Output: squiggles with the analyzer's messages, spans, and notes.

- [x] Publish `textDocument/publishDiagnostics` on open and change, debounced
- [x] Map `Error::Parse` / `Error::Semantic` (and the defensive `Error::Eval`) `Vec<Diagnostic>` to LSP `Diagnostic`s — message, span range, and `notes` as related information
- [x] Clear stale diagnostics when a document becomes valid again
- [x] Build the per-document `ModuleRegistry` with plugin discovery so import and plugin-call validation surfaces in the editor exactly as it does in the CLI

---

### Phase 18: Completion, Hover & Signature Help

Make the module registry discoverable from the editor — the registry as the single source of truth for available modules and their capabilities.

- [x] Module-name completion inside `import "<here>"`, sourced from `ModuleRegistry::module_names()`
- [x] Method completion after `<alias>.` — resolve the alias to its module from the parsed imports, list `Module::methods()`, and insert a call snippet derived from the `MethodSig`
- [x] Signature help inside a module call's `(...)` — render `MethodSig::signature()` and highlight the active positional or named parameter
- [x] Hover over a module alias or method showing its signature and return type; hover over `let` bindings, stage names, and `project.<field>` showing their resolved form
- [x] Completion for `let` bindings, stage names, and `project.<field>` in expression positions

---

### Phase 19: Navigation & Symbols

Let users move around a script. Output: jump-to-definition and an outline, reusing links the analyzer already builds.

- [x] Go-to-definition for `let`, import-alias, and `<stage>.outputs` references — reusing the resolution links from semantic analysis
- [x] Document symbols / outline: pipelines, stages, and top-level `let` bindings
- [x] Find references for stages and `let` bindings (rename deferred unless it falls out cheaply)

---

### Phase 20: Trivia-Preserving Syntax Layer

Groundwork for the formatter: stop throwing comments away. Output: a syntax representation that round-trips source exactly, including comments and blank-line grouping.

- [x] Capture comments (and blank-line grouping) during lexing/parsing instead of discarding them — un-silence the `COMMENT` rule or add a lossless token pass
- [x] Attach trivia to AST nodes as leading and trailing comments, distinguishing end-of-line from standalone comments
- [x] Round-trip guarantee: a no-op render of the trivia-aware tree reproduces the original source byte-for-byte, covered by golden tests across the example scripts

---

### Phase 21: `mainstage format`

Consistent, comment-preserving formatting from the CLI and the editor.

- [x] Pretty-printer over the trivia-aware tree: canonical indentation, spacing, and block layout for `import` / `let` / `project` / `stage` / `pipeline` / `steps` and their expressions, steps, and conditions
- [x] Preserve attached comments through formatting and keep blank-line grouping between top-level items
- [x] `mainstage format [FILES...]` formats in place; `--check` exits non-zero when any file is unformatted (CI gate); `--stdout` prints without writing
- [x] Idempotency and stability golden tests (`format(format(x)) == format(x)`)
- [x] LSP `textDocument/formatting` (and optional range formatting) reusing the same engine

---

### Phase 22: Docs, Editor Integration & Testing

Make the tooling usable and keep it covered.

- [x] Document the LSP feature set and editor setup (a minimal VS Code client plus generic LSP configuration) in `docs/`
- [x] Document `mainstage format` and recommend `format --check` alongside tests in CI
- [x] Integration tests: server lifecycle, a diagnostics fixture, completion / hover / signature-help snapshots, and formatter golden + idempotency suites

---

## Goal 4: Performance, Scalability, Stability & Polishing

Takes the working interpreter from "correct" to "production-grade": measures real workloads, runs independent stages concurrently, makes change detection cheap on large input sets, hardens the runtime against panics and interruptions, and polishes the CLI's output and ergonomics.

The headline scalability change is **parallel stage execution**. Today `run_pipeline_reported` in `core/src/runner.rs` walks a single topologically sorted list one stage at a time (`for stage_name in &sorted`), so independent branches of the dependency DAG never overlap. Goal 4 keeps the existing failure-propagation and change-detection semantics exactly, but lets ready stages run concurrently. The remaining phases — faster hashing, robustness, and UX — are lower-risk and can land independently of the runner change.

**Design decisions:**

- **Measure first:** a criterion-based benchmark harness and recorded baselines land before any optimization, so the parallelism and hashing phases prove their gains against real numbers rather than intuition.
- **Concurrency model:** bounded worker concurrency over the existing DAG, controlled by a `--jobs N` flag (defaulting to the host core count). The `Reporter` trait gains the contract that per-stage output is buffered and flushed atomically so concurrent stages never interleave on the terminal. Shared `cache` and `resolved_outputs` state is moved behind synchronization; failure propagation and `allow_failure` semantics from Phase 6 are preserved unchanged.
- **Change detection:** an `mtime` + size fast path short-circuits the SHA-256 digest when a file is provably unchanged; remaining hashing is parallelized. The on-disk cache format from Phase 7 stays compatible.
- **Stability:** the runtime must never panic on user input — parser fuzzing and property tests enforce this — and an interrupted run (Ctrl-C / SIGTERM) must leave the cache in a consistent state.

---

### Phase 23: Benchmarking & Profiling Harness

Establish a measurement baseline before optimizing. Output: reproducible benchmarks and recorded baseline numbers for the phases that follow.

- [x] Criterion benchmarks for `parse`, `analyze_with`, and `eval_program_with` over representative scripts
- [x] An end-to-end `run_pipeline` benchmark over a large synthetic project fixture (many stages, deep dependency chains, large filesets)
- [x] A fixture generator for synthetic projects parameterized by stage count, DAG depth, and files-per-stage
- [x] Record baseline timings in the repo so Phases 24 and 25 can demonstrate measurable improvement (`docs/BENCHMARKS.md`)

---

### Phase 24: Parallel Stage Execution

Run independent branches of the dependency DAG concurrently while preserving the exact ordering and failure semantics of Phase 6. Output: pipelines complete faster on multi-core hosts with identical results.

- [x] Schedule stages by readiness (all dependencies complete) instead of a single linear toposort, with bounded worker concurrency
- [x] `--jobs N` CLI flag (default: host core count; `--jobs 1` forces the current sequential behavior)
- [x] Make `Reporter` output deterministic — buffer each stage's output and flush it atomically so concurrent stages never interleave on the terminal
- [x] Guard shared `cache` and `resolved_outputs` state behind synchronization; ensure a dependent always observes its dependency's published outputs
- [x] Preserve failure propagation, `allow_failure`, and `on_failure` / `on_success` semantics exactly; cover with concurrent-execution tests

---

### Phase 25: Faster Change Detection

Make the skip-check cheap on large input sets. Output: unchanged stages are detected without re-hashing every file.

- [x] `mtime` + size fast path that short-circuits the SHA-256 digest when a file is provably unchanged, falling back to hashing on ambiguity
- [x] Parallelize per-file hashing for the inputs that still require it
- [x] Avoid redundant re-globbing and re-hashing of filesets already resolved during a run
- [x] Keep the `.mainstage/cache.json` format backward-compatible; benchmark against the Phase 23 baselines

---

### Phase 26: Robustness & Stability

Harden the runtime against malformed input and interruption. Output: no panics on any input, and a clean state after an interrupted run.

- [x] Handle Ctrl-C / SIGTERM: cancel in-flight stages gracefully and leave the cache in a consistent state
- [x] Parser and lexer fuzzing (e.g. `cargo-fuzz`) plus property tests asserting the pipeline never panics on arbitrary input
- [x] Stress tests for large filesets, deep DAGs, and wide fan-out, run under the parallel scheduler
- [x] Audit `unwrap` / `expect` / `panic!` on user-input paths and convert them to user-facing diagnostics

---

### Phase 27: CLI Polish & UX

Make day-to-day use pleasant. Output: clear, configurable terminal output and convenience commands.

- [x] TTY-aware colored output with `--verbose`, `--quiet`, and `--no-color` controls (respecting `NO_COLOR`)
- [x] `--dry-run` — show the planned execution order and which stages would run or skip, without executing
- [x] `mainstage watch` — re-run the pipeline when inputs change
- [x] Refined error formatting (source snippets with carets) and an end-of-run timing summary per stage

---

## Goal 5: Deployment & Ecosystem

Gets Mainstage into users' hands and builds the surrounding ecosystem: continuous integration to keep `main` green, reproducible cross-platform release binaries, distribution through the channels people actually install from, a published editor extension, tooling for plugin authors, and a real documentation and onboarding story.

This goal is mostly new infrastructure rather than language work — there is no `.github/` directory, CI, or release tooling in the repository today. Each phase is largely independent, so they can be tackled in any order once CI (Phase 28) is in place.

**Design decisions:**

- **Licensing:** MIT is treated as the project license for planning purposes. The badge / `license-file` ("Source Available") and the README's "MIT" reference are currently inconsistent; this is reconciled as part of Phase 29 before any public publish, but does not block earlier infrastructure work.
- **CI as the foundation:** Phase 28 lands first and gates every later phase — releases, package manifests, and the editor extension all build on a green, multi-platform CI matrix that also runs `mainstage format --check`.
- **Wide distribution:** Goal 5 deliberately targets many install channels (install script, crates.io, Homebrew, Scoop/winget, Docker) so users on any platform can install with one familiar command. Release binaries are built once (Phase 29) and the package manifests in Phase 30 consume those artifacts.
- **Ecosystem, not just binaries:** the plugin protocol (Phase 13) and the editor client (Phase 22) become first-class, published, documented surfaces so the community can extend Mainstage without forking it.

---

### Phase 28: Continuous Integration

Keep `main` green across platforms. Output: every push and PR is built, tested, linted, and format-checked on all three target OSes.

- [x] GitHub Actions workflow running `cargo build`, `cargo test`, `cargo clippy -D warnings`, and `cargo fmt --check` for the `core`, `cli`, and `lsp` crates
- [x] Run `mainstage format --check` over the example scripts as a CI gate (per Phase 21)
- [x] Matrix across Linux, macOS, and Windows on stable Rust (edition 2024)
- [x] Cache the cargo registry and build artifacts for fast CI

---

### Phase 29: Release Engineering & Cross-Platform Binaries

Produce reproducible, downloadable binaries on every tagged release. Output: a GitHub Release with signed checksums and binaries for every supported target.

- [x] Adopt semantic versioning and maintain a `CHANGELOG`
- [x] Reconcile the license: pick MIT (or the intended license) consistently across `LICENSE.md`, the README badge, and the workspace `license-file` — settled on the existing Source-Available License; fixed the README footer to match
- [x] Tag-triggered release workflow building Linux (gnu + musl), macOS (x86_64 + arm64), and Windows binaries
- [x] Attach binaries and SHA-256 checksums to the GitHub Release

---

### Phase 30: Distribution & Package Managers

Make Mainstage installable through the channels users already use. Output: one-line installs on every major platform. *(Consumes the Phase 29 release artifacts.)*

- [x] `curl | sh` install script that downloads the right release binary for the host platform
- [x] Publish the crates to crates.io so `cargo install mainstage` works
- [x] Homebrew tap (macOS / Linux) and Scoop / winget manifests (Windows)
- [x] Docker image with the CLI as its entry point

---

### Phase 31: Editor Extension Publishing

Ship the editor experience built in Goal 3. Output: a one-click install for VS Code and clear setup for other editors.

- [x] Package the Phase 22 VS Code client and publish it to the Visual Studio Marketplace and OpenVSX
- [x] Bundle or auto-discover the `mainstage lsp` binary so the extension works without manual configuration
- [x] Document generic LSP setup for other editors (Neovim, Helix, etc.)

---

### Phase 32: Plugin Ecosystem & Scaffolding

Make the Phase 13 plugin protocol approachable for authors. Output: scaffolding, a publishing guide, and a discoverable index.

- [x] `mainstage plugin new` scaffolding that emits a working stdio plugin (`describe` / `call`) skeleton
- [x] An authoring and publishing guide for the JSON-over-stdio protocol, including versioning and namespacing conventions
- [x] A discoverable index of community plugins and reference example plugins
- [x] Validation/lint command that checks a plugin against the protocol before publishing

---

### Phase 33: Project Docs, Website & Onboarding

Give newcomers a clear path in. Output: a docs site, a getting-started guide, and an honest project status.

- [x] Getting-started guide and an examples gallery beyond the single `main.ms`
- [x] A docs site rendering the existing `docs/` (grammar, modules, tooling, roadmap)
- [x] `CONTRIBUTING.md` covering the workspace layout, tests, and the CI gates
- [x] Update the README "not yet usable" status once Goals 4 and 5 land

---
