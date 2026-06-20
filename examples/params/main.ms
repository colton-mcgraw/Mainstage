// Build parameters: typed, declared knobs that replace `env()`-for-everything.
//
// Each `param` has a default that can be overridden from the command line with
// `-D <name>=<value>` (repeatable), or from a `[params]` table in plugins.toml.
//
//     mainstage params                      # list params and their effective values
//     mainstage                             # run with the defaults below
//     mainstage run release -D release=true -D jobs=8
//     mainstage list                        # params are shown above the pipelines
project {
    name: "params-demo"
}

// A parameter is resolved once at load time, in declaration order alongside `let`,
// and is referenceable anywhere a `let` is.
param release: bool = false;
param target: string = "x86_64-unknown-linux-gnu";
param jobs: int = 4;
param features: list = ["default"];

// Parameters compose with the rest of the language exactly like a `let` binding.
let profile = if release == true { "release" } else { "debug" };

default pipeline build {
    stages: [compile]
}

pipeline release {
    stages: [compile]
}

stage compile {
    description: "Report the resolved build configuration"
    always_run: true

    steps {
        log "profile=${profile} target=${target} jobs=${jobs}"
        log "features=${features}"
    }
}
