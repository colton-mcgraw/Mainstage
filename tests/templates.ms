// Phase 46 — Reusable Step Templates example.
//
// A `template` names a shared run of steps; a `use` step inlines it. Here a single
// `banner` template is shared by two stages, so the common setup lines are authored once
// instead of copied. Templates are inlined and dropped before the build runs — they
// factor over *steps*, complementing `matrix` (which parameterizes a stage over values).
//
// Run it with `mainstage run build`.
project {
    name: "demo"
}

default pipeline build {
    stages: [frontend, backend]
}

// The shared steps, authored once and inlined wherever `use banner;` appears.
template banner {
    log "── ${project.name} ──"
    log "building on ${platform}"
}

stage frontend {
    always_run: true

    steps {
        use banner;
        log "compiling frontend"
    }
}

stage backend {
    always_run: true

    steps {
        use banner;
        log "compiling backend"
    }
}
