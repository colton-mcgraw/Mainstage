import { delimiter, join } from "node:path";

/** A resolved language-server invocation: the executable and its arguments. */
export interface ResolvedServer {
  command: string;
  args: string[];
}

/**
 * The host facts the resolver depends on, injected rather than read from the
 * ambient process so the resolution logic can be exercised in isolation (no
 * `vscode`, no real filesystem). `extension.ts` supplies the real values.
 */
export interface ResolverHost {
  /** `process.platform` of the host. */
  platform: NodeJS.Platform;
  /** The raw `PATH` environment variable, or `undefined` when unset. */
  pathVar: string | undefined;
  /** The user's home directory (`os.homedir()`). */
  home: string;
  /** Whether `file` exists and is executable on this host. */
  isExecutable: (file: string) => boolean;
}

/**
 * Resolve the language server command without requiring any manual setup.
 *
 * Precedence:
 *   1. `configuredPath` (the `mainstage.server.path` setting), if non-empty.
 *   2. A `mainstage-lsp`, then `mainstage`, binary found on `PATH`.
 *   3. The same, in a common install location.
 *
 * A `mainstage` binary is launched as `mainstage lsp`; a dedicated
 * `mainstage-lsp` binary is launched directly. `extraArgs` are appended in
 * every case.
 */
export function resolveServer(
  configuredPath: string,
  extraArgs: string[],
  host: ResolverHost,
): ResolvedServer | undefined {
  const configured = configuredPath.trim();
  if (configured) {
    return { command: configured, args: [...subcommandFor(configured), ...extraArgs] };
  }

  // Prefer the dedicated server binary, then the CLI's `lsp` subcommand.
  const lsp = findOnPath("mainstage-lsp", host) ?? findInInstallDirs("mainstage-lsp", host);
  if (lsp) {
    return { command: lsp, args: [...extraArgs] };
  }

  const cli = findOnPath("mainstage", host) ?? findInInstallDirs("mainstage", host);
  if (cli) {
    return { command: cli, args: ["lsp", ...extraArgs] };
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

/** Candidate filenames for an executable `name` on the given platform. */
export function executableNames(name: string, platform: NodeJS.Platform): string[] {
  return platform === "win32" ? [`${name}.exe`, `${name}.cmd`, name] : [name];
}

/** The first executable named `name` found across the entries of `PATH`. */
export function findOnPath(name: string, host: ResolverHost): string | undefined {
  if (!host.pathVar) {
    return undefined;
  }
  for (const dir of host.pathVar.split(delimiter)) {
    if (!dir) {
      continue;
    }
    const found = firstExecutable(dir, name, host);
    if (found) {
      return found;
    }
  }
  return undefined;
}

/** The first executable named `name` found in a common install location. */
export function findInInstallDirs(name: string, host: ResolverHost): string | undefined {
  const dirs =
    host.platform === "win32"
      ? [join(host.home, ".cargo", "bin")]
      : [
          join(host.home, ".local", "bin"),
          join(host.home, ".cargo", "bin"),
          "/usr/local/bin",
          "/opt/homebrew/bin",
          "/usr/bin",
        ];

  for (const dir of dirs) {
    const found = firstExecutable(dir, name, host);
    if (found) {
      return found;
    }
  }
  return undefined;
}

function firstExecutable(dir: string, name: string, host: ResolverHost): string | undefined {
  for (const candidate of executableNames(name, host.platform)) {
    const full = join(dir, candidate);
    if (host.isExecutable(full)) {
      return full;
    }
  }
  return undefined;
}
