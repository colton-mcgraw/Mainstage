# Mainstage

**Mainstage** is a declarative build and automation language. Scripts describe *what*
to build and *how* stages relate to each other — the runtime determines execution
order and skips stages whose inputs haven't changed.

Scripts use the `.ms` extension. A project declares **stages** (with `inputs`,
`outputs`, and `steps`) and **pipelines** that group them into named entry points.
Inter-stage dependencies are resolved automatically from `<stage>.outputs`
references — no explicit `depends_on` needed.

```mainstage
project {
    name: "my-app"
    version: "1.0.0"
}

default pipeline build {
    stages: [compile, package]
}

stage compile {
    inputs: glob("src/**/*.rs")
    outputs: ["dist/app"]

    steps {
        $ cargo build --release
        copy "target/release/app" to "dist/app"
    }
}

stage package {
    inputs: [compile.outputs]
    outputs: ["dist/app.tar.gz"]

    steps {
        $ tar -czf "dist/app.tar.gz" "dist/app"
    }
}
```

## Where to start

- **[Getting Started](GETTING_STARTED.md)** — install Mainstage and build your first
  pipeline.
- **[Grammar Reference](GRAMMAR.md)** — the complete language syntax and semantics.
- **[Modules](MODULES.md)** — the standard library, the capability model, and the
  plugin protocol.
- **[Authoring Plugins](PLUGINS.md)** — extend Mainstage with your own modules.
- **[Editor Tooling](TOOLING.md)** — the language server and formatter.
- **[Benchmarks](BENCHMARKS.md)** — the performance harness and baseline numbers.
- **[Roadmap](ROADMAP.md)** — where the project is headed.

This site renders the references in the repository's
[`docs/`](https://github.com/ColtMcG1/mainstage/tree/main/docs) directory. The source,
the runnable [examples gallery](https://github.com/ColtMcG1/mainstage/tree/main/examples),
and the [contributing guide](https://github.com/ColtMcG1/mainstage/blob/main/CONTRIBUTING.md)
all live on [GitHub](https://github.com/ColtMcG1/mainstage).
