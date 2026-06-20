// Phase 43 — Diagnostic & Control-Flow Steps (`log` / `fail`).
//
// `log "<msg>"` prints an interpolated progress message; it routes through the runner's
// reporter, so it honors `--quiet` and is captured in the per-stage buffered output just
// like a `$` command's output. `fail "<reason>"` fails the enclosing stage deliberately —
// it behaves like any other failed step, so a `fail` inside `try` is swallowed and a
// stage's `on_failure` block fires. Run it with `mainstage run diagnostics`.
project {
    name: "demo"
}

default pipeline diagnostics {
    stages: [setup]
}

stage setup {
    always_run: true

    steps {
        log "preparing ${project.name}"
        // A deliberately-failed optional step is swallowed by `try`, so the stage still
        // succeeds and the following steps run.
        try {
            fail "optional refresh failed"
        }
        log "setup complete"
    }
}
