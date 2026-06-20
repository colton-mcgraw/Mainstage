# Changelog

All notable changes to the Mainstage VS Code extension are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project uses [Semantic Versioning](https://semver.org/).

## [1.1.0] - 2026-06-19

### Changed

- **The language server is now bundled with the extension.** Each published VSIX
  is platform-specific and ships a matching `mainstage-lsp` binary, so the
  extension works out of the box with no CLI install and no configuration —
  including in remote containers, WSL, and over SSH. Server auto-discovery on
  `PATH` and in common install locations has been removed; set
  `mainstage.server.path` to override the bundled server with a custom build.
- Declared `extensionKind: ["workspace"]` so the extension runs on the workspace
  side in remote setups, where the build and project files live and where the
  bundled binary's architecture matches.

### Added

- Activation logging of the host platform/architecture, remote name, and the
  resolved server path, to make remote and WSL start-up failures diagnosable.
- The bundled server is marked executable on activation when VSIX extraction
  drops the exec bit (a common cause of `EACCES` on POSIX remotes).
- A clearer notification when no server binary matches the current platform,
  pointing at the install instructions and `mainstage.server.path`.

## [1.0.0] - 2026-06-14

### Added

- Initial release: a client for the Mainstage language server (`mainstage lsp`).
- Diagnostics, completion, hover, signature help, go-to-definition, find
  references, document highlight, document symbols, and formatting — all served
  by the language server, identical to the `mainstage` CLI.
- Zero-config server discovery: finds `mainstage` / `mainstage-lsp` on `PATH`
  and in common install locations, overridable via `mainstage.server.path`.
- Explicit `onLanguage:mainstage` activation so the client starts reliably when
  a `.ms` file is opened.
- Syntax highlighting and editor configuration (comments, brackets,
  auto-closing pairs) for `.ms` files.
- Commands to restart the server and show its output.
- A test suite: resolver unit tests for server discovery, plus integration
  tests that drive the real language server over stdio and assert its
  responses.
