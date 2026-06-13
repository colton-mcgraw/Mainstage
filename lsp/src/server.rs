//! The language server backend and its stdio entry point.
//!
//! [`Backend`] implements [`tower_lsp::LanguageServer`], handling the server
//! lifecycle, full-document synchronization, live diagnostics, and the
//! language-feature requests (completion, hover, signature help). Open documents
//! are kept in an in-memory store and re-analyzed (debounced) on each edit;
//! feature requests parse the latest text on demand and consult a per-directory
//! [`ModuleRegistry`] cache so plugin processes spawn once and stay alive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mainstage_core::ModuleRegistry;
use mainstage_core::ast::Program;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tower_lsp::jsonrpc::Result as RpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::convert::to_lsp_diagnostics;
use crate::{analysis, completion, hover, navigation, signature};

/// How long to wait after the last edit before analyzing and publishing
/// diagnostics. Coalesces rapid keystrokes into a single analysis.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// An open document: its current full text and the editor's version number.
struct Document {
    text: String,
    version: i32,
}

/// The Mainstage language server backend.
pub struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, Document>>>,
    /// In-flight debounce tasks keyed by URI; a new edit aborts the previous.
    debounce: Mutex<HashMap<Url, JoinHandle<()>>>,
    /// Module registries keyed by script directory, so plugin discovery and the
    /// long-lived plugin processes happen once per directory rather than per
    /// request.
    registries: Mutex<HashMap<PathBuf, ModuleRegistry>>,
    /// The last successful parse of each open document. Feature requests fall
    /// back to it when the current text does not parse — which is the common
    /// case mid-keystroke (e.g. just after typing the `.` of `project.`), so
    /// completion and hover keep working instead of going blank.
    programs: Mutex<HashMap<Url, Program>>,
}

