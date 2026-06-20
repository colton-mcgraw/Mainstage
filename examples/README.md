# Examples Gallery

Runnable Mainstage projects, smallest first. Each directory is a self-contained
project — `cd` into one and run `mainstage` (or `mainstage list` to see its
pipelines first).

| Example | What it shows |
| --- | --- |
| [`hello/`](hello/) | The smallest useful script: one `stage`, one `default pipeline`, `mkdir` + `write` steps, and `${project.*}` interpolation. |
| [`params/`](params/) | Typed, command-line-overridable build parameters (`param … = …`), shown with `-D name=value` overrides, `mainstage params`, and a `param` driving an `if/else`. |
| [`static-site/`](static-site/) | `glob` filesets, `for` loops, content-addressed copies via the `hash` module inside interpolation, and a stage that depends on other stages' `outputs`. |
| [`data-report/`](data-report/) | The read-only standard library — `fs`, `json`, `str`, `env` — plus an `if/else` expression driven by an `env(...)` condition. |
| [`multi-file/`](multi-file/) | `include` composition: a root script merges one `.ms` file per component into a single flat build graph, with cross-file `depends_on`/`outputs` references and a `glob` resolved against its included file's own directory. |
| [`plugins/`](plugins/) | External stdio plugins (`greet`, `wordcount`) that add modules without recompiling Mainstage. See [`docs/PLUGINS.md`](../docs/PLUGINS.md). |

The repository root also ships [`main.ms`](../main.ms) — a release-style Rust
build pipeline with `on_failure` / `on_success` hooks and `allow_failure`.

## Running an example

```sh
cd examples/hello
mainstage list     # show pipelines and their stages
mainstage          # run the default pipeline
```

Outputs land under each example's `dist/` directory. Change detection persists in
a local `.mainstage/cache.json`; `mainstage clean` clears it.

## New to Mainstage?

Start with the [Getting Started](../docs/GETTING_STARTED.md) guide, which walks
through these examples step by step, then see the
[Grammar](../docs/GRAMMAR.md) and [Modules](../docs/MODULES.md) references.
