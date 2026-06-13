# Changelog

All notable changes to Mainstage are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While Mainstage is pre-`1.0.0`, breaking changes may land in minor releases; once
`1.0.0` ships, the language grammar and CLI surface follow SemVer.

## [Unreleased]

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
- **Editor tooling** — a `tower-lsp` language server (`mainstage lsp` /
  `mainstage-lsp`) providing diagnostics, completion, hover, signature help,
  navigation, and document symbols, plus a comment-preserving `mainstage format`
  formatter shared by the CLI and the editor.
- **Performance & stability** — parallel stage execution, an `mtime`/size change-
  detection fast path, a criterion benchmark harness, parser fuzzing, and graceful
  Ctrl-C / SIGTERM handling.
- **Continuous integration** — a GitHub Actions workflow running `cargo build`,
  `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, and a
  `mainstage format --check` gate across Linux, macOS, and Windows.
- **Release engineering** — a tag-triggered workflow that builds the `mainstage`
  and `mainstage-lsp` binaries for Linux (gnu + musl), macOS (x86_64 + arm64), and
  Windows, and attaches the archives with SHA-256 checksums to the GitHub Release.

[Unreleased]: https://github.com/ColtMcG1/mainstage/commits/main
