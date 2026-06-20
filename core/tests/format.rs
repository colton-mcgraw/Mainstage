//! Phase 21 integration tests — the `mainstage format` engine.
//!
//! Two guarantees are exercised here. First, *idempotency and stability* over every
//! committed example script: `format(x)` re-parses and re-formats to itself, and the
//! result preserves every comment. Second, a *golden* fixture pins the canonical
//! layout (indentation, spacing, block structure) so style regressions are caught.

use std::path::{Path, PathBuf};

use mainstage_core::{Source, format};

/// The repo-root directory (one level above this crate's manifest dir).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// The committed `.ms` scripts used as formatter fixtures.
fn example_scripts() -> Vec<PathBuf> {
    let root = repo_root();
    vec![
        root.join("main.ms"),
        root.join("examples/multi-file/main.ms"),
        root.join("examples/multi-file/components/frontend.ms"),
        root.join("examples/multi-file/components/backend.ms"),
        root.join("tests/stdlib.ms"),
        root.join("tests/validation_errors.ms"),
        root.join("tests/diagnostics.ms"),
        root.join("tests/plugin/main.ms"),
        root.join("tests/templates.ms"),
    ]
}

fn format_str(src: &str) -> String {
    format(&Source::from_str("test.ms", src)).expect("should format")
}

/// Count `//` comment markers, ignoring those that appear inside string literals —
/// a coarse proxy for "no comment was dropped" across the committed scripts (none of
/// which place a `//` inside a string).
fn comment_lines(text: &str) -> usize {
    text.lines().filter(|l| l.trim_start().starts_with("//") || l.contains("// ")).count()
}

#[test]
fn example_scripts_format_idempotently() {
    for path in example_scripts() {
        let source = Source::from_file(&path).expect("example file should exist");
        let once = format(&source).expect("example should format");
        let twice = format(&Source::from_str(path.clone(), once.clone()))
            .expect("formatted output should re-format");
        assert_eq!(once, twice, "formatting is not idempotent for {}", path.display());
    }
}

#[test]
fn example_scripts_preserve_every_comment() {
    for path in example_scripts() {
        let source = Source::from_file(&path).expect("example file should exist");
        let before = comment_lines(&source.text);
        let after = comment_lines(&format(&source).expect("example should format"));
        assert_eq!(before, after, "comments were dropped formatting {}", path.display());
    }
}

#[test]
fn formatted_output_ends_with_single_newline() {
    let out = format_str("let x = 1;");
    assert!(out.ends_with('\n'));
    assert!(!out.ends_with("\n\n"));
}

/// Golden fixture: a deliberately messy script and its canonical form. Pins the
/// house style end-to-end — imports, `let`, `project`, `stage`, `pipeline`, `steps`,
/// expressions, conditions, and comment placement.
const MESSY: &str = r#"// build config
import   "env"  as  env ;


import "git" as git;
let   target=if platform=="windows"{"win"}else{"nix"};
project{name:"app",version:git.tag(default:"0.0.0")}
default pipeline   dev{stages:[ a , b ]
on_success{
$ echo done
}}
stage   build{inputs:glob("src/*.rs") outputs:["bin"] allow_failure:true
steps{
// build it
$ cargo build
for f in src{mkdir "${f.dir}"}
}}
"#;

const CANONICAL: &str = r#"// build config
import "env" as env;

import "git" as git;
let target = if platform == "windows" { "win" } else { "nix" };
project {
    name: "app"
    version: git.tag(default: "0.0.0")
}
default pipeline dev {
    stages: [a, b]

    on_success {
        $ echo done
    }
}
stage build {
    inputs: glob("src/*.rs")
    outputs: ["bin"]
    allow_failure: true

    steps {
        // build it
        $ cargo build
        for f in src {
            mkdir "${f.dir}"
        }
    }
}
"#;

#[test]
fn golden_canonical_layout() {
    assert_eq!(format_str(MESSY), CANONICAL);
}

#[test]
fn golden_output_is_stable() {
    // The canonical form is a fixed point of the formatter.
    assert_eq!(format_str(CANONICAL), CANONICAL);
}
