# Reference Plugins

Runnable, validated example [Mainstage](../../README.md) plugins. Each one passes
`mainstage plugin check` and is a good template to copy when authoring your own.

| Plugin | Language | What it shows |
|--------|----------|---------------|
| [`greet`](greet/) | Python | The minimal one-method plugin. |
| [`wordcount`](wordcount/) | Python | Multiple methods, the `int` return type, file I/O, and `err` reporting. |

See [`docs/PLUGINS.md`](../../docs/PLUGINS.md) for the authoring and publishing guide,
and [`docs/PLUGIN_INDEX.md`](../../docs/PLUGIN_INDEX.md) for the community index.

## Trying one

```sh
mainstage plugin check greet/greet.py

# Register it for a script by adding to plugins.toml next to your main.ms:
#   [plugins]
#   greet = "path/to/examples/plugins/greet/greet.py"
# then:  import "greet" as greet;
```

Scaffold a fresh skeleton of your own with `mainstage plugin new <name>`.
