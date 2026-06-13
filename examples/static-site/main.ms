// Builds a tiny static website: copies pages into `dist/`, rewrites each asset
// under a content-addressed name, and emits a manifest. Demonstrates `glob`,
// `for` loops over a fileset, the `hash` module inside interpolation, and a
// stage that depends on other stages' outputs.
import "hash" as hash;

project {
    name: "site"
    version: "0.2.0"
}

let pages = glob("src/pages/**/*.html");
let assets = glob("src/assets/**/*.{css,js}");

default pipeline build {
    stages: [pages, assets, manifest]
}

stage pages {
    inputs: pages
    outputs: ["dist/index.html"]

    steps {
        mkdir "dist/"
        for page in pages {
            copy "${page.path}" to "dist/${page.name}"
        }
    }
}

stage assets {
    inputs: assets
    outputs: ["dist/assets"]

    steps {
        mkdir "dist/assets/"
        // Content-addressed filename: <stem>.<sha>.<ext>
        for asset in assets {
            copy "${asset.path}" to "dist/assets/${asset.stem}.${hash.sha256_file(asset.path)}.${asset.ext}"
        }
    }
}

stage manifest {
    inputs: [pages.outputs, assets.outputs]
    outputs: ["dist/manifest.txt"]

    steps {
        write "dist/manifest.txt" content: "${project.name} ${project.version}"
    }
}
