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

**None — the language server is bundled.** The extension ships a per-platform
`mainstage-lsp` binary inside the VSIX, so it works out of the box with no CLI
install and no configuration. The Marketplace and Open VSX serve the build that
matches your OS and architecture (and, in a remote container, WSL, or SSH
session, the remote's OS and architecture).

To use a different server — a development build, or a system install — set
`mainstage.server.path` to a `mainstage` or `mainstage-lsp` executable. A
`mainstage` binary is launched as `mainstage lsp`; a `mainstage-lsp` binary is
launched directly.

If the extension did not include a binary for your platform (e.g. a generic
VSIX installed manually), it shows a notification linking to the
[install instructions](https://github.com/colton-mcgraw/mainstage#installation)
and the `mainstage.server.path` setting.

## Settings

| Setting | Description |
| --- | --- |
| `mainstage.server.path` | Absolute path to a `mainstage` or `mainstage-lsp` executable to use instead of the bundled server. Leave empty to use the bundled server. |
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

### Packaging with the bundled server

Each published VSIX bundles a `mainstage-lsp` binary under `server/`. To build a
package locally, compile the server, copy it in, then package:

```sh
cargo build -p mainstage_lsp --bin mainstage-lsp --release
npm run copy-server                 # copies target/release/mainstage-lsp → server/
npx vsce package --target $(node -p "process.platform")-... --out mainstage.vsix
```

`copy-server` reads `MAINSTAGE_PROFILE` (default `release`) and, for cross
builds, `MAINSTAGE_TARGET` (a cargo target triple → reads from
`target/<triple>/<profile>/`). The CI workflow (`.github/workflows/vscode-extension.yml`)
builds one platform-specific VSIX per supported target this way.

### The test suite

The suite has two parts:

- **Resolver unit tests** (`src/test/resolver.test.ts`) — cover the bundled-vs.
  -configured server resolution and the `mainstage` vs `mainstage-lsp` launch
  logic with an injected host, so no real binary or `vscode` runtime is needed.
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
