//! The language server backend and its stdio entry point.
//!
//! [`Backend`] implements [`tower_lsp::LanguageServer`], handling the server
//! lifecycle (`initialize` / `initialized` / `shutdown`), full-document text
//! synchronization (`didOpen` / `didChange` / `didClose`), and live diagnostics.
//! Open documents are kept in an in-memory store; each edit re-runs the shared
//! analysis (debounced) and publishes the resulting diagnostics so the editor
//! shows parse and semantic errors as the user types.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tower_lsp::jsonrpc::Result as RpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::analysis::analyze_uri;
use crate::convert::to_lsp_diagnostics;

/// How long to wait after the last edit before analyzing and publishing
/// diagnostics. Coalesces rapid keystrokes into a single analysis.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// An open document: its current full text and the editor's version number.
struct Document {
    text: String,
    version: i32,
}

/// The Mainstage language server backend: the client handle, an in-memory store
/// of open documents keyed by URI, and the per-document debounce timers.
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, Document>>>,
    /// In-flight debounce tasks keyed by URI; a new edit aborts the previous.
    debounce: Mutex<HashMap<Url, JoinHandle<()>>>,
}

impl Backend {
    /// Construct a backend bound to `client` with empty state.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            debounce: Mutex::new(HashMap::new()),
        }
    }

    /// Record `text` (at `version`) for `uri` and schedule a debounced analysis
    /// that publishes diagnostics. Each call cancels the previous pending
    /// analysis for the same document so only the latest edit is analyzed.
    async fn update(&self, uri: Url, text: String, version: i32) {
        self.documents.write().await.insert(uri.clone(), Document { text, version });

        let client = self.client.clone();
        let documents = Arc::clone(&self.documents);
        let task_uri = uri.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;

            // Read the latest text for this document; if it was closed while we
            // waited, there is nothing to publish.
            let (text, version) = {
                let docs = documents.read().await;
                match docs.get(&task_uri) {
                    Some(doc) => (doc.text.clone(), doc.version),
                    None => return,
                }
            };

            // Analyze with the same parse → analyze_with pipeline and plugin-aware
            // registry the CLI uses, so import and plugin-call validation surface
            // in the editor exactly as they do on the command line.
            let analysis = analyze_uri(&task_uri, &text);
            let diagnostics = to_lsp_diagnostics(&task_uri, &text, &analysis.diagnostics);
            // Publishing the full current set (empty when the document is valid)
            // also clears any stale diagnostics from a previous edit.
            client.publish_diagnostics(task_uri, diagnostics, Some(version)).await;
        });

        if let Some(previous) = self.debounce.lock().await.insert(uri, handle) {
            previous.abort();
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> RpcResult<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "mainstage-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                // Full-document sync in V1; incremental sync is deferred.
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "mainstage language server initialized")
            .await;
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.update(doc.uri, doc.text, doc.version).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full-document sync: the last change carries the entire document text.
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.update(params.text_document.uri, change.text, params.text_document.version)
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        if let Some(handle) = self.debounce.lock().await.remove(&uri) {
            handle.abort();
        }
        // Clear any diagnostics the editor is still showing for the closed file.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

/// Run the language server over stdio until the client disconnects.
pub async fn serve() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
