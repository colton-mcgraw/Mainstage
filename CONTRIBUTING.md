# Contributing to Mainstage

Thanks for your interest in improving Mainstage! This guide covers the workspace
layout, how to build and test, and the CI gates your change needs to pass.

By contributing you agree your contributions are licensed under the
[Mainstage Source-Available License](LICENSE.md).

---

## Prerequisites

- **Rust** stable, **edition 2024** (`rustup toolchain install stable`).
- **Node.js 20+** — only if you touch the VS Code extension under `editors/vscode`.

```sh
git clone https://github.com/colton-mcgraw/mainstage.git
cd mainstage
cargo build --workspace
```

---

## Workspace layout

Mainstage is a Cargo workspace of three crates plus a fuzz target:

| Path | Crate | What lives here |
| --- | --- | --- |
| [`core/`](core/) | `mainstage_core` | The language: lexer/parser (`parser.rs`, `grammar.pest`), AST (`ast.rs`), semantic analysis (`sema.rs`), evaluator (`eval.rs`), step executor (`executor.rs`), pipeline runner (`runner.rs`), change-detection cache (`cache.rs`), formatter (`format.rs`, `trivia.rs`), and the module system (`modules/`). |
| [`cli/`](cli/) | `mainstage` | The `mainstage` binary — argument parsing, command wiring (`commands.rs`), and plugin scaffolding (`scaffold.rs`). |
| [`lsp/`](lsp/) | `mainstage_lsp` | The `mainstage-lsp` language server (`tower-lsp` over stdio): diagnostics, completion, hover, signature help, navigation, and formatting, all reusing `core`. |
| [`fuzz/`](fuzz/) | — | `cargo-fuzz` targets that assert the parser/pipeline never panics on arbitrary input. |

Supporting directories: [`docs/`](docs/) (the references rendered by the docs site),
[`examples/`](examples/) (the runnable gallery), [`tests/`](tests/) (example `.ms`
scripts exercised by the integration tests), [`editors/vscode/`](editors/vscode/) (the
extension), and [`packaging/`](packaging/) (Homebrew / Scoop / winget manifests).

**Single source of truth:** the CLI and the language server both call `core`'s
`parse` → `analyze_with(registry)` pipeline with the same `ModuleRegistry`. Language
behavior should never diverge between them — put language logic in `core`, not in a
front end.

---

## Building & running

```sh
cargo build                      # debug build of the whole workspace
cargo run --bin mainstage -- list           # run the CLI
cargo run --bin mainstage -- run release    # run a pipeline
./target/debug/mainstage modules            # after a build
```

---

## Testing

Most tests live in `core/tests/` (integration) and as `#[test]` modules inside the
source. Run the whole suite:

```sh
cargo test --workspace
```

Notable suites in `core/tests/`: `parser.rs`, `sema.rs`, `eval.rs`, `runner.rs`,
`change_detection.rs`, `format.rs`, `trivia.rs`, `examples.rs` (drives the `.ms`
files under `tests/`), plus `fuzz.rs` and `stress.rs` for robustness. The LSP
client/server contract is covered by `lsp/tests/server.rs`.

If you add or change language behavior, add a script under `tests/` or `examples/` and
assert on it — `examples.rs` shows the parse/analyze/eval pattern. New `.ms` files must
be in canonical format (see below).

### VS Code extension

```sh
cd editors/vscode
npm ci
npm test            # builds the server first; set MAINSTAGE_LSP_BIN to a prebuilt one
```

---

## CI gates

Every push to `main` and every pull request runs
[`.github/workflows/ci.yml`](.github/workflows/ci.yml). Reproduce it locally before
opening a PR:

```sh
cargo fmt --all --check                                   # 1. Rust formatting
cargo build --workspace --all-targets --locked            # 2. Build (all OSes in CI)
cargo test --workspace --locked                           # 3. Tests
cargo clippy --workspace --all-targets --locked -- -D warnings   # 4. Lint (warnings = errors)
cargo run --bin mainstage -- format --check \
    main.ms tests/stdlib.ms tests/validation_errors.ms tests/plugin/main.ms   # 5. Script formatting
```

In CI, steps 2–5 run on **Linux, macOS, and Windows**; `cargo fmt --check` runs once.
The extension job builds the language server and runs the extension's integration
tests against it.

Two distinct formatters are in play:

- **`cargo fmt`** formats Rust source per [`rustfmt.toml`](rustfmt.toml).
- **`mainstage format`** formats `.ms` scripts to canonical style. Run
  `mainstage format <file>` to auto-fix, or `--check` to verify. Every committed `.ms`
  script must pass `--check`; if you add one, include it in the CI format list above.

---

## Submitting changes

1. Fork the repository and create a topic branch.
2. Make focused commits with clear messages; keep unrelated changes separate.
3. Add or update tests, and update the relevant `docs/` pages.
4. Run the [CI gates](#ci-gates) locally — they must all pass.
5. Open a pull request describing the change and why.

For larger or design-level changes, consider opening an issue first to discuss the
approach. The [Roadmap](docs/ROADMAP.md) shows where the project is headed.
