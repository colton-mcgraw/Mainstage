// mainstage-lsp — Language Server Protocol server for Mainstage.
//
// The server logic lives in the `mainstage_lsp` library; this binary is a thin
// launcher so the same server can also be started via `mainstage lsp`.

fn main() {
    mainstage_lsp::run_stdio();
}
