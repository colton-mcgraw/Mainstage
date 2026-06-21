# Changelog

All notable changes to Mainstage are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Mainstage is early and its user base is small; breaking changes may still land in
minor releases while the language grammar and CLI surface stabilize.

## [Unreleased]

## [1.1.0] - 2026-06-21

### Added

- **Build ordering & caching control** — explicit `depends_on` stage ordering for
  edges the file graph can't infer, `always_run` / `run_once` caching knobs that
  replace sentinel-file hacks, and per-output incremental change detection so
  editing one source rebuilds one object instead of the whole stage.
- **Parameterized & reusable builds** — a `matrix { <dim>: [<values>] }` stage
  expansion that lowers to concrete per-variant stages, top-level `template` /
  `use` blocks that inline a shared step sequence, and typed, declared CLI
  `param`s overridable with `-D name=value` / `--param` (listed by `mainstage
  params`).
- **Richer steps & conditions** — `try { }` blocks for step-level failure
  tolerance, `workdir "<path>" { … }` and `with_env { … } { … }` execution-context
  blocks, `log` / `fail` diagnostic and control-flow steps, block-scoped local
  `let` bindings, force-overwrite `copy`, and expression-based conditions
  (`==`, `!=`, `contains`, `in`, and an emptiness predicate).
- **Test harness** — a `test` stage flavor with `expect` / `assert` steps and a
  matcher set (`contains`, `not_contains`, `starts_with`, `ends_with`, `matches`),
  reporting pass/fail tallies through the buffered, atomic per-stage reporter.
- **Multi-file composition** — `include "<path>";` lexically merges component
  `.ms` files into one flat build graph before analysis, with include-cycle
  detection, deterministic ordering, de-duplication, a flat cross-file namespace,
  and cross-file go-to-definition.
- **Content-addressed output cache** — successful outputs are recorded by content
  hash and stored under `.mainstage/cache/`, then *restored* (not just skipped)
  after a `clean`, a branch switch, or a fresh checkout; `mainstage cache gc` and
  `mainstage cache stats` prune and report on the store.
- **Build introspection** — `mainstage query` prints the dependency graph and its
  reverse edges (with DOT / JSON export), `mainstage explain <stage>` reports why a
  stage ran or was skipped on the last run, and `mainstage profile` / `--profile`
  shows per-stage timings and the critical path.
- **Hermeticity & reproducibility** — declared tool `requires { … }` with optional
  version constraints, per-stage `hermetic: true` environment isolation,
  `--check-reproducible` double-build output diffing, and an input-completeness
  audit that warns when a stage reads outside its declared `inputs`.
- **Run status & feedback** — the runner records each run to a
  `.mainstage/status.json` file (per-stage status, timings, and the live last
  output line); `mainstage ui` shows each stage's elapsed time and latest output
  (`running… : <line>`, `cached`, `restored`, `failed : <error>`); a new
  `mainstage status` command renders the last run's table; and the VS Code
  extension watches the file to show the running stage in its status bar
  (`mainstage.showStatusBar`).
- **Discovery & ergonomics** — a `fs.find_first([...])` helper for portable
  firmware/tool path discovery and an optional stage `description:` field surfaced
  by `mainstage list`, LSP document symbols, and hover.

### Changed

- **Editor extension** — the VS Code client now bundles the `mainstage-lsp` server
  binary (replacing runtime auto-discovery) for zero-config setup, with improved
  stability and logging when running in remote containers or WSL.

## [1.0.0] - 2026-06-13

### Added

- **Language & runtime** — the full V1 grammar (`import` / `let` / `project` /
  `stage` / `pipeline`), the expression evaluator, the step executor, the
  dependency-ordered pipeline runner with failure propagation, and content-hash
  change detection with a persistent `.mainstage/cache.json`.
- **Module system** — a trait-based `ModuleRegistry` with the `env`, `git`, `str`,
  `path`, `hash`, `fs`, `json`, `shell`, `http`, and `time` standard-library
  modules, semantic call validation, a capability model (`--allow-run` /
  `--allow-net`), and external subprocess plugins over a JSON/stdio protocol.
- **CLI** — `run`, `watch`, `list`, `modules`, `format`, `lsp`, `parse`, and
  `clean`, with `--dry-run`, `--jobs`, TTY-aware colored output, and a per-stage
  timing summary.
- **Plugin ecosystem** — `mainstage plugin new` scaffolds a working stdio plugin
  (Python or POSIX-shell) and `mainstage plugin check` lints a plugin against the
  protocol before publishing, alongside an authoring/publishing guide
  (`docs/PLUGINS.md`), a community index (`docs/PLUGIN_INDEX.md`), and runnable
  reference plugins under `examples/plugins/`.
- **Editor tooling** — a `tower-lsp` language server (`mainstage lsp` /
  `mainstage-lsp`) providing diagnostics, completion, hover, signature help,
  navigation, document highlight, and document symbols, plus a comment-preserving
  `mainstage format` formatter shared by the CLI and the editor.
- **Performance & stability** — parallel stage execution, an `mtime`/size change-
  detection fast path, a criterion benchmark harness, parser fuzzing, and graceful
  Ctrl-C / SIGTERM handling.
- **Continuous integration** — a GitHub Actions workflow running `cargo build`,
  `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, and a
  `mainstage format --check` gate across Linux, macOS, and Windows.
- **Release engineering** — a tag-triggered workflow that builds the `mainstage`
  and `mainstage-lsp` binaries for Linux (gnu + musl), macOS (x86_64 + arm64), and
  Windows, and attaches the archives with SHA-256 checksums to the GitHub Release.
- **Distribution** — a `curl | sh` install script (with checksum verification),
  crates.io publishing (`cargo install mainstage`), a Homebrew formula, Scoop and
  winget manifests, and a Docker image with the CLI as its entry point.
- **Editor extension** — a VS Code client (`editors/vscode/`) with syntax
  highlighting and zero-config language-server discovery, a tag-triggered workflow
  publishing it to the Visual Studio Marketplace and Open VSX, and documented LSP
  setup for Neovim and Helix.
- **Docs & onboarding** — a [Getting Started](docs/GETTING_STARTED.md) guide, a
  runnable [examples gallery](examples/) (`hello`, `static-site`, `data-report`)
  beyond `main.ms`, an mdBook documentation site rendering `docs/` (published to
  GitHub Pages), and a [`CONTRIBUTING.md`](CONTRIBUTING.md) covering the workspace
  layout, tests, and CI gates.

[Unreleased]: https://github.com/colton-mcgraw/mainstage/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/colton-mcgraw/mainstage/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/colton-mcgraw/mainstage/releases/tag/v1.0.0
