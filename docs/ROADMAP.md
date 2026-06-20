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

## Goal 6: Expressive Builds — Ordering, Reuse, Incrementality & Testing

Closes the gaps that surface when Mainstage drives a large, multi-target build like the dogfood `main.ms` OS script. That script is, in effect, a bug report against the language: nearly every explanatory comment documents a workaround for a feature the runtime does not yet have. This goal turns those workarounds into first-class constructs.

Today the language infers stage order *only* from `inputs` / `outputs` file references, caches at whole-stage granularity, and has no abstraction that produces stages — so multi-target builds resort to `-j1` plus list order for sequencing, sentinel files to force or suppress caching, copy-pasted per-architecture stages, and `sh -c` strings to escape the no-shell step model. Goal 6 makes ordering, always-run/run-once, step-level failure tolerance, stage parameterization, incremental rebuilds, and a real test harness explicit, while preserving the change-detection and failure-propagation semantics of Goals 1 and 4.

**Design decisions:**

- **Ordering is additive, not a replacement:** the inferred `inputs`/`outputs` dependency graph (Phase 2) stays the source of truth; `depends_on` only adds edges the file graph cannot express (side-effecting setup, "boot after build"). Cycles across the combined graph are a semantic error.
- **Caching becomes intent-revealing:** `always_run` and a success "stamp" replace the two sentinel-file hacks (`outputs: [".always"]` to force re-runs; empty `inputs`/`outputs` to mean "run every time") with declared behavior the reporter and `--dry-run` understand.
- **Parameterization produces stages, not values:** the `matrix` lowering expands one authored stage into N concrete stages *before* analysis, so the dependency graph, change detection, and parallel scheduler see ordinary stages and need no changes. The matrix value is exposed as a built-in alongside `platform`.
- **Incrementality stays compatible:** per-output change detection refines the Phase 7 / Phase 25 cache (mapping each declared output to the inputs that produced it) without breaking the `.mainstage/cache.json` format or the whole-stage fast path.
- **Testing reuses existing machinery:** the test harness is a stage flavor over the Phase 24 reporter — buffered, atomic output, exit-code propagation — not a parallel runtime; assertions are steps so they compose with `for`, `if`, and interpolation.

---

### Phase 34: Explicit Stage Ordering (`depends_on`)

Let stages declare ordering edges the file graph cannot infer. Output: `main.ms` runs `initialize → build → run` correctly under full parallelism, with no `-j1` workaround.

- [x] Add a `depends_on: [<stage>, ...]` stage field that adds dependency edges without requiring a referenced file artifact
- [x] Merge `depends_on` edges into the Phase 2 dependency graph; detect and report cycles across the combined inputs/outputs + `depends_on` graph with source spans
- [x] Honor the new edges in the Phase 24 readiness scheduler and in failure propagation / cancellation, identically to inferred edges (automatic — the scheduler consumes the merged graph)
- [x] Surface ordering edges in `--dry-run` (dependency waves) and `mainstage list` (`(after …)` annotations); document that `stages:` is membership while ordering comes from the graph
- [x] Demonstrate `depends_on` in the example `main.ms`; a multi-target build (e.g. the OS dogfood script) can now express `initialize → build → run` directly and drop the `-j1` workaround

---

### Phase 35: Always-Run & Run-Once Stages

Make "never cache this" and "cache success without a file artifact" declared behaviors instead of sentinel-file tricks. Output: the `run*` stages stop faking outputs, and `initialize` stops re-running `apt-get` on every invocation.

- [x] `always_run: true` stage field — the stage runs every invocation regardless of inputs/outputs; replaces the `outputs: ["build/run/.always"]` hack in `run` / `run_gui` / `run_arm64`
- [x] Success "stamp" semantics via `run_once: true` — a stage with side effects but no file outputs records success in the cache (a stable empty-input stamp) so it is skipped on re-run, covering the `initialize` apt-get case
- [x] Reconcile with the Phase 7 rule that empty-`inputs`/`outputs` stages always run; the default is documented explicitly and `always_run` / `run_once` make it adjustable (the two are mutually exclusive, checked in sema)
- [x] Reflect both states in `--dry-run` (would-run vs would-skip) and the end-of-run summary — both consume the shared `change_detection_inputs` decision so the plan and the run agree

