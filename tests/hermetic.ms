// Phase 53 — Hermeticity & Reproducibility example.
//
// Demonstrates declared tool requirements (`requires`) and per-stage environment
// isolation (`hermetic`). Run it with `mainstage run build`. Two companion CLI flags
// pair with these stages and need no syntax of their own:
//
//   mainstage --check-reproducible run build   // run twice, diff output content
//   mainstage --audit-inputs run build         // warn about undeclared file reads
//
// See docs/HERMETICITY.md for the full description.
project {
    name: "demo"
    version: "1.0.0"
}

default pipeline build {
    stages: [generate, archive]
}

// `requires` is checked before the steps run: `sh` must be on PATH. A missing or
// version-mismatched tool fails the stage with a clear diagnostic instead of erroring
// partway through the steps.
stage generate {
    outputs: ["build/data.txt"]
    requires {
        "sh"
    }

    steps {
        mkdir "build"
        write "build/data.txt" content: "deterministic content"
    }
}

// `hermetic: true` runs the stage's commands with a cleared environment: only PATH (and
// the platform essentials) plus the explicit `with_env` overlay are visible, so the stage
// can't silently depend on an ambient variable. Combine with `--check-reproducible` to
// verify the archive is byte-identical across runs.
stage archive {
    inputs: [generate.outputs]
    outputs: ["build/archive.txt"]
    hermetic: true

    steps {
        with_env { LC_ALL: "C" } {
            $ sh -c "cat build/data.txt > build/archive.txt"
        }
    }
}
