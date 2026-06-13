//! Plugin scaffolding for `mainstage plugin new`.
//!
//! Emits a small, self-contained directory holding a **working** stdio plugin — one
//! that already answers `describe` and a sample `call` correctly — plus a README that
//! explains how to register, test, and extend it. The generated plugin passes
//! `mainstage plugin check` out of the box, so an author starts from green and edits
//! from there.

use std::path::{Path, PathBuf};

use console::style;

/// The language/runtime of a scaffolded plugin.
#[derive(Clone, Copy)]
pub enum Lang {
    /// A `python3` plugin — real JSON parsing, cross-platform.
    Python,
    /// A POSIX-shell plugin — dependency-free, matches `tests/plugin/greet.sh`.
    Shell,
}

impl Lang {
    /// Parse the `--lang` value; `None` for an unrecognized one.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "python" | "py" => Some(Lang::Python),
            "shell" | "sh" => Some(Lang::Shell),
            _ => None,
        }
    }

    /// The file extension for the generated plugin script.
    fn ext(self) -> &'static str {
        match self {
            Lang::Python => "py",
            Lang::Shell => "sh",
        }
    }
}

/// Generate a plugin skeleton named `name` (which may be namespaced, e.g.
/// `acme/lint`) under `dir` in `lang`. Refuses to overwrite an existing directory
/// unless `force` is set. On success, returns the path to the generated executable.
pub fn new_plugin(
    name: &str,
    dir: Option<&str>,
    lang: Lang,
    force: bool,
) -> Result<PathBuf, String> {
    if !is_valid_name(name) {
        return Err(format!(
            "invalid plugin name '{name}': use letters, digits, and underscores, with '/' to \
             namespace (e.g. 'acme/lint')"
        ));
    }
    // The file/dir base is the final namespace segment (`acme/lint` → `lint`).
    let base = name.rsplit('/').next().unwrap_or(name);
    let dir = PathBuf::from(dir.unwrap_or(base));

    if dir.exists() && !force {
        return Err(format!("{} already exists (use --force to overwrite)", dir.display()));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

    let script_name = format!("{base}.{}", lang.ext());
    let script_path = dir.join(&script_name);
    write_executable(&script_path, &render_plugin(name, lang))?;

    let readme = dir.join("README.md");
    write_file(&readme, &render_readme(name, &script_name))?;

    Ok(script_path)
}

/// Print the post-scaffold summary: what was written and the next steps to wire the
/// plugin into a project.
pub fn print_next_steps(name: &str, script: &Path) {
    let dir = script.parent().unwrap_or(Path::new("."));
    println!("{} plugin {}", style("created").green().bold(), style(name).cyan());
    println!("  {}", script.display());
    println!("  {}", dir.join("README.md").display());
    println!("\n{}", style("next steps").bold());
    println!("  1. validate the skeleton:  mainstage plugin check {}", script.display());
    println!("  2. register it for a script — either:");
    println!("       • copy it to  .mainstage/plugins/{name}");
    println!("       • or add to   plugins.toml:  \"{name}\" = \"{}\"", script.display());
    println!("  3. import it:  import \"{name}\" as {};", import_alias(name));
}

// ── Templates ─────────────────────────────────────────────────────────────────

/// Render the plugin source for `name` in `lang`.
fn render_plugin(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Python => render_python(name),
        Lang::Shell => render_shell(name),
    }
}

fn render_python(name: &str) -> String {
    format!(
        r#"#!/usr/bin/env python3
"""{name} — a Mainstage plugin.

Speaks the newline-delimited JSON protocol over stdio: one request per line in,
one response line out. See docs/PLUGINS.md in the Mainstage repository for the
full protocol, value encoding, and publishing conventions.
"""
import json
import sys

# The methods this plugin exposes. Reported in the `describe` response and used by
# Mainstage to validate every call *before* the pipeline runs. Each parameter type
# is one of: string, int, bool, list, fileset, any.
METHODS = [
    {{
        "name": "greet",
        "params": [{{"name": "name", "type": "string", "required": True}}],
        "returns": "string",
    }},
]


def describe():
    # The discovered name (where Mainstage found the plugin) is authoritative; this
    # self-reported name is advisory but should match by convention.
    return {{"name": "{name}", "methods": METHODS}}


def call(method, args):
    # `args` is a list of {{"name": <str|None>, "value": {{"type": ..., "value": ...}}}}.
    # Positional arguments have name == None; keyword arguments carry their name.
    if method == "greet":
        who = args[0]["value"]["value"]
        return {{"ok": {{"type": "string", "value": f"hello, {{who}}"}}}}
    return {{"err": f"unknown method '{{method}}'"}}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = json.loads(line)
        op = req.get("op")
        if op == "describe":
            resp = describe()
        elif op == "call":
            resp = call(req.get("method", ""), req.get("args", []))
        else:
            resp = {{"err": f"unknown op '{{op}}'"}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
"#
    )
}

fn render_shell(name: &str) -> String {
    format!(
        r#"#!/bin/sh
# {name} — a Mainstage plugin.
#
# Speaks the newline-delimited JSON protocol over stdio: one request per line in,
# one response line out. See docs/PLUGINS.md in the Mainstage repository for the
# full protocol. This shell skeleton parses arguments with `sed`; for anything
# beyond simple string handling, prefer the Python skeleton (`--lang python`).
#
# Methods:
#   greet(name: string) -> string   returns "hello, <name>"

while IFS= read -r line; do
  case "$line" in
    *'"op":"describe"'*)
      printf '%s\n' '{{"name":"{name}","methods":[{{"name":"greet","params":[{{"name":"name","type":"string","required":true}}],"returns":"string"}}]}}'
      ;;
    *'"method":"greet"'*)
      # Extract the first argument's string value and prefix a greeting.
      who=$(printf '%s' "$line" | sed 's/.*"value":"\([^"]*\)"}}.*/\1/')
      printf '%s\n' "{{\"ok\":{{\"type\":\"string\",\"value\":\"hello, $who\"}}}}"
      ;;
    *)
      printf '%s\n' '{{"err":"unknown method"}}'
      ;;
  esac
done
"#
    )
}

fn render_readme(name: &str, script: &str) -> String {
    let alias = import_alias(name);
    format!(
        r#"# {name}

A [Mainstage](https://github.com/ColtMcG1/mainstage) plugin — an external module
callable from `.ms` scripts. It speaks the newline-delimited JSON protocol over
stdio; see [`docs/PLUGINS.md`](https://github.com/ColtMcG1/mainstage/blob/main/docs/PLUGINS.md)
for the full specification.

## Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `greet` | `greet(name: string) -> string` | Returns `hello, <name>`. |

## Validate

```sh
mainstage plugin check {script}
```

## Register

Make the plugin discoverable for a project in either of two ways:

- **Directory** — copy the executable to `.mainstage/plugins/{name}`.
- **Manifest** — add it to `plugins.toml` next to your script:

  ```toml
  [plugins]
  "{name}" = "{script}"
  ```

## Use

```mainstage
import "{name}" as {alias};

let greeting = {alias}.greet("World");   // "hello, World"
```
"#
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// A sensible default import alias: the final namespace segment.
fn import_alias(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Whether `name` is a valid plugin module name — `/`-separated identifier segments.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name.split('/').all(is_ident)
}

/// Whether `s` is a valid Mainstage identifier (mirrors the `ident` grammar rule).
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Write `contents` to `path`, marking it executable on Unix.
fn write_executable(path: &Path, contents: &str) -> Result<(), String> {
    write_file(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not set execute bit on {}: {e}", path.display()))?;
    }
    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("could not write {}: {e}", path.display()))
}
