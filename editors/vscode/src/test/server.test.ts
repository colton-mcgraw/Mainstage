import { after, before, describe, it } from "node:test";
import assert from "node:assert/strict";
import { spawn, ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

import {
  createProtocolConnection,
  ProtocolConnection,
  StreamMessageReader,
  StreamMessageWriter,
  InitializeRequest,
  InitializedNotification,
  ShutdownRequest,
  ExitNotification,
  DidOpenTextDocumentNotification,
  PublishDiagnosticsNotification,
  CompletionRequest,
  HoverRequest,
  DocumentHighlightRequest,
  DocumentHighlightKind,
  ServerCapabilities,
  CompletionItem,
  CompletionList,
  MarkupContent,
} from "vscode-languageserver-protocol/node";

/**
 * Locate the built language-server binary the editor would launch. Honors
 * `MAINSTAGE_LSP_BIN`; otherwise looks for the debug build at the workspace
 * root. Returns `undefined` (so the suite is skipped) when no binary exists.
 */
function serverBinary(): string | undefined {
  const override = process.env.MAINSTAGE_LSP_BIN;
  if (override) {
    return existsSync(override) ? override : undefined;
  }
  const exe = process.platform === "win32" ? "mainstage-lsp.exe" : "mainstage-lsp";
  // __dirname is editors/vscode/out/test → up four to the workspace root.
  const candidate = join(__dirname, "..", "..", "..", "..", "target", "debug", exe);
  return existsSync(candidate) ? candidate : undefined;
}

const BIN = serverBinary();
const URI = "file:///tmp/mainstage_extension_test.ms";

describe(
  "language server responses over the extension's transport",
  {
    skip: BIN
      ? false
      : "server binary not built — run `cargo build -p mainstage_lsp --bin mainstage-lsp` " +
        "or set MAINSTAGE_LSP_BIN",
  },
  () => {
    let child: ChildProcessWithoutNullStreams;
    let connection: ProtocolConnection;
    let capabilities: ServerCapabilities;

    before(async () => {
      // Spawn the real server the way `ServerOptions` does (stdio transport).
      child = spawn(BIN as string, [], { stdio: ["pipe", "pipe", "pipe"] });
      // Once the server exits during teardown (after `exit`), an in-flight protocol
      // write to its now-closed stdin would surface as an unhandled async EPIPE after
      // the test has ended. Swallow it — teardown waits for the process to exit below.
      child.stdin.on("error", () => {});
      connection = createProtocolConnection(
        new StreamMessageReader(child.stdout),
        new StreamMessageWriter(child.stdin),
      );
      connection.listen();

      const init = await connection.sendRequest(InitializeRequest.type, {
        processId: process.pid,
        rootUri: null,
        capabilities: {},
      });
      capabilities = init.capabilities;
      connection.sendNotification(InitializedNotification.type, {});
    });

    after(async () => {
      try {
        await connection.sendRequest(ShutdownRequest.type, undefined);
        connection.sendNotification(ExitNotification.type);
      } catch {
        // The server may already be gone; the kill below is the backstop.
      }
      connection?.dispose();
      // Wait for the process to actually exit so no stdio activity (e.g. an EPIPE on
      // the closing pipe) outlives this hook and trips node:test's after-test guard.
      await new Promise<void>((resolve) => {
        if (!child || child.exitCode !== null || child.signalCode !== null) {
          resolve();
          return;
        }
        const done = setTimeout(() => {
          child.kill("SIGKILL");
          resolve();
        }, 2000);
        child.once("close", () => {
          clearTimeout(done);
          resolve();
        });
        child.kill();
      });
    });

    /**
     * Open `text` at {@link URI} and resolve once the server has published
     * diagnostics for it — which guarantees the document is in the server's
     * store before we issue feature requests.
     */
    function openAndAnalyze(text: string): Promise<void> {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          handler.dispose();
          reject(new Error("timed out waiting for diagnostics"));
        }, 5000);
        const handler = connection.onNotification(
          PublishDiagnosticsNotification.type,
          (params) => {
            if (params.uri === URI) {
              clearTimeout(timer);
              handler.dispose();
              resolve();
            }
          },
        );
        connection.sendNotification(DidOpenTextDocumentNotification.type, {
          textDocument: { uri: URI, languageId: "mainstage", version: 1, text },
        });
      });
    }

    function completionLabels(
      result: CompletionItem[] | CompletionList | null,
    ): string[] {
      if (!result) {
        return [];
      }
      const items = Array.isArray(result) ? result : result.items;
      return items.map((i) => i.label);
    }

    it("advertises the capabilities the editor relies on", () => {
      assert.ok(capabilities.completionProvider, "completion");
      assert.ok(capabilities.hoverProvider, "hover");
      assert.ok(capabilities.signatureHelpProvider, "signature help");
      assert.ok(capabilities.documentHighlightProvider, "document highlight");
      assert.ok(capabilities.definitionProvider, "go-to-definition");
      assert.ok(capabilities.documentSymbolProvider, "document symbols");
    });

    it("returns module-method completions after a dot", async () => {
      await openAndAnalyze('import "git" as git;\nlet v = git.');
      const result = await connection.sendRequest(CompletionRequest.type, {
        textDocument: { uri: URI },
        position: { line: 1, character: 12 },
      });
      const labels = completionLabels(result);
      assert.ok(labels.includes("sha"), `expected git.sha, got ${JSON.stringify(labels)}`);
      assert.ok(labels.includes("tag"), `expected git.tag, got ${JSON.stringify(labels)}`);
    });

    it("suggests let-binding variables in expression position", async () => {
      await openAndAnalyze('let sources = "x";\nlet out = sources;');
      const result = await connection.sendRequest(CompletionRequest.type, {
        textDocument: { uri: URI },
        position: { line: 1, character: 11 },
      });
      const labels = completionLabels(result);
      assert.ok(labels.includes("sources"), `expected the variable, got ${JSON.stringify(labels)}`);
    });

    it("surfaces a binding's leading comment in hover", async () => {
      await openAndAnalyze('// the build output directory\nlet out = "dist";\nlet mirror = out;');
      const hover = await connection.sendRequest(HoverRequest.type, {
        textDocument: { uri: URI },
        position: { line: 2, character: 14 },
      });
      assert.ok(hover, "expected a hover response");
      const value = (hover.contents as MarkupContent).value;
      assert.ok(
        value.includes("the build output directory"),
        `hover should include the comment, got: ${value}`,
      );
      assert.ok(value.includes('let out = "dist"'), `hover should show the binding, got: ${value}`);
    });

    it("highlights every occurrence of a variable with read/write kinds", async () => {
      await openAndAnalyze('let name = "demo";\nlet a = name;\nlet b = name;');
      const highlights = await connection.sendRequest(DocumentHighlightRequest.type, {
        textDocument: { uri: URI },
        position: { line: 0, character: 4 },
      });
      assert.ok(highlights, "expected document highlights");
      assert.equal(highlights.length, 3, "declaration plus two uses");
      const writes = highlights.filter((h) => h.kind === DocumentHighlightKind.Write).length;
      const reads = highlights.filter((h) => h.kind === DocumentHighlightKind.Read).length;
      assert.equal(writes, 1, "the declaration is the lone write");
      assert.equal(reads, 2, "both uses are reads");
    });
  },
);
