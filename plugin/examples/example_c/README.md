c_plugin_example
================

Minimal C in-process plugin example for Mainstage.

Build (Makefile)
-----------------

- Linux (default):

```bash
make -C plugin/c_plugin_example all-linux
```

- macOS:

```bash
make -C plugin/c_plugin_example all-macos
```

- Windows (MinGW):

```powershell
make -C plugin/c_plugin_example all-win
```

Build (CMake)
-------------

```bash
cd plugin/c_plugin_example
mkdir build && cd build
cmake ..
cmake --build . --config Release
```

Manifest
--------

This plugin includes `manifest.json` which the host can use to discover and prefer an in-process load.

Usage
-----

- Build the platform shared library and place it where the host looks for plugins.
- The host will search for platform-specific filenames derived from the `entry` in `manifest.json`.

Notes
-----

- Exported functions: `plugin_name`, `plugin_call_json`, `plugin_free`.
- Prefer using `plugin_free` to avoid allocator mismatches.
