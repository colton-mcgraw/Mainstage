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

- [ ] Pretty-printer over the trivia-aware tree: canonical indentation, spacing, and block layout for `import` / `let` / `project` / `stage` / `pipeline` / `steps` and their expressions, steps, and conditions
- [ ] Preserve attached comments through formatting and keep blank-line grouping between top-level items
- [ ] `mainstage format [FILES...]` formats in place; `--check` exits non-zero when any file is unformatted (CI gate); `--stdout` prints without writing
- [ ] Idempotency and stability golden tests (`format(format(x)) == format(x)`)
- [ ] LSP `textDocument/formatting` (and optional range formatting) reusing the same engine

---

### Phase 22: Docs, Editor Integration & Testing

Make the tooling usable and keep it covered.

- [ ] Document the LSP feature set and editor setup (a minimal VS Code client plus generic LSP configuration) in `docs/`
- [ ] Document `mainstage format` and recommend `format --check` alongside tests in CI
- [ ] Integration tests: server lifecycle, a diagnostics fixture, completion / hover / signature-help snapshots, and formatter golden + idempotency suites

---

## Goal 4: Performance, Scalability, Stability & Polishing

## Goal 5: Deployment & Ecosystem

---
