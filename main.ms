import "env" as env;
import "git" as git;

project {
    name:    "my-app"
    version: git.tag(default: "0.0.0")
}

let sources = glob("src/**/*.rs");
let out     = env.get("OUT_DIR", default: "dist");
let target  = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};

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
    inputs:  sources
    outputs: ["target/${target}/release/my-app"]

    steps {
        $ cargo build --release --target ${target}
    }

    on_failure {
        delete "target/"
    }
}

stage lint {
    inputs:        sources
    allow_failure: true

    steps {
        $ cargo clippy
    }
}

stage test {
    inputs: sources

    steps {
        $ cargo test
    }
}

stage package {
    inputs:  [compile.outputs]
    outputs: ["${out}/${project.name}-${project.version}.tar.gz"]

    steps {
        mkdir "${out}/"
        write "${out}/VERSION" content: "${project.version}"
        $ tar -czf "${outputs[0]}" "${out}/"
    }
}