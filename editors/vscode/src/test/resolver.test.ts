import { test } from "node:test";
import assert from "node:assert/strict";

import {
  bundledServerPath,
  resolveServer,
  serverBinaryName,
  subcommandFor,
  ResolverHost,
} from "../serverResolver";

/**
 * A fake host whose `fileExists` answers `true` only for the listed full paths,
 * so resolution can be driven without touching the real filesystem.
 */
function host(overrides: Partial<ResolverHost> & { files?: string[] }): ResolverHost {
  const files = new Set(overrides.files ?? []);
  return {
    platform: overrides.platform ?? "linux",
    extensionPath: overrides.extensionPath ?? "/ext",
    fileExists: overrides.fileExists ?? ((file) => files.has(file)),
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

// ── bundled paths ────────────────────────────────────────────────────────────────

test("serverBinaryName adds the Windows suffix only on win32", () => {
  assert.equal(serverBinaryName("linux"), "mainstage-lsp");
  assert.equal(serverBinaryName("darwin"), "mainstage-lsp");
  assert.equal(serverBinaryName("win32"), "mainstage-lsp.exe");
});

test("bundledServerPath joins under the extension's server/ dir, per platform", () => {
  assert.equal(
    bundledServerPath(host({ platform: "linux", extensionPath: "/ext" })),
    "/ext/server/mainstage-lsp",
  );
  assert.equal(
    bundledServerPath(host({ platform: "win32", extensionPath: "C:\\ext" })),
    "C:\\ext\\server\\mainstage-lsp.exe",
  );
});

// ── resolveServer: configured path ───────────────────────────────────────────────

test("a configured CLI path is launched with the `lsp` subcommand", () => {
  const server = resolveServer("/custom/mainstage", [], host({}));
  assert.deepEqual(server, { command: "/custom/mainstage", args: ["lsp"], bundled: false });
});

test("a configured server-binary path is launched directly", () => {
  const server = resolveServer("/custom/mainstage-lsp", [], host({}));
  assert.deepEqual(server, { command: "/custom/mainstage-lsp", args: [], bundled: false });
});

test("configured extra arguments are appended after the subcommand", () => {
  const server = resolveServer("/custom/mainstage", ["--log", "debug"], host({}));
  assert.deepEqual(server, {
    command: "/custom/mainstage",
    args: ["lsp", "--log", "debug"],
    bundled: false,
  });
});

test("a configured path wins even when a bundled binary exists", () => {
  const server = resolveServer(
    "/custom/mainstage-lsp",
    [],
    host({ files: ["/ext/server/mainstage-lsp"] }),
  );
  assert.deepEqual(server, { command: "/custom/mainstage-lsp", args: [], bundled: false });
});

// ── resolveServer: bundled binary ────────────────────────────────────────────────

test("a blank/whitespace configured path falls through to the bundled binary", () => {
  const server = resolveServer(
    "   ",
    [],
    host({ files: ["/ext/server/mainstage-lsp"] }),
  );
  assert.deepEqual(server, { command: "/ext/server/mainstage-lsp", args: [], bundled: true });
});

test("the bundled binary is found at the platform-specific path on Windows", () => {
  const server = resolveServer(
    "",
    [],
    host({ platform: "win32", extensionPath: "C:\\ext", files: ["C:\\ext\\server\\mainstage-lsp.exe"] }),
  );
  assert.deepEqual(server, {
    command: "C:\\ext\\server\\mainstage-lsp.exe",
    args: [],
    bundled: true,
  });
});

test("extra arguments are appended to the bundled binary too", () => {
  const server = resolveServer(
    "",
    ["--flag"],
    host({ files: ["/ext/server/mainstage-lsp"] }),
  );
  assert.deepEqual(server, {
    command: "/ext/server/mainstage-lsp",
    args: ["--flag"],
    bundled: true,
  });
});

test("resolution yields undefined when nothing is configured and no binary is bundled", () => {
  assert.equal(resolveServer("", [], host({ files: [] })), undefined);
});
