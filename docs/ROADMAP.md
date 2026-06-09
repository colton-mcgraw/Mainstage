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