impl Backend {
    /// Construct a backend bound to `client` with empty state.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
            debounce: Mutex::new(HashMap::new()),
            registries: Mutex::new(HashMap::new()),
            programs: Mutex::new(HashMap::new()),
        }
    }

    /// The syntax tree for `uri` given its current `text`. A successful parse is
    /// cached and returned; a failing parse falls back to the last cached tree,
    /// so language features survive the transient unparseable states the editor
    /// streams while the user is typing.
    async fn program_for(&self, uri: &Url, text: &str) -> Option<Program> {
        match analysis::parse_text(text, &path_of(uri)) {
            Some(program) => {
                self.programs.lock().await.insert(uri.clone(), program.clone());
                Some(program)
            }
            None => self.programs.lock().await.get(uri).cloned(),
        }
    }

    /// The module registry for `script_dir`, building and caching it on first use.
    async fn registry_for(&self, script_dir: &Path) -> ModuleRegistry {
        let mut cache = self.registries.lock().await;
        if let Some(registry) = cache.get(script_dir) {
            return registry.clone();
        }
        let registry = analysis::build_registry(script_dir);
        cache.insert(script_dir.to_path_buf(), registry.clone());
        registry
    }

    /// The current text of `uri`, if open.
    async fn text_of(&self, uri: &Url) -> Option<String> {
        self.documents.read().await.get(uri).map(|doc| doc.text.clone())
    }

    /// Record `text` (at `version`) for `uri` and schedule a debounced analysis
    /// that publishes diagnostics, cancelling any previous pending analysis.
    async fn update(&self, uri: Url, text: String, version: i32) {
        // Refresh the last-good parse cache eagerly: every parseable edit updates
        // it, so a later unparseable keystroke can fall back to the prior tree.
        if let Some(program) = analysis::parse_text(&text, &path_of(&uri)) {
            self.programs.lock().await.insert(uri.clone(), program);
        }
        self.documents.write().await.insert(uri.clone(), Document { text, version });

        let registry = self.registry_for(&script_dir_of(&uri)).await;
        let client = self.client.clone();
        let documents = Arc::clone(&self.documents);
        let task_uri = uri.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;

            let (text, version) = match documents.read().await.get(&task_uri) {
                Some(doc) => (doc.text.clone(), doc.version),
                None => return, // closed while we waited
            };

            let analysis = analysis::analyze(&text, &path_of(&task_uri), &registry);
            let diagnostics = to_lsp_diagnostics(&task_uri, &text, &analysis.diagnostics);
            // Publishing the full current set (empty when valid) clears stale ones.
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
                completion_provider: Some(CompletionOptions {
                    // `.` for member access, `"` for `import "…"` module names.
                    trigger_characters: Some(vec![".".to_string(), "\"".to_string()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client.log_message(MessageType::INFO, "mainstage language server initialized").await;
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
            self.update(params.text_document.uri, change.text, params.text_document.version).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        self.programs.lock().await.remove(&uri);
        if let Some(handle) = self.debounce.lock().await.remove(&uri) {
            handle.abort();
        }
        // Clear any diagnostics the editor is still showing for the closed file.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        let pos = params.text_document_position;
        let Some(text) = self.text_of(&pos.text_document.uri).await else { return Ok(None) };
        let registry = self.registry_for(&script_dir_of(&pos.text_document.uri)).await;
        let program = self.program_for(&pos.text_document.uri, &text).await;
        let items = completion::completions(&text, pos.position, &registry, program.as_ref());
        Ok((!items.is_empty()).then_some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        let pos = params.text_document_position_params;
        let Some(text) = self.text_of(&pos.text_document.uri).await else { return Ok(None) };
        let registry = self.registry_for(&script_dir_of(&pos.text_document.uri)).await;
        let program = self.program_for(&pos.text_document.uri, &text).await;
        Ok(hover::hover(&text, pos.position, &registry, program.as_ref()))
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> RpcResult<Option<SignatureHelp>> {
        let pos = params.text_document_position_params;
        let Some(text) = self.text_of(&pos.text_document.uri).await else { return Ok(None) };
        let registry = self.registry_for(&script_dir_of(&pos.text_document.uri)).await;
        let program = self.program_for(&pos.text_document.uri, &text).await;
        Ok(signature::signature_help(&text, pos.position, &registry, program.as_ref()))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        let pos = params.text_document_position_params;
        let uri = pos.text_document.uri;
        let Some(text) = self.text_of(&uri).await else { return Ok(None) };
        let program = self.program_for(&uri, &text).await;
        Ok(navigation::definition(&text, pos.position, program.as_ref())
            .map(|range| GotoDefinitionResponse::Scalar(Location::new(uri, range))))
    }

    async fn references(&self, params: ReferenceParams) -> RpcResult<Option<Vec<Location>>> {
        let pos = params.text_document_position;
        let uri = pos.text_document.uri;
        let Some(text) = self.text_of(&uri).await else { return Ok(None) };
        let program = self.program_for(&uri, &text).await;
        let include = params.context.include_declaration;
        let locations: Vec<Location> =
            navigation::references(&text, pos.position, program.as_ref(), include)
                .into_iter()
                .map(|range| Location::new(uri.clone(), range))
                .collect();
        Ok((!locations.is_empty()).then_some(locations))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> RpcResult<Option<Vec<DocumentHighlight>>> {
        let pos = params.text_document_position_params;
        let uri = pos.text_document.uri;
        let Some(text) = self.text_of(&uri).await else { return Ok(None) };
        let program = self.program_for(&uri, &text).await;
        let highlights = navigation::highlights(&text, pos.position, program.as_ref());
        Ok((!highlights.is_empty()).then_some(highlights))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(text) = self.text_of(&uri).await else { return Ok(None) };
        let program = self.program_for(&uri, &text).await;
        let symbols = navigation::document_symbols(&text, program.as_ref());
        Ok((!symbols.is_empty()).then(|| {
            DocumentSymbolResponse::Nested(symbols.iter().map(to_document_symbol).collect())
        }))
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> RpcResult<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let Some(text) = self.text_of(&uri).await else { return Ok(None) };
        Ok(crate::format::formatting(&text, &path_of(&uri)))
    }
}

/// Convert a navigation [`Symbol`](navigation::Symbol) into an LSP `DocumentSymbol`.
fn to_document_symbol(symbol: &navigation::Symbol) -> DocumentSymbol {
    let kind = match symbol.kind {
        navigation::SymbolKind::Let => SymbolKind::VARIABLE,
        navigation::SymbolKind::Stage => SymbolKind::CLASS,
        navigation::SymbolKind::Pipeline => SymbolKind::FUNCTION,
    };
    #[allow(deprecated)] // `deprecated` field is mandatory but unused.
    DocumentSymbol {
        name: symbol.name.clone(),
        detail: None,
        kind,
        tags: None,
        deprecated: None,
        range: symbol.range,
        selection_range: symbol.selection,
        children: None,
    }
}

/// The filesystem path for a document `uri`, falling back to its raw path for
/// non-`file:` URIs (used only for span reporting).
fn path_of(uri: &Url) -> PathBuf {
    uri.to_file_path().unwrap_or_else(|_| PathBuf::from(uri.path()))
}

/// The script directory for a document `uri` — the root for plugin discovery.
fn script_dir_of(uri: &Url) -> PathBuf {
    path_of(uri).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

/// Run the language server over stdio until the client disconnects.
pub async fn serve() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
