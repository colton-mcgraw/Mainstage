![Mainstage](./media/mainstage_logo_text.svg)

# Mainstage

[![License](https://img.shields.io/badge/License-blue.svg)](LICENSE.md) [![GitHub issues](https://img.shields.io/github/issues/ColtMcG1/mainstage)](https://github.com/ColtMcG1/mainstage/issues) [![GitHub forks](https://img.shields.io/github/forks/ColtMcG1/mainstage)](https://github.com/ColtMcG1/mainstage/forks) [![GitHub stars](https://img.shields.io/github/stars/ColtMcG1/mainstage)](https://github.com/ColtMcG1/mainstage/stars) [![CI](https://github.com/ColtMcG1/mainstage/actions/workflows/ci-dev.yml/badge.svg)](https://github.com/ColtMcG1/mainstage/actions/workflows/ci-dev.yml) [![Release](https://github.com/ColtMcG1/mainstage/actions/workflows/release.yml/badge.svg)](https://github.com/ColtMcG1/mainstage/actions/workflows/release.yml)

Mainstage is a scripting language designed for cross-platform orchestration and automation tasks. It aims to provide a simple and intuitive syntax for defining workflows. Its design focuses on readability and ease of use, making it accessible for both beginners and experienced developers. It’s designed to be extensible, allowing users to create custom modules and plugins to enhance its functionality. It is not tied to a specific domain, but rather is a general-purpose tool that can be adapted to a wide range of use cases and expanded as needed via its modular architecture.

## Quick Start

```powershell
# Build the CLI and copy Windows binary
cargo build --release --bin mainstage-cli ; Copy-Item -Force ./cli/target/release/mainstage-cli.exe ./mainstage.exe

# Run an example
./mainstage.exe run .\examples\example_stage\script.ms

# Verify stdlib against manifest
./mainstage.exe verify stdlib --plugin-dir .\plugin --strict
```

## Features

- Cross-platform compatibility
- Intuitive syntax for workflow definition
- Extensible architecture with custom modules and plugins

## Installation

To install Mainstage, follow these steps:

- If you are running the local installer, download the installer from the [official website](https://github.com/ColtMcG1/mainstage/releases) and follow the on-screen instructions.
- If you are using a package manager, you can install Mainstage using the following command:

  ```bash
  # Using Homebrew (macOS/Linux)
  brew install mainstage
  # Using Chocolatey (Windows)
  choco install mainstage
  ```

- If you prefer manual installation, follow these steps:

  1. Download the latest release from the [official repository](https://github.com/ColtMcG1/mainstage/releases).
  2. Extract the downloaded archive to your desired location.
  3. Add the Mainstage binary to your system's PATH for easy access.
  4. Verify the installation by running `mainstage --version` in your terminal.

## Getting Started

To get started with Mainstage, create a new script file with the `.ms` extension. Here is a simple example of a Mainstage script:

```mainstage
workspace hello_world {
    say("Hello, World!");
}
```

To run the script, use the following command in your terminal:

```bash
mainstage run your_script.ms
```

See the `examples/` directory for more sample scripts.

### Here is a list of available commands:

- `mainstage run <script.ms> --plugin`: Executes the specified Mainstage script.
- `mainstage build <script.ms> -o <output>`: Compiles the Mainstage script to a binary executable (.msx).
- `mainstage inspect <script.ms>`: Decompiles and displays the internal representation of the script.
- `mainstage verify <module-name> --plugin-dir <path-to-plugins>`: Verifies that a plugin's manifest functions match what the plugin actually registers at runtime.

## Documentation

For detailed documentation on Mainstage, including syntax, built-in functions, and examples, please visit the [official documentation site](https://github.com/ColtMcG1/mainstage/wiki).

See the `docs/` directory for local documentation files.

- GRAMMAR: `docs/GRAMMAR.md`
- MSBC SPEC: `docs/MSBC_SPEC.md`
- Plugin ABI: `docs/PLUGIN_ABI.md`

### Shell Completion (planned)

Generate completion scripts for your shell to enable tab-completion of commands and flags.

Planned command:

```
mainstage completion <powershell|bash|zsh|fish>
```

PowerShell install example (once available):

```powershell
# Generate and import completion for the current session
mainstage completion powershell | Invoke-Expression

# Persist completion across sessions
mainstage completion powershell > $PROFILE\mainstage-completion.ps1
Add-Content -Path $PROFILE -Value "`n. \"$PROFILE\mainstage-completion.ps1\"" 
```

### Configuration

- `MAINSTAGE_PLUGIN_DIR`: default plugin discovery directory (overrides `--plugin-dir`).
- Color/output: respects `NO_COLOR` and supports `--color auto|always|never`.

### Troubleshooting

- Plugin not found: ensure artifacts exist under `target/release` or set `--plugin-dir` to the correct path.
- Windows copy errors: use PowerShell `Copy-Item` (not `cp`).
- Verification cannot load: confirm the plugin is built as in-process and `entry` matches the library name.

### Testing

To run the test suite for Mainstage, navigate to the `core/` directory and execute the following command:

```bash
cargo test
```

Or to run tests by category, use the provided script:

```powershell
.\scripts\run_core_tests_by_category.ps1 -Category [CategoryName]
```

- Replace `[CategoryName]` with the desired test category (e.g., `lowering`, `opt`, `ir`, etc.).
-

### Verify Plugin Manifests

Use the verifier to check that a plugin's manifest functions match what the plugin actually registers at runtime.

```bash
cargo run -- verify <module-name> --plugin-dir <path-to-plugins>
```

- The verifier loads in-process plugins and lists registered functions.
- It compares manifest functions using fully-qualified names (`domain.name`).
- To be resilient, it also matches unqualified names (e.g., `ask`) against qualified manifest entries (e.g., `util.ask`).
- If the plugin library cannot be found or is not in-process, the tool reports it cannot verify.

Common Windows build output paths searched: `target/debug/` and `target/release/` under the plugin crate.

### CLI Quick Reference

Commands:

- `build`: compile a script and emit artifacts
- `run`: execute a `.ms` script
- `inspect`: disassemble a script
- `verify`: compare manifest vs runtime-registered plugin functions

Global flags:

- `--plugin-dir <path>`: directory to search for plugin manifests and artifacts; defaults to `./plugin` if present. Env override: `MAINSTAGE_PLUGIN_DIR`.
- `-v/--verbose` (repeatable): increase logging verbosity; `--quiet` reduces output.
- `--color <auto|always|never>`: control colorized output; respects `NO_COLOR`.

Verify options:

- `--module <name>`: verify a single plugin; if omitted, verifies all discovered manifests.
- `--json`: output machine-readable differences `{ missing: [], extra: [] }`.
- `--strict`: exit non-zero when mismatches are found.

Run options:

- `--dry-run`: parse and validate without executing.
- `--timeout <ms>`: set a global timeout for the run.
- `--env KEY=VALUE` / `--env-file <path>`: set environment overrides.

PowerShell examples (Windows):

```powershell
# Run a script
cargo run -- run .\examples\example_stage\script.ms

# Inspect bytecode
cargo run -- inspect .\examples\example_stage\script.ms

# Verify stdlib plugin
cargo run -- verify stdlib --plugin-dir .\plugin

# Verify all plugins (strict, JSON output)
cargo run -- verify --plugin-dir .\plugin --strict --json
```

### Stdlib Functions (current)

The `stdlib` plugin currently exposes functions across these domains:

- `env`: `env.get`, `env.list`, `env.set`
- `fs`: `fs.copy`, `fs.delete`, `fs.remove_dir`, `fs.exists`, `fs.glob`, `fs.list_dir`, `fs.make_dir`, `fs.move`, `fs.read`, `fs.stat`, `fs.write`
- `json`: `json.parse`, `json.stringify`
- `path`: `path.normalize`, `path.resolve`, `path.relativize`
- `proc`: `proc.exec`, `proc.exit`, `proc.which`
- `rand`: `rand.float`, `rand.int`
- `string`: `string.trim`, `string.replace`
- `time`: `time.current_time_millis`, `time.sleep`
- `util`: `util.ask`, `util.say`, `util.echo`, `util.echo_typed`
- `util.array`: `util.array.append`, `util.array.empty`, `util.array.extend`, `util.array.length`

Note: Some previously planned functions (e.g., `util.fmt`, `proc.get_cwd`, `proc.set_cwd`, `proc.spawn`) are not currently implemented and have been removed from the manifest. The verifier will flag any drift between manifest and runtime.

## Contributing

Contributions to Mainstage are welcome! If you would like to contribute, please follow these steps:

1. Fork the repository on GitHub.
2. Create a new branch for your feature or bug fix.
3. Make your changes and commit them with clear messages.
4. Push your changes to your forked repository.
5. Submit a pull request to the main repository.

Please ensure that your code adheres to the project's coding standards and includes appropriate tests, documentation, and comments.

## License

See the [LICENSE](LICENSE.md) file for license rights and limitations.

## Contact

For questions or support, please open an issue on the [GitHub repository](https://github.com/ColtMcG1/mainstage/issues).

## Acknowledgments

We would like to thank all contributors and users who have supported the development of Mainstage. Your feedback and contributions are invaluable to the growth of this project.

---

Thank you for using Mainstage!

## Releases

- Tagged releases (`v*`) build cross-platform binaries via GitHub Actions and attach them to the GitHub Release. See `.github/workflows/release.yml`.
- Windows: download `mainstage.exe` and add its folder to PATH.
- macOS/Linux: download `mainstage` and place it in a location on PATH (e.g., `/usr/local/bin`).
