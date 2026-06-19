# Mainstage Modules

Modules provide the functions a Mainstage script calls from expressions and
conditions. This document covers the module system, every built-in standard-library
module and its methods, the capability/permission model that gates side-effecting
modules, and the external-plugin protocol for adding your own modules without
recompiling Mainstage.

For the language grammar itself, see [`GRAMMAR.md`](GRAMMAR.md).

---

## Table of Contents

1. [Using Modules](#using-modules)
2. [Value Types](#value-types)
3. [Standard Library](#standard-library)
   - [`env`](#env) · [`git`](#git) · [`str`](#str) · [`path`](#path) · [`hash`](#hash) · [`fs`](#fs) · [`json`](#json) · [`shell`](#shell) · [`http`](#http) · [`time`](#time)
4. [Permissions & Capabilities](#permissions--capabilities)
5. [External Plugins](#external-plugins)
6. [The `mainstage modules` Command](#the-mainstage-modules-command)

---

## Using Modules

Bring a module into scope with an `import` declaration, then call its methods through
the alias:

```mainstage
import "env" as env;
import "git" as git;

let out     = env.get("OUT_DIR", default: "dist");
let version = git.tag(default: "0.0.0");
```

Arguments may be **positional** or **named** (keyword). Named arguments use
`name: value` syntax and can appear in any order after the positional ones:

```mainstage
git.sha(short: true)
env.get("HOME", default: "/tmp")
```

Module calls are validated during semantic analysis — before anything runs. An
unknown module, an unknown method, the wrong number of positional arguments, an
unrecognized keyword, a missing required argument, or a literal argument of the wrong
type are all reported with a source span, not deferred to runtime.

### Signature notation

Throughout this document (and in `mainstage modules`) method signatures are written
in call form:

```text
get(var: string, default?: string) -> string
```

- Each parameter is `name: type`.
- A `?` after the name marks an **optional** parameter.
- Positional parameters are listed first, then keyword parameters (which are always
  passed by name at the call site).
- The type after `->` is the return type.

---

## Value Types

Module parameters and return values are described with these type tags:

| Type      | Description                                                        |
|-----------|--------------------------------------------------------------------|
| `string`  | UTF-8 text                                                         |
| `int`     | 64-bit signed integer                                              |
| `bool`    | `true` or `false`                                                  |
| `list`    | Ordered collection of values                                       |
| `fileset` | A collection of files with path metadata (produced by `glob`)      |
| `any`     | Any value type — used for intentionally untyped parameters         |

> **Note on `int`:** integer literals and the `int` type are first-class in the
> language and round-trip through the plugin protocol, but most built-in methods that
> return a count or size (`str.len`, `fs.size`, `time.unix`) return a **string** for
> backward compatibility and stable interpolation output.

---

## Standard Library

The following modules are built in. They are always registered (a plugin may never
shadow one) and listed by `mainstage modules`.

### `env`

Read environment variables. Pure reads; no capability required.

| Method | Signature | Description |
|--------|-----------|-------------|
| `get` | `get(var: string, default?: string) -> string` | Value of `var`, or `default` (or `""`) if unset. |
| `has` | `has(var: string) -> bool` | Whether `var` is set. |

### `git`

Query the host git repository (runs the `git` executable in the script directory).

| Method | Signature | Description |
|--------|-----------|-------------|
| `sha` | `sha(short?: bool) -> string` | The `HEAD` commit SHA; `short: true` for the abbreviated form. |
| `tag` | `tag(default?: string) -> string` | The most recent tag; returns `default` when there is none (otherwise errors). |

### `str`

String manipulation. Pure and deterministic.

| Method | Signature | Description |
|--------|-----------|-------------|
| `upper` | `upper(s: string) -> string` | Uppercase. |
| `lower` | `lower(s: string) -> string` | Lowercase. |
| `trim` | `trim(s: string) -> string` | Strip leading/trailing whitespace. |
| `replace` | `replace(s: string, from: string, to: string) -> string` | Replace all occurrences. |
| `split` | `split(s: string, sep: string) -> list` | Split into a list; empty `sep` splits into characters. |
| `join` | `join(parts: list, sep: string) -> string` | Join list elements with `sep`. |
| `contains` | `contains(s: string, needle: string) -> bool` | Substring test. |
| `starts_with` | `starts_with(s: string, prefix: string) -> bool` | Prefix test. |
| `ends_with` | `ends_with(s: string, suffix: string) -> bool` | Suffix test. |
| `len` | `len(s: string) -> string` | Character count (as a string). |

### `path`

Path manipulation. Pure string/path operations; no I/O.

| Method | Signature | Description |
|--------|-----------|-------------|
| `join` | `join(base: string, child: string) -> string` | Join two path segments. |
| `dir` | `dir(path: string) -> string` | Parent directory. |
| `base` | `base(path: string) -> string` | Final component (filename). |
| `stem` | `stem(path: string) -> string` | Filename without extension. |
| `ext` | `ext(path: string) -> string` | Extension without the leading dot. |
| `with_ext` | `with_ext(path: string, ext: string) -> string` | Replace the extension (empty `ext` drops it). |
| `abs` | `abs(path: string) -> string` | Absolute form, resolved against the script directory. |

### `hash`

SHA-256 hashing — the same hasher used by change detection.

| Method | Signature | Description |
|--------|-----------|-------------|
| `sha256` | `sha256(text: string) -> string` | Hex digest of the UTF-8 bytes. |
| `sha256_file` | `sha256_file(path: string) -> string` | Hex digest of a file's contents. |

### `fs`

Read-only filesystem queries. File *mutation* stays in the step layer
(`write` / `copy` / `move` / `delete`). Relative paths resolve against the script
directory.

| Method | Signature | Description |
|--------|-----------|-------------|
| `exists` | `exists(path: string) -> bool` | Whether the path exists. |
| `read` | `read(path: string) -> string` | File contents as text. |
| `is_dir` | `is_dir(path: string) -> bool` | Whether the path is a directory. |
| `is_file` | `is_file(path: string) -> bool` | Whether the path is a regular file. |
| `size` | `size(path: string) -> string` | Size in bytes (as a string). |
| `list` | `list(path: string) -> list` | Directory entries, sorted, joined onto `path`. |
| `find_first` | `find_first(paths: list, default?: string) -> string` | First path in the list that exists; falls back to `default:`, or errors if none exist and no default. |

`find_first` resolves a file whose location varies across systems without hardcoding a
single name — for example, OVMF firmware that is `OVMF_CODE_4M.fd` on some distros and
`OVMF_CODE.fd` on others:

```mainstage
import "fs" as fs;

let firmware = fs.find_first([
    "/usr/share/OVMF/OVMF_CODE_4M.fd",
    "/usr/share/OVMF/OVMF_CODE.fd",
]);
```

### `json`

JSON access in **opaque-string** form: values are carried as their serialized text
rather than a distinct value type, so interpolation and `if/else` type compatibility
are unaffected.

| Method | Signature | Description |
|--------|-----------|-------------|
| `parse` | `parse(text: string) -> string` | Validate and return the compact canonical form. |
| `stringify` | `stringify(text: string) -> string` | Validate and pretty-print. |
| `get` | `get(text: string, path: string) -> string` | Extract the value at a dotted path (`"a.b.0"`) as a string. |

```mainstage
import "fs"   as fs;
import "json" as json;

let cfg  = fs.read("config.json");
let name = json.get(cfg, "project.name");
let first = json.get(cfg, "features.0");   // array indices are path segments
```

### `shell`

Run an external command and capture its stdout. **Requires the `run` capability**
(see [Permissions](#permissions--capabilities)). The command is tokenized into argv
exactly like the `$` exec step — no shell is involved.

| Method | Signature | Description |
|--------|-----------|-------------|
| `run` | `run(command: string) -> string` | Run `command`; return trimmed stdout. A non-zero exit is an error. |

### `http`

Make outbound HTTP(S) requests. **Requires the `net` capability.** Non-2xx responses
are errors.

| Method | Signature | Description |
|--------|-----------|-------------|
| `get` | `get(url: string) -> string` | Fetch and return the response body. |
| `download` | `download(url: string, path: string) -> string` | Save the body to `path`; return the written path. |

### `time`

Read the host wall clock. **Not** gated on a capability.

| Method | Signature | Description |
|--------|-----------|-------------|
| `now` | `now() -> string` | Current time as an RFC 3339 string. |
| `unix` | `unix() -> string` | Seconds since the Unix epoch (as a string). |
| `format` | `format(fmt: string) -> string` | Current time formatted with an strftime pattern. |

> **Determinism:** every `time` method reads the current clock, so feeding a `time`
> result into a stage's `inputs`/`outputs` defeats change detection — the digest
> changes on every run. Prefer `time` for display/metadata, not cache keys.

---

## Permissions & Capabilities

Side-effecting modules are gated behind **capabilities**. The granted set defaults to
**all-denied**; a script can only run a process or reach the network when the user
opts in.

| Capability | Gates | Grant via flag | Grant via manifest |
|------------|-------|----------------|--------------------|
| `run` | `shell` | `--allow-run` | `run = true` |
| `net` | `http` | `--allow-net` | `net = true` |

Both can be granted at once with `--allow-all`. Flags and the manifest combine as a
union — a capability granted by either source is in effect.

```toml
# plugins.toml
[permissions]
run = true
net = false
```

```text
mainstage --allow-run run release      # grant `run` for this invocation
```

When a gated method is called without its capability, it fails with a diagnostic that
names both the flag and the manifest key needed to grant it.

---

## External Plugins

A plugin is an external executable that adds a module without recompiling Mainstage.
It speaks a newline-delimited JSON protocol over stdio: the host writes one request
line and reads exactly one response line. The plugin process is spawned once (when the
module loads), reused for every call, and shut down at the end of the run.

Plugins are validated and called identically to built-ins — once discovered, their
reported signatures feed semantic analysis just like a built-in module's.

> **Authoring a plugin?** This section is the protocol reference. For a step-by-step
> guide — scaffolding with `mainstage plugin new`, validating with `mainstage plugin
> check`, and naming, versioning, and publishing conventions — see
> [`PLUGINS.md`](PLUGINS.md). For existing plugins and runnable examples, see
> [`PLUGIN_INDEX.md`](PLUGIN_INDEX.md).

### Protocol

Two operations exist.

**`describe`** — sent once at load time to learn the module's name and methods:

```json
→ {"op":"describe"}
← {"name":"greet","methods":[
     {"name":"hello","params":[{"name":"name","type":"string","required":true}],"returns":"string"},
     {"name":"echo_num","params":[{"name":"n","type":"int","required":true}],"returns":"int"}
   ]}
```

The discovered name (from where the plugin was found) is authoritative; the
self-reported `name` is advisory.

**`call`** — sent for each method invocation, with already-evaluated arguments:

```json
→ {"op":"call","method":"hello","args":[{"value":{"type":"string","value":"World"}}]}
← {"ok":{"type":"string","value":"hello, World"}}
```

On failure the plugin returns `err` instead of `ok`; the message is surfaced as an
evaluation error carrying the call's source span:

```json
← {"err":"could not reach service"}
```

### Value encoding

Values are encoded as internally-tagged JSON objects: `{"type":<tag>,"value":<v>}`.
The `type` tag is one of the [value types](#value-types) — `string`, `int`, `bool`,
`list`, or `fileset`. A `fileset` value is a list of file objects with `path`, `name`,
`stem`, `ext`, and `dir` fields. Method-signature fields (`params`, `named`,
`required`, `returns`) default sensibly when omitted: `params`/`named` to empty,
`required` to `true` for positionals and `false` for keyword params, and `returns` to
`any`.

### Discovery

For a given script, plugins are discovered in this order (built-ins always win and are
never shadowed):

1. **Directory** — executables under `.mainstage/plugins/`. Nested paths form
   namespaced names: `.mainstage/plugins/acme/lint` → module `acme/lint`.
2. **Manifest** — a `plugins.toml` mapping names to executable paths (resolved
   relative to the manifest). Directory entries win on a name conflict.

```toml
# plugins.toml
[plugins]
greet = "greet.sh"
"acme/fmt" = "bin/fmt"
```

Because module names are string literals, a namespaced plugin is imported exactly like
any other module — no grammar change is required:

```mainstage
import "acme/lint" as lint;
```

> The `.mainstage/` directory is conventionally git-ignored (it also holds the
> change-detection cache), so a plugin you want under version control is best declared
> through `plugins.toml`, as in `tests/plugin/`.

### Failure modes

- A missing or non-executable plugin file fails at load with a diagnostic naming the
  module.
- Malformed JSON, an unknown type tag, or a non-zero exit are reported as errors.
- A response with neither `ok` nor `err` is treated as a protocol violation.

### A minimal plugin

See [`tests/plugin/greet.sh`](../tests/plugin/greet.sh) for a complete, working
POSIX-shell plugin (and [`tests/plugin/main.ms`](../tests/plugin/main.ms) for a script
that uses it).

---

## The `mainstage modules` Command

List every available module — built-in and any plugin discovered under the script's
directory — with each method rendered in the signature notation above:

```text
mainstage modules                 # uses main.ms in the current directory
mainstage modules -f path/to/script.ms
```

Gated modules (`shell`, `http`) are always listed; capabilities only affect whether a
method may be *called*, not whether it is shown.

```text
env
  get(var: string, default?: string) -> string
  has(var: string) -> bool
git
  sha(short?: bool) -> string
  tag(default?: string) -> string
...
```

---

## Test Harness

Assertions are not module calls — they are built-in *steps* (`expect` and `assert`),
most useful inside a `test` stage, where they are tallied into a pass/fail count instead
of collapsing to a single exit code:

```mainstage
stage unit {
    test: true
    steps {
        assert "${project.version}" contains "1.2"   // compare a value
        expect ok $ ./run-unit-tests                 // assert a command exits 0
        expect output contains "PASS" $ ./smoke       // scrape captured output
    }
}
```

`expect` can also assert a non-zero exit (`fails`), match captured output
(`output contains` / `output equals`), and take a `timeout <seconds>` for boot-smoke
checks. A failed assertion fails the stage (and the run's exit code) but does not stop the
other assertions in the stage. See [GRAMMAR.md](GRAMMAR.md#test-harness) for the full
syntax and the [`tests/testing.ms`](../tests/testing.ms) example.
