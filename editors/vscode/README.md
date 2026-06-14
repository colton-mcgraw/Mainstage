# Mainstage for VS Code

Language support for [Mainstage](https://github.com/colton-mcgraw/mainstage) build
scripts (`.ms`). This extension is a thin client for the **Mainstage language
server** (`mainstage lsp`), so every editor feature matches the `mainstage` CLI
exactly — the same parser, analyzer, and module registry power both.

## Features

- **Diagnostics** — parse and semantic errors as you type, with the analyzer's
  messages, ranges, and notes.
- **Completion** — module names inside `import "…"`, methods after `alias.`
  (inserted as call snippets), and `let` bindings, stage names, and
  `project.<field>` in expression positions.
- **Hover** — signature and return type for module methods; the resolved form
  of an alias, `let` binding, stage name, or `project.<field>`.
- **Signature help** — the active parameter while typing inside a call's `(…)`.
- **Go-to-definition** and **find references** for `let` bindings, import
  aliases, and `<stage>.outputs`.
- **Document highlight** — put the cursor on a `let` binding or stage to
  highlight its declaration and every use across the document.
- **Document symbols** — an outline of pipelines, stages, and top-level `let`
  bindings.
- **Formatting** — "Format Document" runs the same engine as
  `mainstage format`, so editor and CLI output are identical.
- **Syntax highlighting** for `.ms` files.

## Requirements

The extension needs the Mainstage language server. It works with **no manual
configuration** when a `mainstage` or `mainstage-lsp` binary is installed and
discoverable. On activation it looks, in order, for:

1. the `mainstage.server.path` setting, if set;
2. `mainstage-lsp` or `mainstage` on your `PATH`;
3. `mainstage-lsp` or `mainstage` in a common install location
   (`~/.local/bin`, `~/.cargo/bin`, `/usr/local/bin`, `/opt/homebrew/bin`).

A `mainstage` binary is launched as `mainstage lsp`; a dedicated `mainstage-lsp`
binary is launched directly.

Install the CLI with any method from the
[project README](https://github.com/colton-mcgraw/mainstage#installation), e.g.:

```sh
curl -fsSL https://raw.githubusercontent.com/colton-mcgraw/mainstage/main/install.sh | sh
# or
cargo install mainstage
```

If no binary is found, the extension shows a notification linking to the install
instructions.

## Settings

| Setting | Description |
| --- | --- |
| `mainstage.server.path` | Absolute path to the `mainstage` or `mainstage-lsp` executable. Leave empty to auto-discover. |
| `mainstage.server.arguments` | Extra arguments passed to the server after the `lsp` subcommand. |
| `mainstage.trace.server` | Trace the LSP traffic (`off`, `messages`, `verbose`) for debugging. |

## Commands

- **Mainstage: Restart Language Server**
- **Mainstage: Show Language Server Output**

## Developing & testing

```sh
npm ci          # install dependencies
npm run compile # type-check and emit to out/
npm test        # run the test suite
```

The suite has two parts:

- **Resolver unit tests** (`src/test/resolver.test.ts`) — cover server
  discovery and the `mainstage` vs `mainstage-lsp` launch logic with an
  injected host, so no real binary or `vscode` runtime is needed.
- **Server integration tests** (`src/test/server.test.ts`) — spawn the real
  language server and drive it over stdio with the same
  `vscode-languageserver-protocol` stack the client uses, asserting that
  `initialize`, completion, hover, and document highlight return what the
  editor expects. They build on the workspace's debug binary at
  `target/debug/mainstage-lsp`; set `MAINSTAGE_LSP_BIN` to point elsewhere.
  When no server binary is found, this part is skipped (the unit tests still
  run), so build it first to exercise the full suite:

  ```sh
  cargo build -p mainstage_lsp --bin mainstage-lsp
  npm test
  ```

## License

[Mainstage Source-Available License](LICENSE.md).
