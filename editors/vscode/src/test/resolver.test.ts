import { test } from "node:test";
import assert from "node:assert/strict";

import {
  executableNames,
  resolveServer,
  subcommandFor,
  ResolverHost,
} from "../serverResolver";

/**
 * A fake host whose `isExecutable` answers `true` only for the listed full
 * paths, so resolution can be driven without touching the real filesystem.
 */
function host(overrides: Partial<ResolverHost> & { executables?: string[] }): ResolverHost {
  const executables = new Set(overrides.executables ?? []);
  return {
    platform: overrides.platform ?? "linux",
    pathVar: overrides.pathVar,
    home: overrides.home ?? "/home/dev",
    isExecutable: overrides.isExecutable ?? ((file) => executables.has(file)),
  };
}

// ── subcommandFor ──────────────────────────────────────────────────────────────

test("subcommandFor adds `lsp` only for the bare CLI binary", () => {
  assert.deepEqual(subcommandFor("mainstage"), ["lsp"]);
  assert.deepEqual(subcommandFor("/usr/local/bin/mainstage"), ["lsp"]);
  assert.deepEqual(subcommandFor("C:\\tools\\mainstage.exe"), ["lsp"]);
  assert.deepEqual(subcommandFor("C:\\tools\\mainstage.cmd"), ["lsp"]);
  // The dedicated server binary speaks LSP directly — no subcommand.
  assert.deepEqual(subcommandFor("mainstage-lsp"), []);
  assert.deepEqual(subcommandFor("/opt/bin/mainstage-lsp"), []);
});

test("executableNames adds Windows suffixes only on win32", () => {
  assert.deepEqual(executableNames("mainstage-lsp", "linux"), ["mainstage-lsp"]);
  assert.deepEqual(executableNames("mainstage-lsp", "win32"), [
    "mainstage-lsp.exe",
    "mainstage-lsp.cmd",
    "mainstage-lsp",
  ]);
});

// ── resolveServer: configured path ───────────────────────────────────────────────

test("a configured CLI path is launched with the `lsp` subcommand", () => {
  const server = resolveServer("/custom/mainstage", [], host({}));
  assert.deepEqual(server, { command: "/custom/mainstage", args: ["lsp"] });
});

test("a configured server-binary path is launched directly", () => {
  const server = resolveServer("/custom/mainstage-lsp", [], host({}));
  assert.deepEqual(server, { command: "/custom/mainstage-lsp", args: [] });
});

test("configured extra arguments are appended after the subcommand", () => {
  const server = resolveServer("/custom/mainstage", ["--log", "debug"], host({}));
  assert.deepEqual(server, { command: "/custom/mainstage", args: ["lsp", "--log", "debug"] });
});

test("a blank/whitespace configured path falls through to discovery", () => {
  // Nothing discoverable, so the result is `undefined` rather than `{command:""}`.
  assert.equal(resolveServer("   ", [], host({})), undefined);
});

// ── resolveServer: discovery ─────────────────────────────────────────────────────

test("discovery prefers `mainstage-lsp` on PATH over the bare CLI", () => {
  const server = resolveServer(
    "",
    [],
    host({
      pathVar: "/a:/b",
      executables: ["/a/mainstage", "/b/mainstage-lsp"],
    }),
  );
  // Even though `mainstage` appears earlier on PATH, the dedicated server wins.
  assert.deepEqual(server, { command: "/b/mainstage-lsp", args: [] });
});

test("discovery falls back to the bare CLI with `lsp` when only it exists", () => {
  const server = resolveServer(
    "",
    [],
    host({ pathVar: "/a:/b", executables: ["/b/mainstage"] }),
  );
  assert.deepEqual(server, { command: "/b/mainstage", args: ["lsp"] });
});

test("discovery searches common install dirs when PATH yields nothing", () => {
  const server = resolveServer(
    "",
    [],
    host({ pathVar: "/empty", home: "/home/dev", executables: ["/home/dev/.local/bin/mainstage-lsp"] }),
  );
  assert.deepEqual(server, { command: "/home/dev/.local/bin/mainstage-lsp", args: [] });
});

test("discovery yields undefined when no binary is found anywhere", () => {
  assert.equal(resolveServer("", [], host({ pathVar: "/a:/b", executables: [] })), undefined);
});

test("extra arguments are appended to a discovered server too", () => {
  const server = resolveServer(
    "",
    ["--flag"],
    host({ pathVar: "/b", executables: ["/b/mainstage-lsp"] }),
  );
  assert.deepEqual(server, { command: "/b/mainstage-lsp", args: ["--flag"] });
});