---

### Phase 36: Step-Level Failure Tolerance & Richer File Steps

Bring the `sh -c` escape hatches back into native, checkable steps. Output: `main.ms` expresses its setup and staging file ops without dropping to a shell.

- [x] Step-level failure tolerance via a `try { }` block so steps may continue on non-zero exit — replaces `sh -c "... || true"` (e.g. `apt-get update`). A failure inside the block is swallowed (the stage does not fail and `on_failure` does not fire); captured output is still shown
- [x] Force/overwrite semantics for `copy` (removes an existing destination first, so a read-only target is replaced) — replaces `sh -c "cp -f ..."`
- [x] Confirmed `delete` + `mkdir` + `copy` compose to replace the `sh -c "rm -rf ... && mkdir ... && cp ..."` ESP-staging steps; `mkdir` already creates parents, `delete` is recursive, and `copy` now force-overwrites — no missing primitive
- [x] Documented when to prefer native steps over `$ sh -c` (GRAMMAR.md) and added a `try` block to the example `main.ms`

---

### Phase 37: Parameterized Stages / Build Matrix

Stop copy-pasting per-architecture stages. Output: one authored bootloader stage and one kernel stage expand to their x64 and arm64 variants.

- [x] `matrix { <dim>: [<values>] }` on a stage, lowered to N concrete stages before semantic analysis so the graph, change detection, and scheduler are unchanged
- [x] Expose the active matrix value as a built-in (alongside `platform`) for use in flags, paths, and tool-name selection
- [x] Deterministic generated stage names (e.g. `bootloader[x64]`) usable in `pipeline` stage lists, `depends_on`, and `<stage>.outputs` references
- [x] Validate matrix dimensions/values and report conflicts (duplicate expansion names, empty dimensions) with spans
- [x] Collapse the duplicated `bootloader_*` / `kernel_*` stages in `main.ms` to a single matrixed definition each

---

### Phase 38: Per-Output (Incremental) Change Detection

Refine change detection below whole-stage granularity so editing one source recompiles one object. Output: tight edit/rebuild loops for large stages like the eight-file kernel build.

- [x] Map each declared output to the subset of inputs that produced it (pattern-rule or per-output input association) so unaffected outputs are skipped
- [x] Reuse the Phase 25 mtime+size fast path and parallel hashing per output; keep the `.mainstage/cache.json` format backward-compatible
- [x] Combine with `for file in inputs { ... }` so per-file compile loops gain per-object caching instead of re-running the whole stage
- [x] Benchmark incremental single-file edits against the Phase 23 baselines and the current whole-stage behavior

---

### Phase 39: Test Harness

Make tests first-class instead of a bare non-zero exit. Output: a stage flavor that reports pass/fail counts and assertion failures, suitable for unit tests and OS boot-smoke checks.

- [x] A `test` stage flavor (never cached, like Phase 35 `always_run`) whose result is a pass/fail tally rather than a single exit code
- [x] `expect` / `assert` steps — assert a command's exit status and/or compare its captured stdout/stderr against an expected value, with interpolation support
- [x] Capture-and-match assertions usable for boot-smoke tests (run a `$` step, scrape output for an expected marker, fail on timeout)
- [x] Integrate with the Phase 24 reporter (buffered, atomic per-stage output) and exit-code propagation; add a `--quiet`-aware summary line
- [x] Document the harness in `docs/GRAMMAR.md`/`docs/MODULES.md` and add `tests/` example scripts covering passing and failing assertions

---

### Phase 40: Discovery & Ergonomics

