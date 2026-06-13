# Changelog

All notable changes to the Mainstage VS Code extension are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project uses [Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

### Added

- Initial release: a client for the Mainstage language server (`mainstage lsp`).
- Diagnostics, completion, hover, signature help, go-to-definition, find
  references, document symbols, and formatting — all served by the language
  server, identical to the `mainstage` CLI.
- Zero-config server discovery: finds `mainstage` / `mainstage-lsp` on `PATH`
  and in common install locations, overridable via `mainstage.server.path`.
- Syntax highlighting and editor configuration (comments, brackets,
  auto-closing pairs) for `.ms` files.
- Commands to restart the server and show its output.
