// The frontend component, included by ../main.ms. Its `glob` resolves against
// *this* file's directory (examples/multi-file/components), so it picks up the
// stylesheet sitting next to it under `assets/` — not one relative to the root.
stage frontend {
    inputs: glob("assets/*.css")
    outputs: ["dist/frontend.txt"]

    steps {
        mkdir "dist/"
        write "dist/frontend.txt" content: "frontend built"
    }
}
