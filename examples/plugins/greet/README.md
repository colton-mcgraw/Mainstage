# greet

The minimal reference [Mainstage](../../../README.md) plugin: a single string method.
Speaks the newline-delimited JSON protocol over stdio (see
[`docs/PLUGINS.md`](../../../docs/PLUGINS.md)).

## Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `hello` | `hello(name: string) -> string` | Returns `hello, <name>`. |

## Validate

```sh
mainstage plugin check greet.py
```

## Use

```toml
# plugins.toml
[plugins]
greet = "greet.py"
```

```mainstage
import "greet" as greet;

let msg = greet.hello("World");   // "hello, World"
```
