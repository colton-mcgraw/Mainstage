// Copy the locally built `mainstage-lsp` binary into `server/` so it is bundled
// into the VSIX. Packaging is per-platform: each platform-specific VSIX ships
// only the binary that matches it.
//
// Env:
//   MAINSTAGE_PROFILE  cargo profile dir under `target/` (default: "release").
//   MAINSTAGE_TARGET   cargo target triple; when set, reads from
//                      `target/<triple>/<profile>/` (cross/`--target` builds).
//
// Usage: node scripts/copy-server.js

const { copyFileSync, mkdirSync, existsSync } = require("node:fs");
const { join } = require("node:path");

const isWindows = process.platform === "win32";
const binary = isWindows ? "mainstage-lsp.exe" : "mainstage-lsp";

const profile = process.env.MAINSTAGE_PROFILE || "release";
const target = process.env.MAINSTAGE_TARGET || "";

// scripts/ → editors/vscode → editors → workspace root.
const root = join(__dirname, "..", "..", "..");
const targetDir = target
  ? join(root, "target", target, profile)
  : join(root, "target", profile);
const source = join(targetDir, binary);

if (!existsSync(source)) {
  console.error(
    `Server binary not found at ${source}.\n` +
      "Build it first, e.g.:\n" +
      "  cargo build -p mainstage_lsp --bin mainstage-lsp --release",
  );
  process.exit(1);
}

const destDir = join(__dirname, "..", "server");
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, binary);
copyFileSync(source, dest);
console.log(`Copied ${source} -> ${dest}`);
