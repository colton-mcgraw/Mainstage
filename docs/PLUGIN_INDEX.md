# Mainstage Plugin Index

A discoverable list of Mainstage plugins — modules you can add to a script without
recompiling the interpreter. New to plugins? Start with
[`PLUGINS.md`](PLUGINS.md), the authoring and publishing guide.

> **Status:** Mainstage is pre-1.0 and this index is seeded with the in-repo reference
> examples. Community submissions are welcome — see [Adding Your Plugin](#adding-your-plugin).

---

## Reference Examples

Maintained in this repository under [`examples/plugins/`](../examples/plugins/). Each
one passes `mainstage plugin check` and is a good template to copy.

| Plugin | Language | Methods | What it shows |
|--------|----------|---------|---------------|
| [`greet`](../examples/plugins/greet/) | Python | `hello(name) -> string` | The minimal one-method plugin. |
| [`wordcount`](../examples/plugins/wordcount/) | Python | `lines / words / chars (path) -> int` | Multiple methods, the `int` return type, file I/O, and `err` reporting. |
| [`greet.sh`](../tests/plugin/greet.sh) | POSIX shell | `hello(name) -> string`, `echo_num(n) -> int` | The dependency-free shell form (used by the integration tests). |

Scaffold your own from any of these with:

```sh
mainstage plugin new <name>            # Python
mainstage plugin new <name> --lang sh  # POSIX shell
```

---

## Community Plugins

> _No community plugins are listed yet. Open a pull request to add yours — it will
> appear here._

<!--
Add entries to this table, sorted alphabetically by name. Keep one row per plugin.

| Plugin | Author | Methods | Description | Repository |
|--------|--------|---------|-------------|------------|
| `acme/lint` | @acme | `check(path) -> list` | Lints source files. | https://github.com/acme/mainstage-lint |
-->

| Plugin | Author | Methods | Description | Repository |
|--------|--------|---------|-------------|------------|
| — | — | — | — | — |

---

## Adding Your Plugin

To list a plugin here:

1. Make sure it passes validation: `mainstage plugin check <path>` reports no errors.
2. Use a **namespaced** name (`<owner>/<tool>`, e.g. `acme/lint`) to avoid collisions —
   see [Naming & Namespacing](PLUGINS.md#naming--namespacing).
3. Publish it in a public repository with a README documenting each method and a
   version tag.
4. Open a pull request adding one row to the **Community Plugins** table above, sorted
   alphabetically by plugin name, with:
   - the module name, your handle, the method signatures (or a count), a one-line
     description, and a link to the repository.

Listing here is informational; plugins are not vetted or endorsed. Review a plugin's
source before installing it — a plugin runs as a local process with your permissions.