Polish the long-tail papercuts a large multi-stage script exposes. Output: portable firmware/tool discovery and a navigable stage list.

- [x] First-existing-path discovery helper (e.g. `fs.find_first([...])`) so hardcoded firmware paths like `OVMF_CODE_4M.fd` vs `OVMF_CODE.fd` resolve portably across distros
- [x] Optional `description:` field on stages, surfaced by `mainstage list` (and an optional `--describe`) so a 12-stage build is navigable from the CLI
- [x] Carry stage descriptions and ordering into LSP document symbols / hover (reusing Goal 3 infrastructure)
- [x] Update `main.ms` to use path discovery and stage descriptions

---

## Goal 7: Leaf Expressiveness — Conditions, Step Context, Locals & Reuse

Closes the expressiveness gaps that surface at the *leaves* of the language — the places
where an author has a value in hand but no way to act on it. The architecture from Goals
1–6 is sound (everything funnels through `prepare()`; exhaustive matches turn new
constructs into compile errors until fully wired; modules are validated at analysis time);
this goal is deliberately **not** a refactor. It adds expressiveness to conditions, steps,
and bindings without changing the pass pipeline, the dependency graph, or the cache format.

The motivating observations: a `condition` can only test `env()` and `platform`, so a value
held in a `let` cannot be compared (`grammar.pest` `primary_cond`); `$` steps always run in
`script_dir` with the inherited environment (`executor.rs`), so `cd` and per-command env
still force a drop to `sh -c` — the exact escape hatch Goal 6 set out to retire; there is no
native way to print a message or fail deliberately; `let` is top-level only, so derived paths
are repeated or hoisted; and the Phase 39 assertion matchers are limited to `contains` /
`equals`. Each phase below targets one of these, ordered by impact-to-effort.

**Design decisions:**

- **Conditions become expression-operand comparisons, additively:** the existing `env_cond`
  / `platform_cond` forms stay (and stay the canonical spelling for env/platform); a new
  general form compares two arbitrary expressions with `==` / `!=` plus a `contains` / `in`
  membership test and an emptiness predicate. Reuses eval-time `Value` equality — no new
  runtime machinery. Type compatibility of the two operands is checked in `sema` exactly as
  `if/else` branch compatibility already is.
- **Step context is a block, not per-token soup:** `workdir "<path>" { … }` and
  `with_env { KEY: <expr> … } { … }` blocks set the working directory and environment for
  the steps they enclose, nesting and composing with `if` / `for` / `try`. This keeps the
  `$` line greedy-parsed (no modifier tokens after the command) and gives copy/move/write
  the same context for free.
- **Diagnostics and explicit failure are first-class steps:** `log "<msg>"` routes through
  the `Reporter` (so it respects `--quiet` and buffered/atomic per-stage output) rather than
  shelling out to `echo`; `fail "<reason>"` fails the stage deliberately with a user-facing
  diagnostic, composing with `if` to assert invariants.
- **Locals are block-scoped and immutable:** a `let` permitted inside any step block binds a
  name for the remainder of that block (including per-iteration inside `for`), shadowing is a
  semantic error, and forward-reference rules mirror the top-level `let` pass.
- **Assertions round out the matcher set without a regex engine in core:** add
  `not_contains`, `starts_with`, `ends_with`, and `matches` (anchored glob-style, reusing the
  existing matcher rather than pulling in a regex dependency unless one already exists).
- **Reuse is a pre-analysis expansion, like `matrix`:** a `template` of steps is inlined into
  each referencing stage *before* semantic analysis, so the graph, change detection, and
  scheduler see ordinary stages and need no changes — the same lowering discipline Phase 37
  established.

---

### Phase 41: Expression-Based Conditions

Let conditions compare arbitrary expressions, not just `env()` / `platform`. Output: a
`let`, module-call result, or `project.<field>` can drive an `if/else` expression or `if`
step directly, retiring the "route everything through `env()`" workaround.

