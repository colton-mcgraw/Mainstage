# Mainstage

![Mainstage](./media/mainstage_logo_text.svg)

[![License](https://img.shields.io/badge/License-Source_Available-blue.svg)](LICENSE.md) [![GitHub issues](https://img.shields.io/github/issues/colton-mcgraw/mainstage)](https://github.com/colton-mcgraw/mainstage/issues) [![GitHub forks](https://img.shields.io/github/forks/colton-mcgraw/mainstage)](https://github.com/colton-mcgraw/mainstage/forks) [![GitHub stars](https://img.shields.io/github/stars/colton-mcgraw/mainstage)](https://github.com/colton-mcgraw/mainstage/stargazers)

> **Status:** Beta — usable today. The language, runtime, editor tooling, and
> cross-platform distribution are all in place; remaining work is polish and
> ecosystem growth. New here? Start with the [Getting Started](docs/GETTING_STARTED.md)
> guide. See the [roadmap](docs/ROADMAP.md) for what's next.

Mainstage is a declarative build and automation language. Scripts describe *what* to build and *how* stages relate to each other — the runtime determines execution order and skips stages whose inputs haven't changed.

Scripts use the `.ms` extension. A project declares stages (with `inputs`, `outputs`, and `steps`) and pipelines that group them into named entry points. Inter-stage dependencies are resolved automatically from `<stage>.outputs` references — no explicit `depends_on` needed.

## Example

```mainstage
import "env" as env;
import "git" as git;

project {
    name:    "my-app"
    version: git.tag()
}

let sources = glob("src/**/*.rs");
let out     = env.get("OUT_DIR", default: "dist");
let target  = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};

default pipeline dev {
    stages: [compile]
}

pipeline release {
    stages: [compile, lint, test, package]

    on_success {
        $ slack-notify "Released ${project.version}"
    }
}

stage compile {
    inputs:  sources
    outputs: ["target/${target}/release/my-app"]

    steps {
        $ cargo build --release --target ${target}
    }

    on_failure {
        delete "target/"
    }
}

stage lint {
    inputs:        sources
    allow_failure: true

    steps {
        $ cargo clippy
    }
}

stage test {
    inputs: sources

    steps {
        $ cargo test
    }
}

stage package {
    inputs:  [compile.outputs]
    outputs: ["${out}/${project.name}-${project.version}.tar.gz"]

    steps {
        mkdir "${out}/"
        write "${out}/VERSION" content: "${project.version}"
        $ tar -czf "${outputs[0]}" "${out}/"
    }
}
```

```sh
mainstage               # runs the default pipeline
mainstage run release   # runs the release pipeline
mainstage list          # lists all pipelines and their stages
```

## CLI

| Command | Description |
| --- | --- |
| `mainstage` | Run the `default pipeline`. Error if none is declared. |
| `mainstage run <name>` | Run a named pipeline. |
| `mainstage watch [name]` | Run the pipeline, then re-run it whenever its inputs change. |
| `mainstage list` | List all pipelines and their stages. |
| `mainstage params` | List declared build parameters and their effective values. |
| `mainstage modules` | List available modules and their method signatures. |
| `mainstage format [FILES...]` | Format scripts to canonical style (`--check` for CI, `--stdout` to preview). |
| `mainstage plugin new <name>` | Scaffold a working stdio plugin skeleton (`--lang python\|shell`). |
| `mainstage plugin check <path>` | Lint a plugin against the protocol before publishing. |
| `mainstage lsp` | Run the language server over stdio (for editor integration). |
| `mainstage parse <file>` | Print the parsed AST (debug tool). |
| `mainstage clean` | Clear the change-detection cache and output store. |
| `mainstage cache stats` | Show output-cache size and restore hit-rate. |
| `mainstage cache gc [--max-size SIZE]` | Prune unreferenced blobs; evict LRU to a size ceiling. |

`run`, `watch`, `list`, and `clean` (and the bare `mainstage`) read `main.ms` in the
current directory by default; pass `-f, --file <FILE>` to point at a different script.
Change detection persists per project in `.mainstage/cache.json` next to the script — a
stage is skipped when its `inputs` are unchanged and its declared `outputs` still exist.
A successful run also snapshots each stage's `outputs` into a content-addressed store under
`.mainstage/cache/`; if the inputs are unchanged but the outputs were deleted, they are
**restored** from the store instead of rebuilt. Override build parameters with `-D
<name>=<value>`.

### Output control

These global flags work with any command and may appear before or after it:

| Flag | Effect |
| --- | --- |
| `--dry-run` | Print the planned execution order (grouped into concurrency *waves*) and which stages would run or skip — without executing anything. |
| `-v, --verbose` | Print extra detail, including per-stage timings inline. |
| `-q, --quiet` | Suppress progress output; print only errors. |
| `--no-color` | Disable colored output (also honored via the `NO_COLOR` env var). |

Colored output is automatically disabled when stdout is not a terminal. Every run prints
a per-stage timing summary at the end, and errors are shown with a source snippet and a
caret pointing at the offending span.

### Capabilities

Side-effecting modules are denied by default and must be granted a capability before
they run:

| Module | Capability | Grant |
| --- | --- | --- |
| `shell` (run external commands) | `run` | `--allow-run` |
| `http` (network requests) | `net` | `--allow-net` |

Grant both with `--allow-all`, or declare them per-project in a `[permissions]` block
of `plugins.toml` next to the script:

```toml
[permissions]
run = true
net = false
```

A capability granted by *either* the flags or the manifest is in effect. The `time`
module reads the wall clock but is not gated.

## Installation

Each release publishes prebuilt `mainstage` and `mainstage-lsp` binaries for Linux,
macOS, and Windows.

**Install script** (Linux / macOS) — downloads the right binary for your platform and
verifies its checksum:

```sh
curl -fsSL https://raw.githubusercontent.com/colton-mcgraw/mainstage/main/install.sh | sh
```

Set `MAINSTAGE_VERSION` to pin a release or `MAINSTAGE_BIN_DIR` to change the install
location (default `~/.local/bin`).

**Cargo:**

```sh
cargo install mainstage
```

**Homebrew** (macOS / Linux):

```sh
brew install colton-mcgraw/tap/mainstage
```

**Windows** — Scoop or winget:

```sh
scoop install mainstage
winget install colton-mcgraw.Mainstage
```

**Docker** — the image's entry point is the CLI; mount your project at `/work`:

```sh
docker run --rm -v "$PWD:/work" ghcr.io/colton-mcgraw/mainstage run release
```

## Editor Support

Install the **Mainstage** extension for VS Code from the
[Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=colton-mcgraw.mainstage)
or [Open VSX](https://open-vsx.org/extension/colton-mcgraw/mainstage) for diagnostics,
completion, hover, signature help, navigation, and formatting. It auto-discovers the
`mainstage lsp` server, so no configuration is needed once the CLI is installed.

Other editors connect to the same language server over stdio — see
[Editor Tooling](docs/TOOLING.md) for Neovim, Helix, and generic LSP setup.

## Building from Source

Requires Rust stable (edition 2024).

```sh
git clone https://github.com/colton-mcgraw/mainstage.git
cd mainstage
cargo build --release
```

The CLI binary is at `target/release/mainstage`.

## Workspace

| Crate | Description |
| --- | --- |
| [`core`](core/) | Parser, AST, semantic analysis, and evaluator |
| [`cli`](cli/) | `mainstage` CLI binary |
| [`lsp`](lsp/) | `mainstage-lsp` Language Server |

## Documentation

The full documentation is also published as a browsable site at
**[colton-mcgraw.github.io/mainstage](https://colton-mcgraw.github.io/mainstage/)**.

- [Getting Started](docs/GETTING_STARTED.md) — install Mainstage and build your first pipeline
- [Examples Gallery](examples/) — runnable example projects beyond `main.ms`
- [Grammar Specification](docs/GRAMMAR.md) — full language syntax and semantics reference
- [Modules](docs/MODULES.md) — standard-library modules, capabilities, and the plugin protocol
- [Authoring Plugins](docs/PLUGINS.md) — scaffolding, the stdio protocol, naming, versioning, and publishing
- [Plugin Index](docs/PLUGIN_INDEX.md) — community plugins and runnable reference examples
- [Editor Tooling](docs/TOOLING.md) — language server features, editor setup, and the formatter
- [Benchmarks](docs/BENCHMARKS.md) — performance harness, fixtures, and baseline timings
- [Roadmap](docs/ROADMAP.md) — planned features and milestones

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for the workspace
layout, how to build and test, and the CI gates your change needs to pass.

1. Fork the repository and create a branch.
2. Make your changes with clear commit messages, tests, and docs.
3. Run the [CI gates](CONTRIBUTING.md#ci-gates) locally, then submit a pull request.

## License

[Mainstage Source-Available License](LICENSE.md)
