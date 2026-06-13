# Authoring & Publishing Mainstage Plugins

A **plugin** adds a module to Mainstage — new methods callable from `.ms` scripts —
without recompiling the interpreter or forking the project. A plugin is just an
executable that speaks a small newline-delimited JSON protocol over stdio, so you can
write one in any language that can read stdin, write stdout, and handle JSON.

This guide walks through scaffolding, writing, validating, and publishing a plugin.
For the protocol as a terse reference (and the built-in standard library), see
[`MODULES.md`](MODULES.md#external-plugins). For a list of existing plugins and
runnable reference examples, see [`PLUGIN_INDEX.md`](PLUGIN_INDEX.md).

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [The Protocol](#the-protocol)
3. [Writing a Plugin in Any Language](#writing-a-plugin-in-any-language)
4. [Validating a Plugin](#validating-a-plugin)
5. [Naming & Namespacing](#naming--namespacing)
6. [Versioning](#versioning)
7. [Registering & Discovery](#registering--discovery)
8. [Publishing](#publishing)
9. [Reference Examples](#reference-examples)

---

## Quick Start

Scaffold a working plugin with `mainstage plugin new`:

```sh
mainstage plugin new acme/lint            # Python skeleton (default)
mainstage plugin new acme/lint --lang sh  # POSIX-shell skeleton
```

This writes a self-contained directory holding a plugin that already answers
`describe` and a sample `greet` call, plus a `README.md`. The skeleton passes
validation immediately, so you start from green and edit from there:

```sh
mainstage plugin check lint/lint.py
# checking lint/lint.py
#   module acme/lint · 1 method(s)
# ok: plugin conforms to the protocol
```

`mainstage plugin new` options:

| Flag | Default | Meaning |
|------|---------|---------|
| `--lang <python\|shell>` | `python` | Skeleton language (`py` / `sh` aliases accepted). |
| `--dir <DIR>` | the plugin's base name | Output directory. |
| `--force` | off | Overwrite an existing output directory. |

---

## The Protocol

The host (Mainstage) writes **one request line** and reads **exactly one response
line**. The process is spawned once — when the module loads — kept alive for the whole
run, and reused for every call. Closing stdin signals the plugin to exit.

There are two operations.

### `describe`

Sent once at load time so Mainstage learns the module's name and method signatures.
The signatures feed semantic analysis, so plugin calls are validated *before the
pipeline runs*, exactly like built-in modules.

```json
→ {"op":"describe"}
← {"name":"greet","methods":[
     {"name":"hello","params":[{"name":"name","type":"string","required":true}],"returns":"string"}
   ]}
```

A method signature has up to four fields, all but `name` optional:

| Field | Default | Meaning |
|-------|---------|---------|
| `name` | — (required) | Method name; must be a valid identifier. |
| `params` | `[]` | Positional parameters, in order. |
| `named` | `[]` | Keyword parameters (passed `name: value` at the call site). |
| `returns` | `"any"` | Return [value type](MODULES.md#value-types). |

Each parameter is `{"name": <ident>, "type": <type>, "required": <bool>}`. `type`
defaults to `any`; `required` defaults to `true` for positionals and `false` for
keyword params. Type tags are `string`, `int`, `bool`, `list`, `fileset`, or `any`.

### `call`

Sent for each method invocation with already-evaluated arguments. Positional
arguments have no `name`; keyword arguments carry theirs.

```json
→ {"op":"call","method":"hello","args":[{"value":{"type":"string","value":"World"}}]}
← {"ok":{"type":"string","value":"hello, World"}}
```

On failure, return `err` instead of `ok`. The message is surfaced as an evaluation
error carrying the call's source span:

```json
← {"err":"could not reach service"}
```

### Value encoding

Every value is an internally-tagged object: `{"type": <tag>, "value": <v>}`.

| Type | `value` shape |
|------|---------------|
| `string` | a JSON string |
| `int` | a JSON integer (64-bit signed) |
| `bool` | `true` / `false` |
| `list` | an array of encoded values |
| `fileset` | an array of `{path, name, stem, ext, dir}` objects |

---

## Writing a Plugin in Any Language

The contract is just "read a line, write a line." A complete plugin loop is:

1. Read a line from stdin; if EOF, exit.
2. Parse it as JSON.
3. If `op == "describe"`, print your name and methods.
4. If `op == "call"`, dispatch on `method`, returning `{"ok": …}` or `{"err": …}`.
5. **Flush stdout** after every response — buffered output deadlocks the host, which
   blocks reading your reply.
6. Write diagnostics to **stderr** (it is inherited by the terminal); never to stdout,
   which carries only protocol lines.

The default Python skeleton from `mainstage plugin new` is the recommended starting
point because it parses JSON properly. The shell skeleton is dependency-free but
parses arguments with `sed`, so it suits only simple string handling.

---

## Validating a Plugin

`mainstage plugin check <path>` spawns the plugin, sends `describe`, and checks the
response against the protocol. It is read-only — it never sends `call`, so it never
triggers your plugin's side effects — which makes it safe to run in CI as a
pre-publish gate.

It reports two severities:

- **errors** — the plugin will not load or cannot be called (bad type tag, duplicate
  method, a method or parameter name that is not a valid identifier, a `describe` that
  fails or returns malformed JSON). The command exits non-zero.
- **warnings** — the plugin works but breaks a convention (an empty or non-identifier
  module name, a name that collides with a built-in, a required positional after an
  optional one, no methods at all).

```sh
mainstage plugin check ./wordcount.py || echo "fix before publishing"
```

---

## Naming & Namespacing

A module name is the string in `import "<name>" as <alias>;`. Two conventions:

- **Unscoped** names (`greet`, `wordcount`) are fine for local, single-project
  plugins, but they risk colliding with other plugins a user installs.
- **Namespaced** names — `<owner>/<tool>`, e.g. `acme/lint` — are strongly recommended
  for anything you publish. The `/` separator namespaces by author or organization and
  is reflected in directory discovery (`.mainstage/plugins/acme/lint`).

Each `/`-separated segment must be a valid identifier (letters, digits, underscores;
not starting with a digit). Built-in standard-library names (`env`, `git`, `str`,
`path`, `hash`, `fs`, `json`, `shell`, `http`, `time`) are **reserved** — a plugin may
never shadow one, and discovery silently skips a plugin that tries. `plugin check`
warns you about a collision before you publish.

---

## Versioning

The protocol itself is stable and unversioned at the wire level; these conventions
keep *your plugin* evolvable without breaking the scripts that depend on it.

- **Treat your method signatures as a public API.** Follow
  [Semantic Versioning](https://semver.org): a **patch** fixes behavior, a **minor**
  adds a method or an *optional* parameter, and a **major** removes or renames a
  method, changes a return type, makes an optional parameter required, or otherwise
  breaks existing calls.
- **Add, don't mutate.** Prefer a new method or a new optional/keyword parameter over
  changing an existing one — additive changes never break a caller.
- **Surface your version.** Expose a `version() -> string` method (and/or print it to
  stderr on startup) so users and bug reports can identify the build.
- **Tag releases** in your repository so a specific version can be installed and
  pinned.

---

## Registering & Discovery

For a given script, Mainstage discovers plugins in this order — **built-ins always win
and are never shadowed**:

1. **Directory** — executables under `.mainstage/plugins/`. Nested paths form
   namespaced names: `.mainstage/plugins/acme/lint` → module `acme/lint`. On Unix the
   file needs its execute bit set (`chmod +x`).
2. **Manifest** — a `plugins.toml` next to the script mapping names to executable
   paths (resolved relative to the manifest). Directory entries win on a name conflict.

```toml
# plugins.toml
[plugins]
greet = "greet.py"
"acme/lint" = "bin/lint"
```

Because `.mainstage/` is conventionally git-ignored (it also holds the
change-detection cache), declare any plugin you want under version control through
`plugins.toml`. Once registered, import it like any other module:

```mainstage
import "acme/lint" as lint;

let issues = lint.check("src");
```

---

## Publishing

A plugin is distributed as its executable (and any runtime it needs). A good published
plugin includes:

- a **README** documenting each method's signature and behavior (the scaffold writes a
  starter one),
- the **registration** snippet above so users can wire it in,
- a **`plugin check`** step in your CI, and
- a **license** and a **version tag**.

To make your plugin discoverable by others, add it to the community index — see
[`PLUGIN_INDEX.md`](PLUGIN_INDEX.md) for the format and how to submit an entry.

---

## Reference Examples

Runnable, validated reference plugins live under
[`examples/plugins/`](../examples/plugins/):

- [`greet`](../examples/plugins/greet/) — the minimal one-method plugin.
- [`wordcount`](../examples/plugins/wordcount/) — multiple methods, the `int` return
  type, file I/O, and structured `err` reporting.

The in-repo integration plugin [`tests/plugin/greet.sh`](../tests/plugin/greet.sh)
(with [`tests/plugin/main.ms`](../tests/plugin/main.ms)) shows the POSIX-shell form.
