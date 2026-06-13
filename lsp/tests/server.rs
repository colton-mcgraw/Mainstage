//! Phase 22 integration tests — the language server, end to end.
//!
//! These drive a real [`LspService`] the way an editor would: JSON-RPC requests in,
//! responses (and server-to-client `publishDiagnostics` notifications) out. They
//! cover the server lifecycle, live diagnostics (and their clearing), and the
//! language features — completion, hover, signature help, and formatting — so the
//! wiring in `server.rs` is exercised, not just the per-feature helpers.

use std::time::Duration;

use futures::StreamExt;
use mainstage_lsp::Backend;
use serde_json::{Value, json};
use tower::{Service, ServiceExt};
use tower_lsp::lsp_types::*;
use tower_lsp::{ClientSocket, LspService, jsonrpc::Request};

/// A test client wrapping an [`LspService`] and its server-to-client socket.
struct Harness {
    service: LspService<Backend>,
    socket: ClientSocket,
    next_id: i64,
}

impl Harness {
    fn new() -> Self {
        let (service, socket) = LspService::new(Backend::new);
        Self { service, socket, next_id: 0 }
    }

    /// Send a JSON-RPC request and return its successful result value. A `Null`
    /// `params` is sent as no params (some requests, e.g. `shutdown`, reject params).
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let builder = Request::build(method.to_string()).id(self.next_id);
        let builder = if params.is_null() { builder } else { builder.params(params) };
        let req = builder.finish();
        let response = self
            .service
            .ready()
            .await
            .expect("service ready")
            .call(req)
            .await
            .expect("service call")
            .expect("request yields a response");
        let (_id, result) = response.into_parts();
        result.expect("successful result")
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn notify(&mut self, method: &str, params: Value) {
        let req = Request::build(method.to_string()).params(params).finish();
        let response = self
            .service
            .ready()
            .await
            .expect("service ready")
            .call(req)
            .await
            .expect("service call");
        assert!(response.is_none(), "{method} is a notification and must not respond");
    }

    /// Complete the `initialize` / `initialized` handshake and return the advertised
    /// server capabilities.
    async fn initialize(&mut self) -> ServerCapabilities {
        let result = self.request("initialize", json!({ "capabilities": {} })).await;
        let init: InitializeResult = serde_json::from_value(result).unwrap();
        self.notify("initialized", json!({})).await;
        init.capabilities
    }

    /// Open `text` at `uri` (version 1).
    async fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "mainstage",
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await;
    }

    /// Replace the whole document at `uri` with `text` (full-sync change).
    async fn change(&mut self, uri: &str, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [ { "text": text } ],
            }),
        )
        .await;
    }

    /// Wait for the next `publishDiagnostics` notification the server emits.
    async fn next_diagnostics(&mut self) -> PublishDiagnosticsParams {
        let socket = &mut self.socket;
        let wait = async {
            while let Some(req) = socket.next().await {
                if req.method() == "textDocument/publishDiagnostics" {
                    let params = req.params().cloned().expect("diagnostics carry params");
                    return serde_json::from_value(params).expect("valid PublishDiagnosticsParams");
                }
            }
            panic!("socket closed before diagnostics arrived");
        };
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .expect("timed out waiting for diagnostics")
    }
}

const URI: &str = "file:///tmp/mainstage_test.ms";

#[tokio::test]
async fn initialize_advertises_every_capability() {
    let mut h = Harness::new();
    let caps = h.initialize().await;

    assert!(
        matches!(
            caps.text_document_sync,
            Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL))
        ),
        "full-document sync"
    );
    assert!(caps.completion_provider.is_some(), "completion");
    assert!(caps.hover_provider.is_some(), "hover");
    assert!(caps.signature_help_provider.is_some(), "signature help");
    assert!(matches!(caps.definition_provider, Some(OneOf::Left(true))), "go-to-definition");
    assert!(matches!(caps.references_provider, Some(OneOf::Left(true))), "find references");
    assert!(matches!(caps.document_symbol_provider, Some(OneOf::Left(true))), "symbols");
    assert!(
        matches!(caps.document_formatting_provider, Some(OneOf::Left(true))),
        "document formatting"
    );

    // The lifecycle terminates cleanly.
    let shutdown = h.request("shutdown", json!(null)).await;
    assert_eq!(shutdown, Value::Null);
}

