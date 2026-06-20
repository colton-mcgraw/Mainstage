// The backend component, included by ../main.ms. A plain stage with no inputs of
// its own; the root's `bundle` stage depends on its outputs.
stage backend {
    outputs: ["dist/backend.txt"]

    steps {
        mkdir "dist/"
        write "dist/backend.txt" content: "backend built"
    }
}
