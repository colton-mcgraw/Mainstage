// External-plugin example (Phase 15).
//
// `greet` is provided by the subprocess plugin declared in `plugins.toml`. Once
// discovered, plugin calls are validated and evaluated identically to built-ins —
// the analyzer checks `greet.hello` / `greet.echo_num` against the signatures the
// plugin reports from `describe`.
//
// Consumed by `core/tests/examples.rs` (the plugin-driven test is unix-only).

import "greet" as greet;
import "str"   as str;

let who   = greet.hello("World");   // "hello, World"
let shout = str.upper(who);         // "HELLO, WORLD"
let n     = greet.echo_num(21);     // 21  — an int literal round-trips through the plugin