- [x] Extend `primary_cond` in `grammar.pest` with a general comparison form
      (`<expr> (== | != | contains | in) <expr>`) and an emptiness predicate, keeping
      `env_cond` / `platform_cond` as-is and ordering alternatives so the specific forms win
- [x] Add the AST node(s) and `span` arm; thread through `parser.rs` (`build_condition`)
- [x] `sema.rs`: resolve operand sub-expressions and check operand type compatibility (reuse
      the `if/else` branch-compatibility logic) with precise diagnostics carrying operand spans
- [x] `eval.rs`: evaluate the general comparison via `Value` equality / list membership;
      keep the env/platform fast paths
- [x] `format.rs`, `navigation.rs` (`walk_steps`), and the LSP feature files: add the new arm
- [x] Docs (`docs/GRAMMAR.md` conditions section + EBNF) and tests: parser, sema (type
      mismatch), eval, and an example `.ms` exercising a `let`-driven condition

---

### Phase 42: Step Execution Context (`workdir` / `with_env`)

Give steps a working directory and environment without dropping to a shell. Output: the most
common remaining `sh -c "cd … && …"` and `FOO=bar cmd` patterns become native, checkable steps.

- [x] `workdir "<path>" { <step>* }` and `with_env { <key>: <expr>, … } { <step>* }` block
      steps in the grammar, AST, and parser; both nest and compose with `if` / `for` / `try`
- [x] Thread an execution context (cwd + env overlay) through `executor.rs::execute_step`,
      replacing the hardcoded `current_dir(&ctx.script_dir)` with the active context; apply to
      `$`, `copy`, `move`, `write`, `mkdir`, `delete` uniformly
- [x] `sema.rs`: resolve `with_env` value expressions and the `workdir` path; validate the
      path is a string-typed expression
- [x] Confirm `workdir` + `with_env` compose to replace the `sh -c "cd … && VAR=… cmd"`
      escape hatch; document when to prefer them over `$ sh -c` (`docs/GRAMMAR.md`)
- [x] `format.rs` / LSP exhaustive-match arms; tests for nesting, env overlay precedence, and
      a relative `workdir` resolved against `script_dir`

---

### Phase 43: Diagnostic & Control-Flow Steps (`log` / `fail`)

Make printing a message and failing deliberately first-class. Output: scripts emit progress
and assert invariants without `$ echo` or a sentinel non-zero command.

- [x] `log "<msg>"` step (interpolated) routed through a new `Reporter` method (default no-op
      body so `NoopReporter` / test reporters keep compiling), honoring `--quiet` and buffered
      per-stage output
- [x] `fail "<reason>"` step that fails the enclosing stage with a user-facing `Error::Eval`
      diagnostic carrying the step span; interacts with `try` (swallowed) and `on_failure`
      (fires) exactly like a failed command
- [x] Grammar / AST / parser / `sema` (interpolation resolution) / `eval` wiring; `format.rs`
      and LSP `walk_steps` arms
- [x] Docs and tests: `log` output under `--quiet` vs default, `fail` inside `if`, and `fail`
      inside `try` not propagating

---

### Phase 44: Block-Scoped Bindings (local `let`)

Allow `let` inside step blocks so derived values are named once. Output: multi-path stages and
`for` loop bodies stop repeating interpolated expressions.

- [x] Permit `let <ident> = <expr>;` as a step; scope the binding to the remainder of its
      enclosing block (including per-iteration inside `for`)
- [x] `sema.rs`: extend name resolution into step scopes with the top-level forward-reference
      rule; report shadowing of an outer binding as a semantic error with both spans
- [x] `eval.rs` / `executor.rs`: maintain a scoped binding environment while executing a block;
      ensure `EvalContext` field additions update `eval_program_with`, `clone_base`, and the
      test helpers
- [x] `format.rs` / LSP (completion of locals in scope, go-to-definition) arms; tests for
      scoping, shadowing errors, and a `for`-loop-local binding

---

### Phase 45: Richer Assertion Matchers

