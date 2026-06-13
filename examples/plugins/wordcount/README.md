# wordcount

A reference [Mainstage](../../../README.md) plugin returning file metrics. A step up
from [`greet`](../greet/): multiple methods, the `int` return type, file I/O, and
structured `err` reporting. Speaks the newline-delimited JSON protocol over stdio (see
[`docs/PLUGINS.md`](../../../docs/PLUGINS.md)).

Relative paths resolve against the script's directory — the plugin runs with that as
its working directory.

## Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `lines` | `lines(path: string) -> int` | Number of lines in the file. |
| `words` | `words(path: string) -> int` | Number of whitespace-separated words. |
| `chars` | `chars(path: string) -> int` | Number of characters. |

A missing or unreadable file is reported via `err`, surfaced as an evaluation error at
the call site.

## Validate

```sh
mainstage plugin check wordcount.py
```

## Use

```toml
# plugins.toml
[plugins]
wordcount = "wordcount.py"
```

```mainstage
import "wordcount" as wc;

let total = wc.lines("README.md");
```
