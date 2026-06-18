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
    stages: [compile, lint, test, package]

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
