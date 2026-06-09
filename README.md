# Mainstage

![Mainstage](./media/mainstage_logo_text.svg)

[![License](https://img.shields.io/badge/License-Source_Available-blue.svg)](LICENSE.md) [![GitHub issues](https://img.shields.io/github/issues/ColtMcG1/mainstage)](https://github.com/ColtMcG1/mainstage/issues) [![GitHub forks](https://img.shields.io/github/forks/ColtMcG1/mainstage)](https://github.com/ColtMcG1/mainstage/forks) [![GitHub stars](https://img.shields.io/github/stars/ColtMcG1/mainstage)](https://github.com/ColtMcG1/mainstage/stargazers)

> **Status:** Early development — not yet usable. See the [roadmap](docs/ROADMAP.md).

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
| `mainstage list` | List all pipelines and their stages. |
| `mainstage parse <file>` | Print the parsed AST (debug tool). |
| `mainstage clean` | Clear the change-detection cache. |

## Building from Source

Requires Rust stable (edition 2024).

```sh
git clone https://github.com/ColtMcG1/mainstage.git
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

- [Grammar Specification](docs/GRAMMAR.md) — full language syntax and semantics reference
- [Roadmap](docs/ROADMAP.md) — planned features and milestones

## Contributing

1. Fork the repository and create a branch.
2. Make your changes with clear commit messages.
3. Submit a pull request.

## License

[MIT](LICENSE.md)
