# In-Process Plugin Guide

This document describes the in-process plugin ABI, how to build example plugins (Rust and C), how to declare them in a plugin manifest, how the host expects memory to be managed, and quick troubleshooting and security notes.

Overview
--------

In-process plugins are platform shared libraries (DLL/.so/.dylib) that the host loads into the same process and calls directly. They expose a small C-style API that accepts/returns JSON for arguments and results. In-process plugins are powerful but run in the host process (no sandboxing); only load trusted plugins.

ABI (required functions)
------------------------

Plugins must expose the following C ABI functions (names must be exactly as shown):

- `plugin_name()`
	- Returns: `const char *`
	- Description: a null-terminated UTF-8 string with the plugin's canonical name. The host treats this string as read-only; the plugin should return either a static string or a pointer that remains valid for the lifetime of the library.

- `plugin_call_json(const char *func, const char *args_json)`
	- Returns: `char *`
	- Parameters:
		- `func` — null-terminated UTF-8 string naming the function the host wants to call.
		- `args_json` — null-terminated JSON string representing an array of arguments (JSON array).
	- Return value: a pointer to a malloc-allocated, null-terminated JSON string containing the return value (any valid JSON value). The host will take responsibility for freeing this string — see `plugin_free` below.

- `plugin_free(char *ptr)` — optional but strongly recommended
	- Description: frees a pointer previously returned by `plugin_call_json`. If present, the host will call this to free plugin-allocated buffers. If `plugin_free` is not exported the host will call `free()` from the C runtime (which works in most cases but may be unsafe across allocator boundaries on some platforms or language toolchains).

Memory ownership rules
----------------------

- `plugin_name()` returns a pointer the host will *not* free. Keep it static or otherwise valid for the library lifetime.
- `plugin_call_json()` must return a heap-allocated null-terminated C string (UTF-8) containing JSON. The host will free it by calling `plugin_free` if exported; otherwise the host will call the platform `free()` as a fallback.
- Prefer exporting `plugin_free` to avoid allocator mismatch issues across runtimes.

JSON contract
-------------

- Arguments: `args_json` is a JSON array, e.g. `[1, "a", {"k": true}]`.
- Result: any JSON value (object, array, string, number, boolean or `null`).
- Errors: on error, return a JSON object with the shape `{"error": "message"}`. The host will propagate errors to the caller.

Manifest
--------

Add a manifest entry to tell the host this is an in-process plugin and what library name to load. Example `manifest.json` snippet:

```
{
	"name": "rust_inproc",
	"entry": "rust_inproc",
	"kind": "inprocess"
}
```

- `entry`: base name of the library (no extension). The host will try platform-specific filenames: `rust_inproc.dll` (Windows), `librust_inproc.so` (Linux), `librust_inproc.dylib` (macOS), and also plain `rust_inproc`.
- `kind: "inprocess"` signals the registry to prefer trying an in-process load.

Runtime Function Names
----------------------

- Functions are registered with fully-qualified names of the form `domain.name` (e.g., `fs.read`, `env.set`).
- The CLI verifier normalizes manifest and runtime names and also compares unqualified names to be tolerant of minor discrepancies.

Verifying Your Plugin
---------------------

Run the CLI verifier to compare your manifest with the runtime-registered functions:

```
cargo run -- verify-manifest <module-name> --plugin-dir <path-to-plugins>
```

- The verifier attempts to locate your built plugin artifacts under typical Cargo paths, including `target/debug` and `target/release`.
- On Windows, `.dll` and `.exe` naming variants are also probed based on your `entry` name.

Rust example (cdylib)
---------------------

1. Create a crate with `crate-type = ["cdylib"]` in `Cargo.toml`.
2. Implement the required ABI functions. Example minimal implementation (conceptual):

```rust
use std::ffi::{CString, CStr};
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn plugin_name() -> *const c_char {
		static NAME: &str = "rust_inproc";
		CString::new(NAME).unwrap().into_raw() // plugin owns the allocation for the lifetime
}

#[no_mangle]
pub extern "C" fn plugin_call_json(func: *const c_char, args_json: *const c_char) -> *mut c_char {
		// parse inputs, dispatch, produce JSON string, return pointer via CString::into_raw()
		// on error return CString::new("{\"error\":\"msg\"}").unwrap().into_raw()
		std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn plugin_free(ptr: *mut c_char) {
		if ptr.is_null() { return }
		unsafe { CString::from_raw(ptr) }; // reclaims and drops the allocation
}
```

Build (release) and locate the produced shared library under `target/release/`.

C example
---------

Implement the same three functions using `malloc`/`free` or a static string for `plugin_name`. Example build commands:

PowerShell / Windows (MinGW):
```powershell
gcc -shared -o rust_inproc.dll plugin.c
```

Linux:
```bash
gcc -shared -fPIC -o librust_inproc.so plugin.c
```

macOS:
```bash
gcc -shared -fPIC -o librust_inproc.dylib plugin.c
```

Testing locally
---------------

1. Build the plugin artifact (Rust/C) for your platform.
2. Place the shared library in a location the host will search (same folder as the CLI or a configured plugin directory), or run the local ignored test which expects a built artifact.
3. Run the ignored integration test in `core` (PowerShell):

```powershell
cd 'c:\Users\coltm\Code\mainstage-v-0-2-0\plugin\rust_inproc'
cargo build --release
cd 'c:\Users\coltm\Code\mainstage-v-0-2-0\core'
cargo test -- --ignored
```

Security and stability notes
----------------------------

- In-process plugins run inside the host process. They can crash or corrupt memory — only load plugins you trust.
- Prefer exporting `plugin_free` to avoid cross-allocator issues.
- Avoid running untrusted code in-process; for untrusted plugins prefer the external (process) plugin mechanism.
- Consider validating plugin names and paths in deployment automation to avoid DLL preloading/planting attacks.

Troubleshooting
---------------

- If the host fails to load the library, ensure the filename and manifest `entry` match and the library was built for the target platform/ABI.
- On symbol resolution errors, confirm the exported names are exactly `plugin_name`, `plugin_call_json`, and optionally `plugin_free` with C ABI (no Rust-only mangling).
- If return buffers cause crashes, implement and export `plugin_free` and use the same allocator for allocation and free.

Where to put examples
---------------------

Place example plugin crates or sources under `plugin/` (we use `plugin/rust_inproc` for the Rust example). Keep `manifest.json` adjacent so the host's discovery can associate the manifest with the library artifact.

CI
--

A GitHub Actions workflow `build-c-plugin.yml` is included under `.github/workflows/`. It builds `plugin/c_plugin_example` using CMake on Ubuntu, macOS and Windows runners and then runs the `core` tests with ignored tests enabled (so the in-process discovery test will run when a built artifact is present).
