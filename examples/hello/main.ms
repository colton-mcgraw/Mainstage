// The smallest useful Mainstage script: one stage, one pipeline.
//
// Run it from this directory with:
//
//     mainstage          # runs the default pipeline (`build`)
//     mainstage list     # shows the pipeline and its stages
project {
    name: "hello"
    version: "1.0.0"
}

default pipeline build {
    stages: [greet]
}

stage greet {
    outputs: ["dist/hello.txt"]

    steps {
        mkdir "dist/"
        write "dist/hello.txt" content: "Hello from ${project.name} v${project.version}!"
    }
}