Round out the Phase 39 test harness. Output: smoke tests can assert the *absence* of a marker
and match on prefixes/suffixes/patterns.

- [x] Extend `match_op` with `not_contains`, `starts_with`, `ends_with`, and `matches`
      (anchored glob-style; reuse an existing matcher rather than adding a regex dependency)
- [x] Wire through `expect_output` and `assert_step` in AST / parser / `sema` / `executor`
- [x] `format.rs` / LSP arms; docs in `docs/GRAMMAR.md` / `docs/MODULES.md`
- [x] Tests: each matcher passing and failing, plus a boot-smoke example asserting an error
      marker is absent from captured output

---

### Phase 46: Reusable Step Templates

Factor a shared sequence of steps out of unrelated stages. Output: common setup/teardown step
runs are authored once and inlined, complementing `matrix` (which parameterizes over values).

- [x] A top-level `template <ident> { <step>* }` item and a `use <ident>;` step that inlines
      it, lowered *before* semantic analysis so the graph, change detection, and scheduler are
      unchanged (mirrors the Phase 37 `matrix` expansion discipline)
- [x] Validate template names (uniqueness, referenced template exists, no recursive `use`
      cycles) with source spans
- [x] Surface templates in `format.rs`, LSP document symbols, and go-to-definition for `use`
- [x] Docs (`docs/GRAMMAR.md`) and tests: inlining, a cycle error, and an example `.ms`
      sharing a template across two stages

---

### Phase 47: LSP Editor Integration Improvments

Make the LSP server integration more ergonomic. Output: a VS Code extension that works out of the box and a generic LSP client that is easy to configure.

- [x] Remove auto discover and instead bundle the `mainstage lsp` binary with the VS Code extension so it works without manual configuration.
- [x] Improved VS Code extension stability when running in VS Code remote containers or WSL, including better error handling and logging.

---

## Goal 8: Scaling Up — Composition, Configuration, Shared Caching & Introspection

Goals 1–7 made the *language* expressive: a single `.ms` file can already drive a
sophisticated, parallel, incremental, well-tested build. The gap that remains is not in
the language — it is in the **tool**. Today a build is one file, on one machine, with an
opaque graph: there is no way to split a build across a large repository, parameterize it
from the command line beyond routing everything through `env()`, share built artifacts
between machines or CI runs, restore outputs after a `clean` or a branch switch, or ask
the graph *why* a stage ran. Those are exactly the capabilities that separate a competent
build system (the Bazel / Buck / Nx / Turborepo / Gradle tier) from a nice single-file
task runner. Goal 8 closes that distance.

It is deliberately **not** a rewrite. Every phase preserves the pass-pipeline discipline
the codebase already runs on — everything funnels through `cli/src/commands.rs::prepare()`
(`parse → expand_matrix → expand_templates → analyze_with → eval_program_with`); new
multi-file and parameter constructs **lower before semantic analysis**, exactly like
`matrix` (Phase 37) and `template` (Phase 46), so the dependency graph, change detection,
and the parallel scheduler keep seeing one flat, ordinary `Program`; exhaustive matches in
`format.rs` / `navigation.rs` / `ast.rs::span()` continue to turn every new node into a
compile error until it is fully wired; and the on-disk cache format (`cache.rs`,
`.mainstage/cache.json`) stays backward compatible (new fields default in).

**To future agents** : Backward compatibility can be ignored! The user base is small and early, so breaking changes are cheap. The design decisions below reflect the current state of the codebase and the trade-offs it presents; if a later phase reveals a better way to do something, feel free to break compatibility and change course. Including the previously mentioned caching; a breaking change to the cache format is not only acceptable but likely necessary to implement a content-addressed output store (Phase 50), so the Phase 50 design decision reflects the ideal end state, not the incremental steps to get there.

**Design decisions:**

