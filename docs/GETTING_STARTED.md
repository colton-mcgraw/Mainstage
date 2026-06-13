# Getting Started

Mainstage is a declarative build and automation language. You describe *what* to
build as a set of **stages** — each with `inputs`, `outputs`, and `steps` — and group
them into named **pipelines**. The runtime works out the execution order from the
dependencies between stages and skips any stage whose inputs haven't changed.

This guide takes you from install to a working pipeline in a few minutes. For the
full language reference see [`GRAMMAR.md`](GRAMMAR.md); for the built-in modules see
[`MODULES.md`](MODULES.md).

---

## 1. Install

Pick whichever fits your platform — full options are in the
[README](../README.md#installation).

```sh
# Linux / macOS — downloads the right binary and verifies its checksum
curl -fsSL https://raw.githubusercontent.com/ColtMcG1/mainstage/main/install.sh | sh

# Cargo (any platform with Rust stable, edition 2024)
cargo install mainstage
```

Verify it:

```sh
mainstage --help
```

---

## 2. Your first script

Mainstage scripts use the `.ms` extension, and the CLI looks for `main.ms` in the
current directory by default. Create one:

```mainstage
// main.ms
project {
    name: "hello"
    version: "1.0.0"
}

default pipeline build {
    stages: [greet]
}

stage greet {
    outputs: ["dist/hello.txt"]

    steps {
        mkdir "dist/"
        write "dist/hello.txt" content: "Hello from ${project.name} v${project.version}!"
    }
}
```

Three pieces are doing the work:

- **`project`** declares metadata you can reference anywhere as `project.<field>`.
- **`pipeline`** names an entry point and lists the stages it runs. Marking one
  `default` lets you run it with a bare `mainstage`.
- **`stage`** is a unit of work. `outputs` is what it produces; `steps` is how. The
  `${...}` syntax interpolates expressions into strings.

Run it:

```sh
mainstage list     # show pipelines and their stages
mainstage          # run the default pipeline
cat dist/hello.txt # -> Hello from hello v1.0.0!
```

This is the [`examples/hello`](../examples/hello/) project — copy it and tinker.

---

## 3. Stages, inputs & dependencies

Stages rarely stand alone. A stage's `inputs` declare what it reads; referencing
another stage's `outputs` makes the runtime run them in the right order — no explicit
`depends_on` needed.

```mainstage
let sources = glob("src/**/*.rs");

stage compile {
    inputs: sources
    outputs: ["dist/app"]

    steps {
        $ cargo build --release
        copy "target/release/app" to "dist/app"
    }
}

stage package {
    inputs: [compile.outputs]               // depends on `compile`
    outputs: ["dist/app.tar.gz"]

    steps {
        $ tar -czf "dist/app.tar.gz" "dist/app"
    }
}
```

- **`glob(...)`** resolves a pattern into a *fileset* — a list of files with `.path`,
  `.name`, `.stem`, `.ext`, and `.dir` properties you can loop over.
- **`let`** binds a reusable value at the top level (evaluated once, in order).
- **`$`** runs a program directly (no shell): the line is tokenized into argv after
  interpolation, and the program is resolved on `PATH`.
- Because `package` reads `compile.outputs`, `compile` always runs first. Independent
  stages can run concurrently — see [`--jobs`](../README.md#cli).

### Change detection

After a run, Mainstage records each stage's input digest and output paths in
`.mainstage/cache.json`. On the next run a stage is **skipped** when its inputs are
unchanged *and* its declared outputs still exist. Force a full rebuild with
`mainstage clean`. (The `.mainstage/` directory is conventionally git-ignored.)

---

## 4. Steps you can use

Inside `steps { }` (and `on_failure` / `on_success`):

| Step | Purpose |
| --- | --- |
| `$ <program> <args...>` | Run an executable on `PATH` (no shell). |
| `copy <src> to <dest>` | Copy a file or directory (creates the destination dir). |
| `move <src> to <dest>` | Move a file or directory. |
| `mkdir <path>` | Create the full directory tree. |
| `delete <path>` | Remove a file/dir recursively (no-op if absent). |
| `write <path> content: <string>` | Write a string to a file. |
| `for <var> in <fileset> { ... }` | Loop, binding `<var>.path`, `.name`, … per file. |
| `if <condition> { ... } else { ... }` | Branch on a `platform` or `env(...)` condition. |

Inside a stage's steps, `inputs` (the resolved fileset) and `outputs` (the declared
output paths, indexable as `outputs[0]`) are in scope.

---

## 5. Modules & expressions

`import` brings a built-in module into scope; call its methods through the alias.
Modules are validated at analysis time — a bad import, method, or argument type is an
error *before* anything runs.

```mainstage
import "env" as env;
import "git" as git;
import "str" as str;

let out     = env.get("OUT_DIR", default: "dist");
let version = git.tag(default: "0.0.0");
let slug    = str.lower("My-App");          // -> "my-app"

let target = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};
```

The standard library includes `env`, `git`, `str`, `path`, `hash`, `fs`, and `json`
(pure / read-only), plus `shell`, `http`, and `time` (the first two gated behind
`--allow-run` / `--allow-net`). The full list with signatures is in
[`MODULES.md`](MODULES.md), or run `mainstage modules`. Need something custom? Write
an [external plugin](PLUGINS.md) — no recompile required.

---

## 6. Failure handling

Stages can react to failure, and pipelines can run hooks at the end:

```mainstage
pipeline release {
    stages: [compile, lint, test, package]

    on_success {
        $ slack-notify "Released ${project.version}"
    }
}

stage compile {
    inputs: sources
    outputs: ["dist/app"]

    steps {
        $ cargo build --release
    }

    on_failure {
        delete "dist/"          // clean up a partial build
    }
}

stage lint {
    inputs: sources
    allow_failure: true         // a failure here won't stop the pipeline

    steps {
        $ cargo clippy
    }
}
```

When a stage fails, every stage that depends on its outputs is cancelled — unless the
failed stage set `allow_failure: true`, which treats it as succeeded. A pipeline's
`on_failure` block binds the `failed_stage` variable.

---

## 7. Explore the gallery

The [`examples/`](../examples/) directory has runnable projects beyond `main.ms`:

- [`hello/`](../examples/hello/) — the script from this guide.
- [`static-site/`](../examples/static-site/) — `glob`, `for` loops, `hash`-based
  content addressing, and inter-stage dependencies.
- [`data-report/`](../examples/data-report/) — the `fs` / `json` / `str` / `env`
  standard library and an `if/else` expression.
- [`plugins/`](../examples/plugins/) — external stdio plugins.

`cd` into any of them and run `mainstage`.

---

## Next steps

- **Language reference:** [`GRAMMAR.md`](GRAMMAR.md)
- **Modules & plugins:** [`MODULES.md`](MODULES.md), [`PLUGINS.md`](PLUGINS.md)
- **Editor support:** [`TOOLING.md`](TOOLING.md) — diagnostics, completion, and
  formatting via the language server.
- **Contributing:** [`CONTRIBUTING.md`](../CONTRIBUTING.md)
