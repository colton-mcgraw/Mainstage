import { accessSync, constants } from "node:fs";
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

import { resolveServer as resolve, ResolvedServer, ResolverHost } from "./serverResolver";

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

/**
 * Resolve the language server command from the current configuration and host,
 * delegating to the testable {@link resolve} helper. Returns `undefined` when no
 * server can be located.
 */
function resolveServer(): ResolvedServer | undefined {
  const config = workspace.getConfiguration("mainstage");
  const extraArgs = config.get<string[]>("server.arguments", []);
  const configured = config.get<string>("server.path", "");

  const host: ResolverHost = {
    platform: process.platform,
    pathVar: process.env.PATH,
    home: homedir(),
    isExecutable,
  };
  return resolve(configured, extraArgs, host);
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
