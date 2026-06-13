import { accessSync, constants } from "node:fs";
import { delimiter, join } from "node:path";
import { homedir } from "node:os";

import {
  commands,
  ExtensionContext,
  window,
  workspace,
  Uri,
} from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;
const outputChannel = window.createOutputChannel("Mainstage Language Server");

const INSTALL_DOCS = "https://github.com/ColtMcG1/mainstage#installation";

export async function activate(context: ExtensionContext): Promise<void> {
  context.subscriptions.push(
    outputChannel,
    commands.registerCommand("mainstage.restartServer", () => restart(context)),
    commands.registerCommand("mainstage.showOutput", () => outputChannel.show()),
  );

  // Restart automatically when the server location or arguments change.
  context.subscriptions.push(
    workspace.onDidChangeConfiguration((event) => {
      if (
        event.affectsConfiguration("mainstage.server.path") ||
        event.affectsConfiguration("mainstage.server.arguments")
      ) {
        void restart(context);
      }
    }),
  );

  await start(context);
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

async function start(context: ExtensionContext): Promise<void> {
  const server = resolveServer();
  if (!server) {
    await reportMissingServer();
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command: server.command, args: server.args, transport: TransportKind.stdio },
    debug: { command: server.command, args: server.args, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "mainstage" }],
    synchronize: {
      // The server reads `plugins.toml` for module discovery and permissions.
      fileEvents: workspace.createFileSystemWatcher("**/plugins.toml"),
    },
    outputChannel,
  };

  client = new LanguageClient(
    "mainstage",
    "Mainstage Language Server",
    serverOptions,
    clientOptions,
  );

  outputChannel.appendLine(
    `Starting Mainstage language server: ${server.command} ${server.args.join(" ")}`,
  );

  try {
    await client.start();
    context.subscriptions.push(client);
  } catch (err) {
    outputChannel.appendLine(`Failed to start language server: ${String(err)}`);
    void window.showErrorMessage(
      "Mainstage language server failed to start. See the output for details.",
      "Show Output",
    ).then((choice) => {
      if (choice === "Show Output") {
        outputChannel.show();
      }
    });
  }
}

async function restart(context: ExtensionContext): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
  await start(context);
}

interface ResolvedServer {
  command: string;
  args: string[];
}

/**
 * Resolve the language server command without requiring any manual setup.
 *
 * Precedence:
 *   1. The `mainstage.server.path` setting, if non-empty.
 *   2. A `mainstage` or `mainstage-lsp` binary found on `PATH`.
 *   3. A `mainstage` or `mainstage-lsp` binary in a common install location.
 *
 * A `mainstage` binary is launched as `mainstage lsp`; a dedicated
 * `mainstage-lsp` binary is launched directly. The configured
 * `mainstage.server.arguments` are appended in every case.
 */
function resolveServer(): ResolvedServer | undefined {
  const config = workspace.getConfiguration("mainstage");
  const extraArgs = config.get<string[]>("server.arguments", []);

  const configured = config.get<string>("server.path", "").trim();
  if (configured) {
    return { command: configured, args: [...subcommandFor(configured), ...extraArgs] };
  }

  // Prefer the dedicated server binary, then the CLI's `lsp` subcommand.
  const lsp = findOnPath("mainstage-lsp") ?? findInInstallDirs("mainstage-lsp");
  if (lsp) {
    return { command: lsp, args: [...extraArgs] };
  }

  const cli = findOnPath("mainstage") ?? findInInstallDirs("mainstage");
  if (cli) {
    return { command: cli, args: ["lsp", ...extraArgs] };
  }

  return undefined;
}

/** Decide the leading subcommand for a user-supplied executable path. */
function subcommandFor(command: string): string[] {
  const base = command.replace(/\\/g, "/").split("/").pop() ?? command;
  // A `mainstage-lsp` binary speaks LSP directly; the `mainstage` CLI needs `lsp`.
  return /^mainstage(\.exe)?$/i.test(base) ? ["lsp"] : [];
}

function executableNames(name: string): string[] {
  return process.platform === "win32" ? [`${name}.exe`, `${name}.cmd`, name] : [name];
}

function findOnPath(name: string): string | undefined {
  const path = process.env.PATH;
  if (!path) {
    return undefined;
  }
  for (const dir of path.split(delimiter)) {
    if (!dir) {
      continue;
    }
    const found = firstExecutable(dir, name);
    if (found) {
      return found;
    }
  }
  return undefined;
}

function findInInstallDirs(name: string): string | undefined {
  const home = homedir();
  const dirs =
    process.platform === "win32"
      ? [join(home, ".cargo", "bin")]
      : [
          join(home, ".local", "bin"),
          join(home, ".cargo", "bin"),
          "/usr/local/bin",
          "/opt/homebrew/bin",
          "/usr/bin",
        ];

  for (const dir of dirs) {
    const found = firstExecutable(dir, name);
    if (found) {
      return found;
    }
  }
  return undefined;
}

function firstExecutable(dir: string, name: string): string | undefined {
  for (const candidate of executableNames(name)) {
    const full = join(dir, candidate);
    if (isExecutable(full)) {
      return full;
    }
  }
  return undefined;
}

function isExecutable(file: string): boolean {
  try {
    accessSync(file, process.platform === "win32" ? constants.F_OK : constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

async function reportMissingServer(): Promise<void> {
  const message =
    "Could not find the Mainstage language server. Install the `mainstage` CLI " +
    "(which bundles `mainstage lsp`) or set `mainstage.server.path`.";
  outputChannel.appendLine(message);

  const choice = await window.showWarningMessage(
    message,
    "Install Instructions",
    "Open Settings",
  );
  if (choice === "Install Instructions") {
    void commands.executeCommand("vscode.open", Uri.parse(INSTALL_DOCS));
  } else if (choice === "Open Settings") {
    void commands.executeCommand(
      "workbench.action.openSettings",
      "mainstage.server.path",
    );
  }
}
