//! Phase 20 integration tests — the trivia-preserving syntax layer.
//!
//! The headline guarantee is byte-for-byte round-tripping: lexing any source into
//! the lossless token stream and rendering it back must reproduce the original bytes
//! exactly. These golden tests run that guarantee over every committed example `.ms`
//! script, plus a comment-rich fixture that proves comments and blank-line grouping
//! survive (the example scripts carry no comments of their own).

use std::path::{Path, PathBuf};

use mainstage_core::ast::Item;
use mainstage_core::{CommentKind, Source, attach_trivia, comments, lex, parse, render};

/// The repo-root directory (one level above this crate's manifest dir).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// The committed `.ms` scripts used as golden round-trip fixtures.
fn example_scripts() -> Vec<PathBuf> {
    let root = repo_root();
    vec![
        root.join("main.ms"),
        root.join("tests/stdlib.ms"),
        root.join("tests/validation_errors.ms"),
        root.join("tests/plugin/main.ms"),
    ]
}

#[test]
fn example_scripts_round_trip_byte_for_byte() {
    for path in example_scripts() {
        let source = Source::from_file(&path).expect("example file should exist");
        let rendered = render(&lex(&source));
        assert_eq!(rendered, source.text, "lossless round-trip failed for {}", path.display());
    }
}

/// A script exercising every trivia case: a file-leading comment, blank-line
/// grouping between items, a standalone comment inside a block, an end-of-line
/// comment, a `//` inside a string, and a dangling comment at end of file.
const COMMENTED: &str = r#"// Build script for the demo app.

import "git" as git; // version control

project {
    name:    "demo"
    version: git.tag(default: "0.0.0") // fallback when untagged
}

// The default pipeline only compiles.
default pipeline dev {
    stages: [compile]
}

stage compile {
    steps {
        // Notify before building.
        $ echo "see http://example.com for docs"
    }
}
// end of file
"#;

#[test]
fn commented_script_round_trips_byte_for_byte() {
    let source = Source::from_str("commented.ms", COMMENTED);
    assert_eq!(render(&lex(&source)), COMMENTED);
}

#[test]
fn commented_script_classifies_every_comment() {
    let source = Source::from_str("commented.ms", COMMENTED);
    let cs = comments(&lex(&source));
    // The `//` inside the echo string is not a comment.
    let texts: Vec<&str> = cs.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "// Build script for the demo app.",
            "// version control",
            "// fallback when untagged",
            "// The default pipeline only compiles.",
            "// Notify before building.",
            "// end of file",
        ]
    );
    let kinds: Vec<CommentKind> = cs.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        vec![
            CommentKind::Standalone,
            CommentKind::EndOfLine,
            CommentKind::EndOfLine,
            CommentKind::Standalone,
            CommentKind::Standalone,
            CommentKind::Standalone,
        ]
    );
}

#[test]
fn commented_script_attaches_trivia_to_nodes() {
    let source = Source::from_str("commented.ms", COMMENTED);
    let program = parse(&source).expect("fixture should parse");
    let map = attach_trivia(&program, &lex(&source));

    // The leading file comment attaches to the first item (the import).
    let import = &program.items[0];
    let import_trivia = map.get(import.span()).expect("import carries trivia");
    assert_eq!(import_trivia.leading.len(), 1);
    assert_eq!(import_trivia.leading[0].text, "// Build script for the demo app.");
    assert_eq!(import_trivia.trailing.len(), 1);
    assert_eq!(import_trivia.trailing[0].text, "// version control");

    // The default-pipeline item carries a leading comment and a one-blank-line gap.
    let pipeline =
        program.items.iter().find(|i| matches!(i, Item::Pipeline(_))).expect("pipeline present");
    let pipeline_trivia = map.get(pipeline.span()).expect("pipeline carries trivia");
    assert_eq!(pipeline_trivia.leading[0].text, "// The default pipeline only compiles.");
    assert_eq!(pipeline_trivia.blank_lines_before, 1);

    // The standalone comment inside the steps block attaches to the exec step.
    let stage = program
        .items
        .iter()
        .find_map(|i| match i {
            Item::Stage(s) => Some(s),
            _ => None,
        })
        .expect("stage present");
    let step_trivia = map.get(stage.steps[0].span()).expect("step carries trivia");
    assert_eq!(step_trivia.leading[0].text, "// Notify before building.");
}
