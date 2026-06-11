//! The language server backend and its stdio entry point.
//!
//! [`Backend`] implements [`tower_lsp::LanguageServer`], handling the server
//! lifecycle (`initialize` / `initialized` / `shutdown`) and full-document text
//! synchronization (`didOpen` / `didChange` / `didClose`). Open documents are
//! kept in an in-memory store, re-analyzed on every edit so the server always
//! holds an up-to-date parsed view. Publishing diagnostics and other features
//! build on this foundation in later phases.

use std::collections::HashMap;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result as RpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::analysis::{analyze_uri, Analysis};

/// An open document: its current full text and the latest analysis of it.
struct Document {
    /// The full source text as last synced from the editor.
    #[allow(dead_code)]
    text: String,
    /// The parsed program and diagnostics from the most recent analysis.
    #[allow(dead_code)]
    analysis: Analysis,
}

/// The Mainstage language server backend: the client handle plus an in-memory
/// store of open documents keyed by URI.
pub struct Backend {
    #[allow(dead_code)]
    client: Client,
    documents: RwLock<HashMap<Url, Document>>,
}

impl Backend {
    /// Construct a backend bound to `client` with an empty document store.
    pub fn new(client: Client) -> Self {
        Self { client, documents: RwLock::new(HashMap::new()) }
    }

    /// Re-analyze `text` for `uri` and store the result, keeping the server's
    /// parsed view of the document current. Later phases publish the collected
    /// diagnostics; here the in-memory view is the only product.
    async fn refresh(&self, uri: Url, text: String) {
        let analysis = analyze_uri(&uri, &text);
        self.documents.write().await.insert(uri, Document { text, analysis });
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
        self.refresh(doc.uri, doc.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Full-document sync: the last change carries the entire document text.
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.refresh(params.text_document.uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.write().await.remove(&params.text_document.uri);
    }
}

/// Run the language server over stdio until the client disconnects.
pub async fn serve() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
