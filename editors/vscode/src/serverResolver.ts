import { posix, win32 } from "node:path";

/** A resolved language-server invocation: the executable and its arguments. */
export interface ResolvedServer {
  command: string;
  args: string[];
  /**
   * Whether `command` is the server binary bundled inside the extension (as
   * opposed to a user-configured path). The caller uses this to decide whether
   * to repair the executable bit, which VSIX extraction can drop.
   */
  bundled: boolean;
}

/**
 * The host facts the resolver depends on, injected rather than read from the
 * ambient process so the resolution logic can be exercised in isolation (no
 * `vscode`, no real filesystem). `extension.ts` supplies the real values.
 */
export interface ResolverHost {
  /** `process.platform` of the host the extension runs on. */
  platform: NodeJS.Platform;
  /** The extension's install directory (`context.extensionPath`). */
  extensionPath: string;
  /** Whether `file` exists on disk. */
  fileExists: (file: string) => boolean;
}

/**
 * Resolve the language server command with no manual setup required.
 *
 * Precedence:
 *   1. `configuredPath` (the `mainstage.server.path` setting), if non-empty —
 *      an explicit escape hatch for development or a system install.
 *   2. The server binary bundled inside the extension (`server/mainstage-lsp`),
 *      shipped per-platform in the VSIX so the extension works out of the box.
 *
 * A configured `mainstage` binary is launched as `mainstage lsp`; a configured
 * `mainstage-lsp` binary — and the bundled one — speak LSP directly. `extraArgs`
 * are appended in every case. Returns `undefined` when nothing is configured and
 * no bundled binary is present for this platform.
 */
export function resolveServer(
  configuredPath: string,
  extraArgs: string[],
  host: ResolverHost,
): ResolvedServer | undefined {
  const configured = configuredPath.trim();
  if (configured) {
    return {
      command: configured,
      args: [...subcommandFor(configured), ...extraArgs],
      bundled: false,
    };
  }

  const bundled = bundledServerPath(host);
  if (host.fileExists(bundled)) {
    return { command: bundled, args: [...extraArgs], bundled: true };
  }

  return undefined;
}

/**
 * Decide the leading subcommand for a user-supplied executable path: a bare
 * `mainstage` CLI (optionally with a `.exe`/`.cmd` suffix) needs the `lsp`
 * subcommand, while any other name (e.g. `mainstage-lsp`) speaks LSP directly.
 */
export function subcommandFor(command: string): string[] {
  const base = command.replace(/\\/g, "/").split("/").pop() ?? command;
  return /^mainstage(\.(exe|cmd))?$/i.test(base) ? ["lsp"] : [];
}

/** The bundled server's filename on the given platform. */
export function serverBinaryName(platform: NodeJS.Platform): string {
  return platform === "win32" ? "mainstage-lsp.exe" : "mainstage-lsp";
}

/**
 * The path where the per-platform server binary is bundled inside the
 * extension. Packaging copies the matching `mainstage-lsp` into `server/`.
 */
export function bundledServerPath(host: ResolverHost): string {
  // Join with the host's own `path` conventions so the path is correct whether
  // the extension host is Windows (`\`) or POSIX (`/`).
  const { join } = host.platform === "win32" ? win32 : posix;
  return join(host.extensionPath, "server", serverBinaryName(host.platform));
}
