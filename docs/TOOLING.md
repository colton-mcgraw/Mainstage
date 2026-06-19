# Mainstage Editor Tooling

Mainstage ships two developer-experience tools that share the language core, so
editor behavior never diverges from the command line:

- a **Language Server** (`mainstage lsp`) for diagnostics, completion, hover,
  signature help, navigation, and formatting in any LSP-capable editor; and
- a **formatter** (`mainstage format`) for canonical, comment-preserving style,
  usable from the CLI, in CI, and from the editor.

For the language itself see [`GRAMMAR.md`](GRAMMAR.md); for modules see
[`MODULES.md`](MODULES.md).

---

## Table of Contents

1. [Language Server](#language-server)
   - [Features](#features)
   - [Launching the server](#launching-the-server)
   - [Editor setup](#editor-setup)
2. [Formatter](#formatter)
   - [`mainstage format`](#mainstage-format)
   - [Canonical style](#canonical-style)
   - [Formatting in CI](#formatting-in-ci)
   - [Editor formatting](#editor-formatting)

---

## Language Server

The language server is a thin protocol shell over `mainstage_core`: it runs the same
`parse` → `analyze_with` pipeline and the same [`ModuleRegistry`](MODULES.md) the CLI
builds, so completion, hover, diagnostics, and validation match `mainstage` exactly.
It speaks LSP over stdio.

### Features

| Capability | What it does |
|------------|--------------|
| **Diagnostics** | Parse and semantic errors as you type, published on open and change (debounced). Messages, ranges, and notes mirror the CLI; stale squiggles clear when the document becomes valid. |
| **Completion** | Module names inside `import "…"`; methods after `alias.` (inserted as call snippets from the method signature); `let` bindings, stage names, and `project.<field>` in expression positions. |
| **Hover** | Signature and return type for a module method; the resolved form of a module alias, `let` binding, stage name, or `project.<field>`. A stage also shows its `description:` and any `depends_on` ordering. |
| **Signature help** | The active parameter while typing inside a module call's `(…)`. |
| **Go-to-definition** | Jump to the declaration of a `let`, an import alias, or a `<stage>.outputs` reference. |
| **Find references** | All uses of a stage or `let` binding. |
| **Document highlight** | With the cursor on a `let` binding or stage, highlight its declaration (as a write) and every use (as reads) in the document. |
| **Document symbols** | An outline of pipelines, stages, and top-level `let` bindings; a stage's description and ordering appear as its detail. |
| **Formatting** | Whole-document formatting via the shared [formatter](#formatter). |

**Document sync** is full-document in V1 (the editor sends the entire buffer on each
change); incremental sync and range formatting are not yet implemented.

### Launching the server

Editors launch the server over stdio with:

```sh
mainstage lsp
```

The standalone `mainstage-lsp` binary is equivalent and is useful when the language
server ships separately from the CLI.

### Editor setup

#### VS Code

Install the **Mainstage** extension from the
[Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=colton-mcgraw.mainstage)
or [Open VSX](https://open-vsx.org/extension/colton-mcgraw/mainstage) (search for
"Mainstage", or run `code --install-extension colton-mcgraw.mainstage`). It registers the
`.ms` language and manages the language server for you.

The extension needs no configuration when a `mainstage` or `mainstage-lsp` binary is
installed. On activation it locates the server in this order:

1. the `mainstage.server.path` setting, if set;
2. `mainstage-lsp` or `mainstage` on your `PATH`;
3. `mainstage-lsp` or `mainstage` in a common install location (`~/.local/bin`,
   `~/.cargo/bin`, `/usr/local/bin`, `/opt/homebrew/bin`).

A `mainstage` binary is launched as `mainstage lsp`; a dedicated `mainstage-lsp`
binary is launched directly. If no binary is found, the extension links to the
[install instructions](../README.md#installation). Point `mainstage.server.path` at a
specific executable to override discovery.

The extension source lives in [`editors/vscode/`](../editors/vscode/); see its README
to build and run it from source.

#### Neovim (built-in LSP)

```lua
vim.filetype.add({ extension = { ms = "mainstage" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "mainstage",
  callback = function(args)
    vim.lsp.start({
      name = "mainstage",
      cmd = { "mainstage", "lsp" },
      root_dir = vim.fs.dirname(vim.fs.find({ "main.ms", ".git" }, { upward = true })[1]),
    })
  end,
})
```

#### Helix

Add a language entry to `~/.config/helix/languages.toml`:

```toml
[[language]]
name = "mainstage"
scope = "source.mainstage"
file-types = ["ms"]
comment-token = "//"
language-servers = ["mainstage"]

[language-server.mainstage]
command = "mainstage"
args = ["lsp"]
```

(Use `command = "mainstage-lsp"` with no `args` if you installed the standalone
server binary.)

#### Generic LSP client

Any editor with an LSP client works. Configure:

- **Command:** `mainstage lsp` (stdio transport — no port or socket).
- **Language id:** `mainstage`.
- **File match:** `*.ms`.

---

## Formatter

The formatter rewrites a script to a single canonical style. It is built on a
trivia-preserving syntax layer, so **comments are never dropped** — they are
re-attached to the nodes they belong to and emitted in place.

### `mainstage format`

```sh
mainstage format [FILES...] [--check] [--stdout]
```

| Invocation | Behavior |
|------------|----------|
| `mainstage format` | Format `main.ms` in place. |
| `mainstage format a.ms b.ms` | Format each listed file in place. |
| `mainstage format --check a.ms` | Write nothing; exit non-zero if any file is not already formatted (prints which). |
| `mainstage format --stdout a.ms` | Print the formatted result to stdout without writing. |

In-place formatting only rewrites files that actually change, and reports each one it
touches. A syntactically invalid file is reported as an error and leaves the file
untouched — the formatter never lays out a tree it cannot parse. `--stdout` and
`--check` are mutually exclusive.

### Canonical style

The style is deliberately simple and deterministic, which makes formatting a fixed
point — `format(format(x)) == format(x)`:

- four-space indentation;
- a single space around `=` and after `:` (no column alignment);
- one statement, field, or step per line;
- blank lines between top-level items are preserved (collapsed to one); blocks within
  a `stage` or `pipeline` (`steps`, `on_failure`, `on_success`) are separated by a
  blank line;
- trailing commas are dropped from `project` and field lists (kept between list
  elements);
- conditions are parenthesized only where operator precedence requires it
  (`or` binds looser than `and`, which binds looser than `!`).

```mainstage
// before
project{name:"app",version:git.tag(default:"0.0.0")}

// after
project {
    name: "app"
    version: git.tag(default: "0.0.0")
}
```

Leading comments stay above their node, end-of-line comments stay on the node's line,
and a dangling comment at end of file is kept after the last item.

### Formatting in CI

Run the formatter in `--check` mode alongside your tests so unformatted code fails the
build, the same way `cargo fmt --check` does for Rust:

```yaml
# .github/workflows/ci.yml (excerpt)
- name: Check formatting
  run: mainstage format --check **/*.ms

- name: Test
  run: cargo test --workspace
```

`--check` exits non-zero and lists every file that would be reformatted, so the job
fails with an actionable diff target.

### Editor formatting

The language server exposes the same engine through `textDocument/formatting`, so
"Format Document" in any LSP-capable editor produces identical output to
`mainstage format`. Range formatting is not yet supported; the whole document is
formatted as a single edit.
