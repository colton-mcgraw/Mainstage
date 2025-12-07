# Plugin ABI Overview

This document describes the Mainstage plugin ABI, covering in-process plugins, function naming, manifests, memory, and verification tooling.

## Function Names

- Plugins register functions under fully-qualified names: `domain.name` (e.g., `fs.read`, `env.set`, `util.array.append`).
- Qualified names improve discoverability and avoid collisions across domains.

## Manifests

Each plugin includes a `manifest.json` adjacent to its source or build output. Required fields:

```json
{
	"name": "stdlib",
	"version": "0.1.0",
	"kind": "inprocess",
	"entry": "stdlib",
	"path": "target/release",
	"functions": [
		{ "domain": "fs", "name": "read", "args": [{"name":"path","kind":"String"}], "returns": {"name":"content","kind":"String","optional": true} }
	]
}
```

- `entry`: base library name (no extension). The host probes platform-specific filenames.
- `path`: directory or file path to the built library. If relative, resolved against the manifest directory.
- `functions`: declared functions; the CLI verifier compares this list to runtime registrations.

## In-Process ABI

In-process plugins export a simple C ABI (see `docs/INPROCESS_PLUGIN.md` for details). At minimum:
- `plugin_name()` → `const char*`
- `plugin_call_json(const char* func, const char* args_json)` → `char*`
- `plugin_free(char*)` (recommended)

Typed ABI is supported for richer types (arrays/objects) with deep-free semantics.

## Memory Ownership

- Strings returned by the plugin must be heap-allocated and freed by the host via `plugin_free` when provided, otherwise via `free()`.
- Typed values crossing the boundary must provide a deep-free callback when the plugin allocates memory for strings/arrays/objects.

## Verification Tooling

Use the CLI `verify` command to check that a plugin's manifest matches runtime-registered functions:

```bash
cargo run -- verify <module-name> --plugin-dir ./plugin
```

- Compares using qualified names; also tolerates unqualified runtime names.
- Options:
	- `--json`: emit `{ checked, mismatched, results }` for automation.
	- `--strict`: non-zero exit status on mismatches (CI-friendly).
- Artifact search: probes `manifest.path`, the manifest directory, and typical Cargo paths (`target/debug`, `target/release`).

## Best Practices

- Prefer qualified names for clarity.
- Keep manifests up to date with actual runtime functions.
- Export `plugin_free` and ensure allocator compatibility.
- Sign release binaries and publish checksums for distribution.
