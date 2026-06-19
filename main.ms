import "env" as env;
import "git" as git;

project {
    name: "my-app"
    version: git.tag(default: "0.0.0")
}

let sources = glob("src/**/*.rs");
let out = env.get("OUT_DIR", default: "dist");
let target = if platform == "windows" { "x86_64-pc-windows-msvc" } else { "x86_64-unknown-linux-gnu" };

default pipeline dev {
    stages: [compile]
}

pipeline release {
    stages: [compile, lint, test, package, bundle]

    on_success {
        $ slack-notify "Released ${project.version}"
    }
}

stage compile {
    inputs: sources
    outputs: ["target/${target}/release/my-app"]

    steps {
        $ cargo build --release --target ${target}
    }

    on_failure {
        delete "target/"
    }
}

stage lint {
    inputs: sources
    allow_failure: true

    steps {
        $ cargo clippy
    }
}

// Run the tests only after a successful build. `test` reads `sources`, not
// `compile.outputs`, so this ordering is not inferable from files — `depends_on`
// states it explicitly. Out-of-pipeline edges are ignored, so `dev` is unaffected.
stage test {
    inputs: sources
    depends_on: [compile]

    steps {
        $ cargo test
        // Coverage is best-effort — don't fail the build if the tool isn't installed.
        try {
            $ cargo tarpaulin --out Html
        }
    }
}

stage package {
    inputs: [compile.outputs]
    outputs: ["${out}/${project.name}-${project.version}.tar.gz"]

    steps {
        mkdir "${out}/"
        write "${out}/VERSION" content: "${project.version}"
        $ tar -czf "${outputs[0]}" "${out}/"
    }
}

// One authored stage, two architectures. `matrix` expands `bundle` into
// `bundle[x86_64]` and `bundle[aarch64]` before the build graph is built, and the
// active `${arch}` is exposed as a built-in — so per-target flags and paths stay in a
// single definition instead of copy-pasted stages. Listing the base name `bundle` in
// the `release` pipeline runs every variant.
stage bundle {
    matrix {
        arch: ["x86_64", "aarch64"]
    }
    inputs: [compile.outputs]
    outputs: ["${out}/${project.name}-${arch}.tar.gz"]

    steps {
        mkdir "${out}/"
        $ cargo build --release --target ${arch}-unknown-linux-gnu
        $ tar -czf "${out}/${project.name}-${arch}.tar.gz" "${out}/"
    }
}