- **Composition is lexical inclusion, not a package manager.** An `include` merges the
  items of another `.ms` file into the program *before* analysis, so the build remains a
  single flat graph — no cross-file runtime, no dependency resolver, no network fetch. The
  hard problems are name collisions across files and `script_dir`/`glob` resolution, which
  are settled with explicit rules rather than a module system.
- **Parameters are typed and declared, not stringly-typed env reads.** A `param` item
  carries a type and a default and is overridable from the CLI (`-D name=value`). It
  retires the "route everything through `env()`" idiom the way Phase 41 retired it for
  conditions: env stays available, but a build's knobs become first-class, validated, and
  discoverable (`mainstage list`, `--dry-run`, LSP).
- **Caching gains a content-addressed output store (CAS), then a transport.** The existing
  input-digest cache is extended to record each output's *content* hash; a local CAS keyed
  by those hashes lets a stage's outputs be **restored** rather than only **skipped** —
  surviving `clean`, branch switches, and fresh checkouts. A remote backend is then a thin
  push/pull transport over that same CAS keyed by the same digest, so local and shared
  caches never diverge. A cache miss, timeout, or backend error must **never fail a
  build** — caching is an optimization, degrading to local-only.
- **Introspection reuses what the analyzer already built.** `query` / `explain` / `profile`
  read the dependency graph (`AnalysisResult`) and the change-detection decision
  (`change_detection_inputs`) and the Phase 27 timing summary — no new analysis pass.
- **Hermeticity is opt-in and additive.** Declared tool requirements and an optional
  per-stage environment isolation wrap execution without changing the step model; a
  double-build reproducibility check is a runner mode, not new language.

---

### Phase 48: Multi-File Composition (`include`)

Let a build span many files and directories so a growing repository is not one giant
`main.ms`. Output: a root script can pull in per-component `.ms` files and the runtime
sees a single, flat build graph.

- [x] `include "<path>";` top-level item that lexically merges another `.ms` file's items
      into the program, resolved relative to the including file; lower it in `prepare()`
      **before** semantic analysis (mirroring `matrix.rs` / `templates.rs`) so the graph,
      change detection, and scheduler only ever see one ordinary `Program`
- [x] Cycle detection across the include graph, deterministic include ordering, and
      duplicate-include de-duplication — each reported with a source span
- [x] Name-collision rules across files for `stage` / `let` / `template` / `pipeline` /
      `param` (a documented flat namespace with a precise collision error, or qualified
      names) so two components can't silently clobber each other — settled on a **flat
      namespace**: included items share one namespace and a duplicate name is reported by
      the existing `sema` duplicate-name checks (cross-file references stay unqualified)
