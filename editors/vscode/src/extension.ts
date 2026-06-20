import { accessSync, chmodSync, constants, existsSync } from "node:fs";
import { arch, platform } from "node:process";

import {
  commands,
  env,
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

import { resolveServer as resolve, ResolvedServer, ResolverHost } from "./serverResolver";

let client: LanguageClient | undefined;
const outputChannel = window.createOutputChannel("Mainstage Language Server");

const INSTALL_DOCS = "https://github.com/colton-mcgraw/mainstage#installation";

export async function activate(context: ExtensionContext): Promise<void> {
  context.subscriptions.push(
    outputChannel,
    commands.registerCommand("mainstage.restartServer", () => restart(context)),
    commands.registerCommand("mainstage.showOutput", () => outputChannel.show()),
  );

  // Log the environment up front: when something fails to start, the platform,
  // architecture, and whether we're inside a remote (WSL, container, SSH) are
  // the first things needed to diagnose a missing or mismatched server binary.
  outputChannel.appendLine(
    `Mainstage extension activating — host ${platform}-${arch}, ` +
      (env.remoteName ? `remote "${env.remoteName}"` : "local") +
      `, extension at ${context.extensionPath}`,
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
  const server = resolveServer(context);
  if (!server) {
    await reportMissingServer();
    return;
  }

  // VSIX extraction can drop the executable bit on POSIX hosts (the archive is a
  // zip), which surfaces in remote containers and WSL as an EACCES on spawn.
  // Repair it for the binary we ship before we try to launch it.
  if (server.bundled) {
    ensureExecutable(server.command);
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
    `Starting Mainstage language server (${server.bundled ? "bundled" : "configured"}): ` +
      `${server.command} ${server.args.join(" ")}`,
  );

  try {
    await client.start();
    context.subscriptions.push(client);
  } catch (err) {
    client = undefined;
    outputChannel.appendLine(`Failed to start language server: ${String(err)}`);
    outputChannel.appendLine(
      "If you are in a remote container, WSL, or over SSH, confirm the extension " +
        "is installed on the remote (its host arch must match the bundled binary), " +
        "or set `mainstage.server.path` to a server on the remote.",
    );
    void window
      .showErrorMessage(
        "Mainstage language server failed to start. See the output for details.",
        "Show Output",
      )
      .then((choice) => {
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

/**
 * Resolve the language server command from the current configuration and the
 * extension's install directory, delegating to the testable {@link resolve}
 * helper. Returns `undefined` when no server can be located.
 */
function resolveServer(context: ExtensionContext): ResolvedServer | undefined {
  const config = workspace.getConfiguration("mainstage");
  const extraArgs = config.get<string[]>("server.arguments", []);
  const configured = config.get<string>("server.path", "");

  const host: ResolverHost = {
    platform,
    extensionPath: context.extensionPath,
    fileExists: existsSync,
  };
  return resolve(configured, extraArgs, host);
}

/**
 * Ensure the bundled server is executable on POSIX hosts. A no-op on Windows
 * (no exec bit) and when the bit is already set; otherwise `chmod 0755`, logging
 * either the repair or a warning if it could not be applied.
 */
function ensureExecutable(file: string): void {
  if (platform === "win32") {
    return;
  }
  try {
    accessSync(file, constants.X_OK);
    return;
  } catch {
    // Fall through and attempt to set the bit.
  }
  try {
    chmodSync(file, 0o755);
    outputChannel.appendLine(`Marked bundled server executable: ${file}`);
  } catch (err) {
    outputChannel.appendLine(
      `Warning: could not mark the bundled server executable (${file}): ${String(err)}`,
    );
  }
}

async function reportMissingServer(): Promise<void> {
  const message =
    `The Mainstage extension did not include a language server for this platform ` +
    `(${platform}-${arch}). Install a platform-specific build of the extension, or ` +
    "set `mainstage.server.path` to a `mainstage` or `mainstage-lsp` executable.";
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