#[tokio::test]
async fn diagnostics_are_published_and_then_cleared() {
    let mut h = Harness::new();
    h.initialize().await;

    // An invalid document surfaces at least one diagnostic.
    h.open(URI, "let x = ;").await;
    let published = h.next_diagnostics().await;
    assert_eq!(published.uri.as_str(), URI);
    assert!(!published.diagnostics.is_empty(), "invalid source should report diagnostics");

    // Fixing it republishes an empty set, clearing the stale squiggles.
    h.change(URI, 2, "let x = 1;").await;
    let cleared = h.next_diagnostics().await;
    assert!(cleared.diagnostics.is_empty(), "valid source should clear diagnostics");
}

#[tokio::test]
async fn completion_offers_module_methods_after_dot() {
    let mut h = Harness::new();
    h.initialize().await;
    // Mid-edit text (`git.` with no call): completion works off a lexical scan.
    let text = "import \"git\" as git;\nlet v = git.";
    h.open(URI, text).await;

    let result = h
        .request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 1, "character": 12 },
            }),
        )
        .await;

    let labels = completion_labels(&result);
    assert!(labels.iter().any(|l| l == "sha"), "git.sha should be offered, got {labels:?}");
    assert!(labels.iter().any(|l| l == "tag"), "git.tag should be offered, got {labels:?}");
}

#[tokio::test]
async fn completion_survives_an_unparseable_keystroke() {
    let mut h = Harness::new();
    h.initialize().await;

    // A valid document establishes a parse the server can cache.
    let valid = "project {\n    name: \"demo\"\n}\nlet v = \"\";";
    h.open(URI, valid).await;

    // Typing `project.` makes the document momentarily unparseable. Member
    // completion still offers the project field from the last good parse.
    let editing = "project {\n    name: \"demo\"\n}\nlet v = project.";
    h.change(URI, 2, editing).await;

    let result = h
        .request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 3, "character": 16 },
            }),
        )
        .await;

    let labels = completion_labels(&result);
    assert!(labels.iter().any(|l| l == "name"), "project.name should be offered, got {labels:?}");
}

#[tokio::test]
async fn hover_shows_method_signature() {
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "import \"git\" as git;\nlet v = git.sha();").await;

    // Hover over `sha` in the call.
    let result = h
        .request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 1, "character": 13 },
            }),
        )
        .await;

    let hover: Hover = serde_json::from_value(result).expect("hover payload");
    let text = hover_text(&hover.contents);
    assert!(text.contains("sha"), "hover should mention the method, got: {text}");
}

#[tokio::test]
async fn signature_help_inside_call_parens() {
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "import \"env\" as env;\nlet v = env.get();").await;

    // Cursor between the parens of `env.get(|)`.
    let result = h
        .request(
            "textDocument/signatureHelp",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 1, "character": 16 },
            }),
        )
        .await;

    let help: SignatureHelp = serde_json::from_value(result).expect("signature help payload");
    assert!(!help.signatures.is_empty(), "a signature should be offered");
    assert!(
        help.signatures[0].label.contains("get"),
        "signature should describe env.get, got: {}",
        help.signatures[0].label
    );
}

#[tokio::test]
async fn completion_offers_let_bindings_in_expression_position() {
    // Reproduces "suggestions aren't working": completion in a plain expression
    // position (not after a `.`) must offer the document's `let` variables.
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "let sources = \"x\";\nlet out = sources;").await;

    // Cursor inside the `sources` reference on line 1 (an expression position).
    let result = h
        .request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 1, "character": 11 },
            }),
        )
        .await;

    let labels = completion_labels(&result);
    assert!(
        labels.iter().any(|l| l == "sources"),
        "the `sources` let binding should be suggested, got {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "platform"),
        "the `platform` built-in should be suggested, got {labels:?}"
    );
}

#[tokio::test]
async fn hover_shows_leading_doc_comment() {
    // Reproduces "comments don't show in hover": a comment directly above a `let`
    // is surfaced as documentation when hovering a reference to that binding.
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "// the build output directory\nlet out = \"dist\";\nlet mirror = out;").await;

    // Hover the `out` reference on line 2 (`let mirror = out;`).
    let result = h
        .request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 2, "character": 14 },
            }),
        )
        .await;

    let hover: Hover = serde_json::from_value(result).expect("hover payload");
    let text = hover_text(&hover.contents);
    assert!(
        text.contains("the build output directory"),
        "hover should surface the leading comment, got: {text}"
    );
    assert!(text.contains("let out = \"dist\""), "hover should show the binding, got: {text}");
}