- [x] Define and test `script_dir` / `glob` / relative-path resolution per *including* vs
      *included* file (a glob in an included file resolves against that file's directory),
      and carry the originating file+span on every node for diagnostics — step relative
      paths still resolve against the run's script directory (or an enclosing `workdir`),
      documented in `docs/GRAMMAR.md`
- [x] grammar / ast / parser / sema wiring; `format.rs` and LSP (`navigation.rs`) arms;
      cross-file go-to-definition (the cursor on an `include` jumps to the included file);
      docs in `docs/GRAMMAR.md` and a multi-file example project under `examples/`

---

### Phase 49: CLI Parameters & Build Configurations

Replace the `env()`-for-everything idiom with typed, declared build parameters that are
overridable from the command line. Output: `mainstage -D release=true run ci` instead of
exporting environment variables.

- [x] `param <ident>: <type> = <default>;` top-level item (`string` / `int` / `bool` /
      `list`), resolved at load time in declaration order alongside `let` and referenceable
      anywhere a `let` is
- [x] `-D <name>=<value>` / `--param <name>=<value>` CLI flags (and an optional manifest
      `[params]` block) to override defaults, with typed parsing and precise diagnostics on
      an unknown name or a type mismatch
- [x] `sema.rs` validation (unique names, default's type matches the declared type,
      forward-reference rule); `eval.rs` wiring (thread the resolved param set through
      `EvalContext` / `clone_base` / test helpers)
- [x] Surface parameters and their effective values in `mainstage list`, `--dry-run`, and a
      `mainstage params` listing; LSP completion and hover; `format.rs` + LSP exhaustive
      arms; docs in `docs/GRAMMAR.md` and an example

---

### Phase 50: Content-Addressed Output Cache

Make change detection *restore* outputs, not just skip stages. Output: outputs survive a
`mainstage clean`, a branch switch, or a fresh checkout without re-running the stage.

- [x] Extend `cache.rs` to record each declared output's content hash at a successful run,
      and store output blobs in a local content-addressed store under `.mainstage/cache/`
      keyed by digest (new fields default in — old `cache.json` still loads)
- [x] On a cache hit whose outputs are *missing* from the tree, restore them from the CAS
      instead of re-running the stage; fall back to a full rebuild when a blob is absent
- [x] `mainstage cache gc` (prune unreferenced blobs) and `mainstage cache stats` (size /
      hit-rate reporting); a configurable size ceiling with LRU eviction
- [x] Reuse the Phase 25 mtime+size fast path and parallel hashing for output digests;
      benchmark restore-from-CAS vs. rebuild against the Phase 23 baselines

---

### Phase 51: Remote / Shared Cache

Share built artifacts across machines and CI runs. Output: a second machine (or a CI job)
that has never built the project pulls finished outputs instead of recomputing them.

- [ ] A `CacheBackend` trait with a local-directory backend and an HTTP backend; push/pull
      CAS blobs and cache entries keyed by the **same** digest the local cache uses, so a
      remote hit is indistinguishable from a local one
- [ ] `--remote-cache <url>` flag and a manifest `[cache]` block; read-through /
      write-through policy plus a read-only mode for untrusted CI; gate the network on the
      existing `net` capability
- [ ] Graceful degradation: a cache miss, timeout, auth failure, or malformed blob logs a
      warning and falls back to local-only — **never** fails the build
- [ ] Integration tests against an in-process fake backend (hit, miss, corruption,
      timeout); document the wire protocol and a CI recipe in `docs/`

---

### Phase 52: Build Graph Query & Explain

Make the dependency graph and the change-detection decisions inspectable. Output: an author
can see *why* a stage ran, what depends on it, and where the time went.

- [x] `mainstage query` — print the stage dependency graph and its reverse edges, filtered
      by pipeline, with DOT and JSON export for external tooling, reading `AnalysisResult`
- [x] `mainstage explain <stage>` — why the stage ran or was skipped on the last run: which
      input changed, which output was missing, a whole-stage vs. per-output decision, a
      local hit, or a CAS/remote restore (reading the `change_detection_inputs` decision)
- [x] `mainstage profile` / a `--profile` flag — per-stage timings and the critical path,
      building on the Phase 27 end-of-run timing summary
- [x] Tests over a fixture graph (diamond + fan-out) asserting query output and explain
      verdicts; a `docs/` page

---

### Phase 53: Hermeticity & Reproducibility

Move builds from "works on my machine" toward reproducible. Output: a build declares the
tools it needs, can isolate itself from ambient state, and can be checked for determinism.

- [ ] Declared tool requirements — a `requires { … }` stage field or a top-level
      `tool`/`toolchain` item asserting a program is present and (optionally) a version
      constraint, checked before the stage runs with a clear "missing/mismatched tool"
      diagnostic
- [ ] Optional per-stage environment isolation (`hermetic: true`): run with a cleared
      environment plus an explicit passthrough/`with_env` allowlist, so a stage can't
      silently depend on ambient variables
- [ ] `--check-reproducible` — run a pipeline twice and diff output content hashes,
      reporting the specific non-deterministic outputs (reusing the Phase 50 output hashing)
- [ ] Input-completeness audit: where the platform allows, warn when a stage reads files
      outside its declared `inputs` (the most common cause of a stale cache); `docs/` +
      an example

---
