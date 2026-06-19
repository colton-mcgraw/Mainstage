# Mainstage Grammar Specification

This document is the authoritative reference for the Mainstage scripting language grammar. It covers every construct, its syntax, its semantics, and how the runtime interprets it.

Mainstage is a declarative build and automation language. Scripts describe *what* to build and *how* stages relate to each other — the runtime figures out *in what order* to run them and *whether to skip* them based on change detection.

---

## Table of Contents

1. [File Structure](#file-structure)
2. [Comments](#comments)
3. [Top-Level Constructs](#top-level-constructs)
4. [Expressions](#expressions)
5. [Conditions](#conditions)
6. [Steps](#steps)
7. [Failure Handling](#failure-handling)
8. [Multiple Pipelines & CLI](#multiple-pipelines--cli)
9. [Built-in Variables & Properties](#built-in-variables--properties)
10. [Formal Grammar (EBNF)](#formal-grammar-ebnf)
11. [Complete Example](#complete-example)

---

## File Structure

Mainstage scripts use the `.ms` extension. A script is a sequence of top-level items in any order, with one exception: `import` declarations must appear before any other item that references the imported module.

Top-level items are:

- `import` — bring a module into scope
- `let` — declare a named value
- `project` — project metadata
- `stage` — a build stage
- `pipeline` / `default pipeline` — a named entry point

---

## Comments

Line comments begin with `//` and extend to the end of the line. There are no block comments.

```mainstage
// This is a comment
let out = "dist"  // inline comment
```

---

## Top-Level Constructs

### `import`

Brings a module into scope under an alias. Modules provide functions callable in expressions.

```text
import "<module-name>" as <alias>;
```

```mainstage
import "env" as env;
import "git" as git;
```

The module name is a **string literal**, so it is not constrained by identifier rules.
This is what lets external plugins use namespaced names like `"acme/lint"` without any
lexer or grammar change — the `/` lives inside the quoted string, never in the token
stream:

```mainstage
import "acme/lint" as lint;
```

The alias, by contrast, is a plain identifier and must be a valid `ident`. See
[`MODULES.md`](MODULES.md) for the full list of built-in modules and the plugin
mechanism.

The semicolon is required on `import` and `let` declarations. It is optional on block constructs (`project`, `stage`, `pipeline`).

---

### `let`

Declares a named value. Values are immutable. The right-hand side is any expression, including conditional expressions.

```text
let <name> = <expr>;
```

```mainstage
let version = "1.0.0";
let out     = env.get("OUT_DIR", default: "dist");
let target  = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};
```

`let` bindings are evaluated once at script load time, in declaration order. Forward references to bindings not yet declared are not allowed.

---

### `project`

Declares metadata about the project. Fields are key-value pairs. Commas between fields are optional.

```text
project {
    <field>: <expr>
    ...
}
```

| Field         | Type     | Required | Description                        |
|---------------|----------|----------|------------------------------------|
| `name`        | `string` | Yes      | Project name                       |
| `version`     | `string` | No       | Semantic version string            |
| `description` | `string` | No       | Short description                  |
| `author`      | `string` | No       | Author name and optional contact   |

```mainstage
project {
    name:        "my-app"
    version:     "1.0.0"
    description: "A cross-platform build example"
    author:      "Colt McGraw"
}
```

Project fields are accessible anywhere in the script via `project.<field>` (e.g. `project.version`).

---

### `stage`

Defines a build stage: what files it consumes, what it produces, and what steps to run.

```text
stage <name> {
    inputs:        <expr>
    outputs:       <expr>
    allow_failure: <bool>

    steps {
        <step>
        ...
    }

    on_failure {
        <step>
        ...
    }
}
```

| Field           | Type              | Required | Description                                               |
|-----------------|-------------------|----------|-----------------------------------------------------------|
| `inputs`        | `fileset` / `list`| No       | Files this stage consumes. Used for change detection.     |
| `outputs`       | `list`            | No       | Paths this stage produces. Used for change detection.     |
| `depends_on`    | stage-name list   | No       | Explicit ordering edges to other stages (see below).      |
| `matrix`        | block             | No       | Expand the stage into one variant per combination of values (see below). |
| `allow_failure` | `bool`            | No       | If `true`, pipeline does not stop on stage failure.       |
| `always_run`    | `bool`            | No       | If `true`, the stage runs every invocation (see below).   |
| `run_once`      | `bool`            | No       | If `true`, the stage's success is cached without `outputs` (see below). |
| `steps`         | block             | No       | Ordered steps to execute.                                 |
| `on_failure`    | block             | No       | Steps to run if this stage fails. Always runs on failure. |

A stage with no `steps` is valid — it acts as a grouping node in the dependency graph.

**Change detection:** Before running a stage, the runtime hashes all `inputs`. If the hashes match the previous run and all `outputs` exist, the stage is skipped. A stage with `inputs` but no `outputs` is skipped purely on its inputs being unchanged. A stage with **neither** `inputs` nor `outputs` has nothing to compare, so by default it runs on every invocation; `always_run` and `run_once` make that behavior explicit and adjustable.

**`always_run`:** Forces the stage to run on every invocation, bypassing change detection even when it has unchanged `inputs` and present `outputs`. This is the explicit form of an *action* stage — booting an emulator, deploying, running a server — that must never be treated as cached. Prefer it over the older idiom of declaring an output path the steps never create.

```mainstage
stage run {
    inputs:     ["build/app.efi"]
    always_run: true            // an action, not a cached artifact — never skipped
    steps { $ qemu-system-x86_64 -kernel build/app.efi }
}
```

**`run_once`:** Records the stage's success in the cache even when it declares **no** `outputs`, so a side-effecting setup stage runs once and is skipped thereafter. It is the complement of `always_run`: instead of "always run", it means "run, then remember". The stamp is invalidated when the stage's `inputs` change (if it has any) or when the cache is cleared with `mainstage clean`.

```mainstage
stage initialize {
    run_once: true              // install the toolchain once; skip on later runs
    steps { $ ./scripts/install-toolchain.sh }
}
```

`always_run` and `run_once` are mutually exclusive — setting both on one stage is a semantic error.

**Dependency resolution:** If a stage's `inputs` references another stage's `outputs` (e.g. `compile.outputs`), the runtime automatically runs that stage first. No explicit `depends_on` is needed for file-based dependencies.

**Explicit ordering (`depends_on`):** When one stage must run after another but shares no file artifact with it — a side-effecting setup stage, or a "run after build" relationship — declare the edge explicitly. `depends_on` takes a bracketed list of stage names:

```mainstage
stage initialize {
    steps { $ ./scripts/install-toolchain.sh }
}

stage build {
    inputs:     glob("src/**")
    depends_on: [initialize]   // run after `initialize`, even with no shared files
    steps { $ make }
}
```

These edges are merged with the inferred `<stage>.outputs` edges into a single dependency graph, so they participate identically in ordering, parallel scheduling, and failure propagation. A `depends_on` edge only orders stages **within the same pipeline** — like inferred edges, a reference to a stage not listed in the running pipeline is ignored, not auto-added. Referencing an unknown stage, depending on yourself, or forming a dependency cycle (across the combined `inputs`/`outputs` + `depends_on` graph) is a semantic error reported with a source span.

**Build matrix (`matrix`):** A `matrix` block expands one authored stage into several concrete stages — one per combination of dimension values — so a multi-target build (e.g. per-architecture) lives in a single definition instead of copy-pasted stages. Each dimension is a name and a list of string values:

```text
matrix {
    <dim>: [<value>, ...]
    ...
}
```

```mainstage
stage bundle {
    matrix {
        arch: ["x86_64", "aarch64"]
    }
    outputs: ["dist/app-${arch}.tar.gz"]

    steps {
        $ cargo build --release --target ${arch}-unknown-linux-gnu
        $ tar -czf "dist/app-${arch}.tar.gz" dist/
    }
}
```

The expansion happens **before** semantic analysis, so the dependency graph, change detection, and the parallel scheduler only ever see ordinary stages.

- **Generated names.** Each variant is named `<stage>[<value>]`, e.g. `bundle[x86_64]` and `bundle[aarch64]`. With multiple dimensions the suffix joins the values in declaration order: `kernel { matrix { arch: ["x64"], mode: ["debug", "release"] } }` produces `kernel[x64,debug]` and `kernel[x64,release]` (the cartesian product).
- **The active value is a built-in.** Inside the stage, each dimension name resolves to its value as a built-in string variable, alongside `platform` — usable in `inputs`, `outputs`, interpolations (`${arch}`), and `$` command lines.
- **Referencing variants.** The bracketed names are never written by hand. Instead, reference the **base** name and it fans out to every variant: a base name in a pipeline's `stages:` list or another stage's `depends_on:` runs (or waits on) all variants, and `<base>.outputs` resolves to the combined outputs of every variant. The generated names appear in `mainstage list` and `--dry-run` output.
- **Validation.** An empty dimension (`arch: []`), a repeated dimension or value, a dimension that shadows a built-in (`platform`, `inputs`, `outputs`, `failed_stage`), or two variants resolving to the same generated name are semantic errors reported with a source span.

```mainstage
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
```

---

### `pipeline` / `default pipeline`

Declares a named entry point into the build graph. A pipeline selects which stages participate and in what logical order.

```text
[default] pipeline <name> {
    input:   <expr>
    stages:  <list-expr>

    on_failure {
        <step>
        ...
    }

    on_success {
        <step>
        ...
    }
}
```

| Field        | Type        | Required | Description                                               |
|--------------|-------------|----------|-----------------------------------------------------------|
| `input`      | `fileset`   | No       | The root file set that triggers the pipeline.             |
| `stages`     | `list`      | Yes      | Ordered list of stage names included in this pipeline.    |
| `on_failure` | block       | No       | Steps to run if any stage in the pipeline fails.          |
| `on_success` | block       | No       | Steps to run after all stages complete successfully.      |

The `default` modifier designates the pipeline that runs when `mainstage` is invoked with no arguments. Only one `default pipeline` may exist per script — a second declaration is a parse error.

```mainstage
default pipeline dev {
    input:  glob("src/**")
    stages: [compile]
}

pipeline release {
    input:  glob("src/**")
    stages: [compile, lint, test, package, deploy]

    on_failure {
        $ slack-notify "Release failed at ${failed_stage}"
    }

    on_success {
        $ slack-notify "Released ${project.version}"
    }
}
```

---

## Expressions

Expressions appear on the right-hand side of `let` bindings, field values, and step arguments.

### String Literals

Enclosed in double quotes. May span multiple lines.

```mainstage
let greeting = "hello"
```

### String Interpolation

Embed any expression inside `${}` within a string. The result is coerced to a string.

```mainstage
let path = "dist/${project.name}-${project.version}.tar.gz"
```

Note: `${}` is interpolation (expression inside braces). `$` alone (no braces, at the start of a step) is the exec operator — the two are distinct and unambiguous by position.

### Integer Literals

A signed whole number, stored as a 64-bit integer (`i64`). An optional leading `-`
denotes a negative value. Integers may be used in `let` bindings, list elements,
module-call arguments (where a parameter is typed `int`), and string interpolations.

```mainstage
let retries = 3
let offset  = -5
let ports   = [8080, 8081, 8082]
```

When interpolated into a string, an integer renders as its decimal form:

```mainstage
let url = "http://localhost:${ports[0]}"   // "http://localhost:8080"
```

A literal outside the `i64` range, or one immediately followed by identifier
characters (e.g. `12abc`), is a parse error.

### Boolean Literals

```mainstage
allow_failure: true
allow_failure: false
```

### List Literals

An ordered collection of expressions enclosed in `[]`. Trailing comma is allowed.

```mainstage
outputs: ["dist/app", "dist/app.sha256"]
stages:  [compile, test, package]
```

### Glob

Returns a `fileset` — a typed collection of files with path metadata. Accepts one or more glob patterns.

```mainstage
let sources = glob("src/**/*.rs")
let headers = glob("include/**/*.h", "vendor/**/*.h")
```

Globs are evaluated at runtime relative to the script file's directory.

### Conditional Expression (`if / else`)

Evaluates to one of two expressions based on a condition. Both branches must produce the same type. The `else` branch is required when used as an expression.

```mainstage
let target = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};
```

Conditional expressions can appear anywhere a value is expected, including field values and list elements:

```mainstage
pipeline release {
    stages: if env("CI") {
        [compile, lint, test, package, deploy]
    } else {
        [compile, lint, test, package]
    }
}
```

### Module Calls

Invoke a function from an imported module. Arguments may be positional or named (keyword arguments).

```mainstage
env.get("OUT_DIR", default: "dist")
git.tag()
git.sha(short: true)
```

### Stage Output Reference

Reference the declared outputs of another stage. Creates an implicit dependency — the runtime ensures the referenced stage runs first.

```mainstage
stage package {
    inputs: [compile.outputs, assets]
    ...
}
```

`<stage-name>.outputs` has type `list`.

---

## Conditions

Conditions appear in `if` expressions and `if` steps.

| Syntax                          | Meaning                                       |
|---------------------------------|-----------------------------------------------|
| `env("VAR")`                    | True if the environment variable is set       |
| `env("VAR") == "value"`         | True if the variable equals the string        |
| `env("VAR") != "value"`         | True if the variable does not equal the string|
| `platform == "windows"`         | True on Windows                               |
| `platform == "linux"`           | True on Linux                                 |
| `platform == "macos"`           | True on macOS                                 |
| `platform != "windows"`         | True on any platform except Windows           |
| `!<condition>`                  | Logical negation                              |
| `<condition> and <condition>`   | Logical AND                                   |
| `<condition> or <condition>`    | Logical OR                                    |

Conditions are short-circuit evaluated. `and` binds more tightly than `or`. Use parentheses to control grouping:

```mainstage
if env("CI") and (platform == "linux" or platform == "macos") {
    ...
}
```

---

## Steps

Steps appear inside `steps { }` and `on_failure { }` / `on_success { }` blocks. They execute in declaration order.

### `$` — Execute Program

Runs a program directly. **Not passed to a shell.** The runtime tokenizes the remainder of the line into argv — the first token is the program, the rest are arguments. String interpolation is applied before tokenization. Quote tokens that contain spaces.

```text
$ <program> [<arg> ...]
```

```mainstage
$ cargo build --release
$ tar -czf "${outputs[0]}" dist/
$ aws s3 cp "${inputs[0]}" "s3://my-bucket/releases/"
```

The runtime resolves the program name against the system `PATH`. Path separators in arguments are normalized per platform.

### `copy` — Copy Files

Copies a file, file set, or directory to a destination path. Creates the destination directory if it does not exist, and **force-overwrites** an existing destination file — even a read-only one (the destination is removed first, like `cp -f`) — so re-copying a file whose previous copy inherited read-only permissions does not fail. Copying a directory merges into the destination, overwriting files of the same name; files present only in the destination are left untouched. For a clean replacement, `delete` the destination first, then `copy`.

```text
copy <src> to <dest>
```

```mainstage
copy assets to "${out}/assets/"
copy "LICENSE" to "${out}/LICENSE"
copy ovmf_vars to "build/run/OVMF_VARS.fd"   // overwrites the prior run's copy
```

### `move` — Move Files

Moves a file or directory to a destination path.

```text
move <src> to <dest>
```

```mainstage
move "target/release/my-app" to "${out}/my-app"
```

### `mkdir` — Create Directory

Creates a directory and all required parent directories.

```text
mkdir <path>
```

```mainstage
mkdir "${out}/assets/"
```

### `delete` — Remove Files

Removes a file, directory, or file set. Deleting a directory removes it recursively. Does not error if the path does not exist.

```text
delete <path>
```

```mainstage
delete "target/"
delete "${out}/"
```

### `write` — Write a File

Writes a string to a file, creating it (or overwriting it) at the given path.

```text
write <path> content: <string>
```

```mainstage
write "${out}/VERSION" content: "${project.version}"
```

### `if` / `else` — Conditional Steps

Conditionally executes a block of steps. The `else` branch is optional in step context.

```text
if <condition> {
    <step>
    ...
} else {
    <step>
    ...
}
```

```mainstage
steps {
    if platform == "windows" {
        $ cargo build --release --target x86_64-pc-windows-msvc
    } else {
        $ cargo build --release
    }

    if env("CI") {
        $ aws s3 cp "${outputs[0]}" "s3://releases/"
    }
}
```

### `for` — Iterate Over a File Set

Executes a block of steps once per file in a file set. The loop variable exposes file metadata properties.

```text
for <var> in <fileset> {
    <step>
    ...
}
```

```mainstage
stage compile {
    inputs:  sources
    outputs: ["obj/"]

    steps {
        mkdir "obj/"
        for file in inputs {
            $ gcc -c "${file.path}" -o "obj/${file.stem}.o"
        }
    }
}
```

**File item properties:**

| Property    | Description                                  | Example                    |
|-------------|----------------------------------------------|----------------------------|
| `file.path` | Full path relative to the script directory   | `"src/main.rs"`            |
| `file.name` | Filename with extension                      | `"main.rs"`                |
| `file.stem` | Filename without extension                   | `"main"`                   |
| `file.ext`  | Extension without leading dot                | `"rs"`                     |
| `file.dir`  | Parent directory path                        | `"src"`                    |

### `try` — Tolerate a Failing Step

Runs a block of steps but does **not** propagate a failure: if a step inside the block fails, the remaining steps in the block are skipped and the stage continues as though the block had succeeded. This is the native, checkable replacement for the `$ sh -c "… || true"` idiom — a best-effort step whose failure is acceptable.

```text
try {
    <step>
    ...
}
```

```mainstage
stage initialize {
    steps {
        // A refresh that may fail on an unrelated third-party source must not block
        // the install that follows.
        try {
            $ apt-get update
        }
        $ apt-get install -y nasm gcc
    }
}
```

A step's captured output is still shown; only its non-zero exit is swallowed. `try` does not trigger the stage's `on_failure` block, because the stage itself does not fail.

> **Prefer native steps over `$ sh -c`.** Reach for `copy` / `move` / `mkdir` / `delete` / `write` and `try` instead of shelling out: they run without a shell (no quoting or `PATH` surprises), are validated at analysis time, and work identically across platforms. For example, `sh -c "rm -rf d && mkdir -p d/sub && cp a d/sub/b"` is better written as `delete "d"` then `mkdir "d/sub"` then `copy a to "d/sub/b"`, and `sh -c "cmd || true"` as `try { $ cmd }`.

---

## Failure Handling

### Stage-Level `on_failure`

Declared inside a `stage` block. Runs if and only if that stage's `steps` produce a non-zero exit code or a built-in step fails. Used for local cleanup and diagnostics.

```mainstage
stage test {
    inputs:  sources
    outputs: ["coverage/"]

    steps {
        $ cargo test
    }

    on_failure {
        delete "coverage/"
        $ cargo test -- --nocapture
    }
}
```

### Pipeline-Level `on_failure` / `on_success`

Declared inside a `pipeline` block. `on_failure` runs if any stage in the pipeline fails (after that stage's own `on_failure` completes). `on_success` runs after all stages complete successfully.

The variable `failed_stage` is available inside `on_failure` and resolves to the name of the stage that failed.

```mainstage
pipeline release {
    stages: [compile, test, package, deploy]

    on_failure {
        $ slack-notify "Pipeline failed at ${failed_stage}"
        delete "${out}/"
    }

    on_success {
        $ slack-notify "Released ${project.version} successfully"
    }
}
```

### `allow_failure`

When `allow_failure: true` is set on a stage, a failure in that stage does not cancel downstream stages or trigger the pipeline's `on_failure`. The stage is treated as succeeded for dependency resolution purposes. Useful for non-blocking checks like linting or coverage.

```mainstage
stage lint {
    inputs:        sources
    allow_failure: true

    steps {
        $ cargo clippy
    }
}
```

### Failure Propagation Rules

1. A failed stage **cancels all stages that depend on its outputs**. Stages with no dependency on the failed stage continue running.
2. `allow_failure: true` stages are treated as succeeded — their outputs are considered valid and downstream stages are not cancelled.
3. A stage's `on_failure` block **always runs** when that stage fails, regardless of propagation. It is not subject to cancellation.
4. The pipeline `on_failure` runs once after all stage-level `on_failure` blocks complete.
5. If multiple stages fail (possible when they run in parallel), `failed_stage` resolves to the first one that failed in declaration order.

---

## Multiple Pipelines & CLI

A script may declare any number of pipelines. Pipelines are independent entry points — they are not chained or run sequentially by default. Stage definitions are shared across pipelines.

### CLI Usage

| Command                    | Behavior                                                      |
|----------------------------|---------------------------------------------------------------|
| `mainstage`                | Run the `default pipeline`. Error if none is declared.        |
| `mainstage run <name>`     | Run the named pipeline.                                       |
| `mainstage list`           | List all declared pipelines and their stages.                 |

### Parallel execution

Independent branches of a pipeline's stage dependency graph run concurrently. A stage
starts as soon as every stage it depends on has completed, so unrelated stages overlap
on multi-core hosts while dependency order — and the failure-propagation, `allow_failure`,
and `on_failure` / `on_success` semantics — are preserved exactly.

The `-j` / `--jobs N` flag caps how many stages run at once (default: the host core
count). `--jobs 1` forces fully sequential execution with live, unbuffered step output.
With more than one worker, each stage's terminal output — its status markers and the
captured stdout/stderr of its steps — is buffered and flushed as one atomic block, so the
output of concurrent stages never interleaves.

```text
mainstage run ci              // run with the default worker budget
mainstage --jobs 4 run ci     // run at most 4 stages concurrently
mainstage --jobs 1 run ci     // sequential, live output
```

### Interruption

Pressing Ctrl-C (or sending `SIGTERM`) requests a graceful stop: the runner stops
launching new stages and lets the stages already in flight finish, so their outputs stay
whole. The change-detection cache is then written atomically — a temp file renamed into
place — so an interrupted run never leaves a truncated or corrupt `cache.json`. The run
exits reporting cancellation; completed stages are recorded, so a re-run resumes from
where it left off.

### Rules

- Exactly zero or one pipeline may be marked `default`. Two `default` declarations is a parse error.
- Running `mainstage` with no arguments and no `default pipeline` is a runtime error — the user must be explicit.
- Pipelines share stage *definitions* but each pipeline invocation runs its stages independently. Running `mainstage run dev` and `mainstage run release` are fully independent executions.

```mainstage
default pipeline dev {
    stages: [compile]
}

pipeline ci {
    stages: [compile, lint, test]
}

pipeline release {
    stages: [compile, lint, test, package, deploy]
}
```

```text
mainstage               // runs "dev"
mainstage run ci        // runs "ci"
mainstage run release   // runs "release"
```

---

## Built-in Variables & Properties

These are available throughout a script without import.

| Name                  | Type     | Description                                              |
|-----------------------|----------|----------------------------------------------------------|
| `platform`            | `string` | Current OS: `"windows"`, `"linux"`, or `"macos"`         |
| `project.name`        | `string` | Value of the `name` field in the `project` block         |
| `project.version`     | `string` | Value of the `version` field in the `project` block      |
| `project.description` | `string` | Value of the `description` field in the `project` block  |
| `project.author`      | `string` | Value of the `author` field in the `project` block       |

Inside `steps`, `on_failure`, and `on_success` blocks, the following context variables are also available:

| Name            | Available in       | Description                                      |
|-----------------|--------------------|--------------------------------------------------|
| `inputs`        | stage steps        | The resolved file list for the stage's inputs    |
| `outputs`       | stage steps        | The declared output paths for the stage          |
| `failed_stage`  | pipeline on_failure| Name of the stage that caused the failure        |

Inside a stage declared with a `matrix` block, each matrix dimension name (e.g. `arch`) is also available as a built-in string variable resolving to that variant's value. See [`stage`](#stage).

---

## Formal Grammar (EBNF)

```ebnf
program         = item* ;
item            = import_decl
                | let_decl
                | project_block
                | stage_block
                | pipeline_block ;

import_decl     = "import" string "as" ident ";" ;
let_decl        = "let" ident "=" expr ";" ;

project_block   = "project" "{" project_field* "}" ;
project_field   = ident ":" expr ","? ;

stage_block     = "stage" ident "{" stage_field* "}" ;
stage_field     = "inputs"        ":" expr                              ","?
                | "outputs"       ":" expr                              ","?
                | "depends_on"    ":" "[" ( ident ( "," ident )* ","? )? "]" ","?
                | "matrix"        "{" matrix_dim*                       "}"
                | "allow_failure" ":" bool                              ","?
                | "always_run"    ":" bool                              ","?
                | "run_once"      ":" bool                              ","?
                | "steps"         "{" step*                             "}"
                | "on_failure"    "{" step*                             "}" ;
matrix_dim      = ident ":" "[" ( string ( "," string )* ","? )? "]" ","? ;

pipeline_block  = "default"? "pipeline" ident "{" pipeline_field* "}" ;
pipeline_field  = "input"      ":" expr         ","?
                | "stages"     ":" list_expr    ","?
                | "on_failure" "{" step*        "}"
                | "on_success" "{" step*        "}" ;

(* Steps *)
step            = exec_step
                | copy_step
                | move_step
                | mkdir_step
                | delete_step
                | write_step
                | if_step
                | for_step
                | try_step ;

exec_step       = "$" token+ NEWLINE ;
copy_step       = "copy" expr "to" expr ;
move_step       = "move" expr "to" expr ;
mkdir_step      = "mkdir" expr ;
delete_step     = "delete" expr ;
write_step      = "write" expr "content" ":" string ;
if_step         = "if" condition "{" step* "}" ( "else" "{" step* "}" )? ;
for_step        = "for" ident "in" expr "{" step* "}" ;
try_step        = "try" "{" step* "}" ;

(* Expressions *)
expr            = string
                | int
                | bool
                | list_expr
                | glob_expr
                | if_expr
                | module_call
                | stage_ref
                | member_access
                | ident ;

if_expr         = "if" condition "{" expr "}" "else" "{" expr "}" ;
list_expr       = "[" ( expr ( "," expr )* ","? )? "]" ;
glob_expr       = "glob" "(" string ( "," string )* ")" ;
module_call     = ident "." ident "(" arg_list? ")" ;
stage_ref       = ident "." "outputs" ;
member_access   = ident "." ident ;
arg_list        = arg ( "," arg )* ;
arg             = expr | ident ":" expr ;

(* Conditions *)
condition       = or_cond ;
or_cond         = and_cond ( "or" and_cond )* ;
and_cond        = unary_cond ( "and" unary_cond )* ;
unary_cond      = "!" unary_cond | primary_cond ;
primary_cond    = "(" condition ")"
                | "env" "(" string ")" ( ( "==" | "!=" ) string )?
                | "platform" ( "==" | "!=" ) platform_val ;
platform_val    = '"windows"' | '"linux"' | '"macos"' ;

(* Primitives *)
string          = '"' ( char | interpolation )* '"' ;
interpolation   = "${" expr "}" ;
int             = "-"? digit+ ;
bool            = "true" | "false" ;
ident           = [a-zA-Z_] [a-zA-Z0-9_]* ;
digit           = [0-9] ;
```

---

## Complete Example

```mainstage
import "env" as env;
import "git" as git;

project {
    name:    "my-app"
    version: git.tag()
    author:  "Colt McGraw"
}

let sources = glob("src/**/*.rs");
let assets  = glob("assets/**/*");
let out     = env.get("OUT_DIR", default: "dist");
let target  = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};

// --- Pipelines ---

default pipeline dev {
    input:  sources
    stages: [compile]
}

pipeline ci {
    input:  sources
    stages: [compile, lint, test]

    on_failure {
        $ slack-notify "CI failed at ${failed_stage} on ${env("BRANCH")}"
    }
}

pipeline release {
    input:  sources
    stages: if env("CI") {
        [compile, lint, test, package, deploy]
    } else {
        [compile, lint, test, package]
    }

    on_failure {
        $ slack-notify "Release failed at ${failed_stage}"
        delete "${out}/"
    }

    on_success {
        $ slack-notify "Released ${project.version}"
    }
}

// --- Stages ---

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
    inputs:  sources
    outputs: ["coverage/"]

    steps {
        $ cargo test
    }

    on_failure {
        delete "coverage/"
    }
}

stage package {
    inputs:  [compile.outputs, assets]
    outputs: ["${out}/${project.name}-${project.version}.tar.gz"]

    steps {
        mkdir "${out}/"
        copy assets to "${out}/assets/"
        write "${out}/VERSION" content: "${project.version}"
        $ tar -czf "${outputs[0]}" "${out}/"
    }
}

stage deploy {
    inputs: package.outputs

    steps {
        if env("DRY_RUN") {
            $ echo "Dry run — skipping upload of ${inputs[0]}"
        } else {
            $ aws s3 cp "${inputs[0]}" "s3://releases/${project.name}/"
        }
    }
}
```