#[tokio::test]
async fn goto_definition_jumps_to_a_let_declaration() {
    // Backs "highlighting variables": the server resolves a variable reference to
    // its declaration site.
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "let out = \"dist\";\nlet mirror = out;").await;

    // Go to definition from the `out` reference on line 1.
    let result = h
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 1, "character": 14 },
            }),
        )
        .await;

    let response: GotoDefinitionResponse =
        serde_json::from_value(result).expect("definition payload");
    let location = match response {
        GotoDefinitionResponse::Scalar(loc) => loc,
        other => panic!("expected a single definition location, got {other:?}"),
    };
    assert_eq!(location.uri.as_str(), URI);
    // The declared `out` sits at line 0, character 4 (`let out = …`).
    assert_eq!(location.range.start, Position::new(0, 4));
}

#[tokio::test]
async fn find_references_returns_every_use_of_a_variable() {
    // Backs "highlighting variables": find-references collects the declaration and
    // both uses of a `let` binding so the editor can highlight them.
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "let name = \"demo\";\nlet a = name;\nlet b = name;").await;

    let result = h
        .request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": URI },
                "position": { "line": 0, "character": 4 },
                "context": { "includeDeclaration": true },
            }),
        )
        .await;

    let locations: Vec<Location> = serde_json::from_value(result).expect("references payload");
    assert_eq!(locations.len(), 3, "declaration plus two uses, got {locations:?}");
    assert!(locations.iter().all(|loc| loc.uri.as_str() == URI));
}

#[tokio::test]
async fn document_symbols_classify_a_let_as_a_variable() {
    // Backs "highlighting variables": the outline reports `let` bindings as
    // variables, which is what editors use to colorize and list them.
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "let out = \"dist\";\nstage build {\n    steps {\n        $ echo hi\n    }\n}")
        .await;

    let result =
        h.request("textDocument/documentSymbol", json!({ "textDocument": { "uri": URI } })).await;

    let response: DocumentSymbolResponse =
        serde_json::from_value(result).expect("document symbols payload");
    let symbols = match response {
        DocumentSymbolResponse::Nested(symbols) => symbols,
        DocumentSymbolResponse::Flat(_) => panic!("expected nested document symbols"),
    };
    let out = symbols.iter().find(|s| s.name == "out").expect("the `out` symbol");
    assert_eq!(out.kind, SymbolKind::VARIABLE, "a `let` binding is a variable");
}

#[tokio::test]
async fn formatting_returns_a_full_document_edit() {
    let mut h = Harness::new();
    h.initialize().await;
    h.open(URI, "import   \"git\"   as   git ;\nlet  x=1 ;").await;

    let result = h
        .request(
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": URI },
                "options": { "tabSize": 4, "insertSpaces": true },
            }),
        )
        .await;

    let edits: Vec<TextEdit> = serde_json::from_value(result).expect("formatting edits");
    assert_eq!(edits.len(), 1, "one whole-document edit");
    assert_eq!(edits[0].new_text, "import \"git\" as git;\nlet x = 1;\n");
}

// ── Helpers ──────────────────────────────────────────────────────────────────────

/// Extract completion labels from a `textDocument/completion` result, which may be a
/// bare array or a `CompletionList`.
fn completion_labels(result: &Value) -> Vec<String> {
    let response: CompletionResponse =
        serde_json::from_value(result.clone()).expect("completion payload");
    let items = match response {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    };
    items.into_iter().map(|i| i.label).collect()
}

/// Flatten hover contents to a single searchable string.
fn hover_text(contents: &HoverContents) -> String {
    match contents {
        HoverContents::Markup(m) => m.value.clone(),
        HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
        HoverContents::Scalar(MarkedString::LanguageString(ls)) => ls.value.clone(),
        HoverContents::Array(items) => items
            .iter()
            .map(|m| match m {
                MarkedString::String(s) => s.clone(),
                MarkedString::LanguageString(ls) => ls.value.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
