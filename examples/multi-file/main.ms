// Multi-file composition with `include` (Phase 48). A root script pulls in one
// `.ms` file per component; the runtime flattens them into a single build graph,
// so a stage declared in one file can depend on a stage declared in another. All
// names share one flat namespace — a name defined in two files is an error.
//
//     mainstage list     # shows the merged pipeline and every stage
//     mainstage          # runs the default pipeline
project {
    name: "multi-file"
    version: "0.1.0"
}

// Each component lives in its own file, resolved relative to this one. A `glob`
// inside an included file matches files next to *that* file, not this root.
include "components/frontend.ms";
include "components/backend.ms";

default pipeline build {
    stages: [frontend, backend, bundle]
}

// The root wires the components together: `bundle` consumes both components'
// outputs, referencing stages declared in the included files by their bare names.
stage bundle {
    inputs: [frontend.outputs, backend.outputs]
    outputs: ["dist/bundle.txt"]

    steps {
        mkdir "dist/"
        write "dist/bundle.txt" content: "${project.name} ${project.version} bundle"
    }
}
