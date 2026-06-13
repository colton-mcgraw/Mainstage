//! Phase 26 — panic-safety property tests.
//!
//! The runtime must never *panic* on arbitrary input: malformed scripts must surface
//! as `Err` diagnostics, never an `unwrap`/`unreachable!`/slice-out-of-bounds crash.
//! These tests stand in for (and complement) a `cargo-fuzz` target — see `fuzz/` — by
//! throwing tens of thousands of seeded-random and pathological inputs at the
//! input-processing front end (`parse` → `analyze` → `eval_program`) and asserting each
//! call returns rather than unwinds.
//!
//! Step *execution* is deliberately not fuzzed: a randomly generated program could
//! contain `$ rm -rf …` or a `write` to an arbitrary path, so running it would be unsafe.
//! Parsing, analysis, and program evaluation (which only resolve `let`/`project`, read
//! globs, and call read-only modules) cover the parser/lexer/evaluator panic surface.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use mainstage_core::{Source, analyze, eval_program, parse};

/// A tiny deterministic xorshift PRNG so the corpus is reproducible across runs.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Fragments drawn from the Mainstage surface syntax. Random sequences of these exercise
/// the parser far more deeply than random bytes, which it rejects almost immediately.
const TOKENS: &[&str] = &[
    "import",
    "as",
    "let",
    "project",
    "stage",
    "pipeline",
    "default",
    "steps",
    "on_failure",
    "on_success",
    "inputs",
    "outputs",
    "allow_failure",
    "if",
    "else",
    "for",
    "in",
    "platform",
    "glob(",
    "env",
    "git",
    "copy",
    "move",
    "mkdir",
    "delete",
    "write",
    "to",
    "content",
    "true",
    "false",
    "{",
    "}",
    "[",
    "]",
    "(",
    ")",
    ":",
    ",",
    ".",
    "=",
    "\"x\"",
    "\"${a}\"",
    "\"${a.b}\"",
    "\"unterminated",
    "name",
    "version",
    "123",
    "-5",
    "9999999999999999999999",
    "$",
    "$ cmd ${x}",
    "//c\n",
    "\n",
    " ",
    "\t",
    "@",
    "%",
    "&",
    "\\",
    ";",
    "..",
    "::",
    "-",
    "0x",
    "${",
    "}$",
];

/// Run the full front end on `src`, asserting it returns rather than panicking. The
/// result (`Ok`/`Err`) is irrelevant — only the absence of an unwind matters.
fn assert_no_panic(src: &str, dir: &std::path::Path) {
    let owned = src.to_string();
    let dir = dir.to_path_buf();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let source = Source::from_str("fuzz.ms", owned.clone());
        if let Ok(program) = parse(&source) {
            // Analysis is pure; program evaluation is read-only (globs + read-only modules).
            let _ = analyze(&program);
            let _ = eval_program(&program, &dir);
        }
    }));
    assert!(outcome.is_ok(), "input panicked the front end:\n{src:?}");
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_fuzz_{tag}_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn token_salad_never_panics() {
    let dir = temp_dir("tokens");
    // Quiet the panic hook so a genuine failure reports cleanly via the assert rather
    // than dumping a backtrace for every caught unwind.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut rng = Rng(0x9E3779B97F4A7C15);
    for _ in 0..20_000 {
        let len = rng.below(40);
        let mut src = String::new();
        for _ in 0..len {
            src.push_str(TOKENS[rng.below(TOKENS.len())]);
            if rng.below(3) == 0 {
                src.push(' ');
            }
        }
        assert_no_panic(&src, &dir);
    }

    std::panic::set_hook(prev);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn random_bytes_never_panic() {
    let dir = temp_dir("bytes");
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut rng = Rng(0xDEADBEEFCAFEBABE);
    for _ in 0..20_000 {
        let len = rng.below(64);
        // Build from arbitrary code points (lossy) to stress UTF-8 handling and spans.
        let mut src = String::new();
        for _ in 0..len {
            let cp = rng.below(0x110000);
            if let Some(c) = char::from_u32(cp as u32) {
                src.push(c);
            } else {
                src.push('?');
            }
        }
        assert_no_panic(&src, &dir);
    }

    std::panic::set_hook(prev);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pathological_corpus_never_panics() {
    let dir = temp_dir("corpus");
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    // Hand-picked inputs that probe specific edge cases: unbalanced delimiters, deep
    // nesting, integer overflow, empty constructs, unterminated strings/interpolations.
    let corpus = [
        "",
        " ",
        "\n\n\n",
        "{",
        "}",
        "stage",
        "stage {",
        "stage a {",
        "stage a { steps {",
        "let x =",
        "let x = ${",
        "let = glob(",
        "project { name: }",
        "default pipeline {",
        "pipeline p { stages: [",
        "let n = 99999999999999999999999999999;",
        "let n = -99999999999999999999999999999;",
        "stage a { steps { $ } }",
        "stage a { steps { write } }",
        "stage a { steps { copy to } }",
        "stage a { steps { for x in {} } }",
        "let s = \"${a.b.c.d.e}\";",
        "let s = \"no close ${a\";",
        "import \"\" as ;",
        "import as as as;",
        &"[".repeat(500),
        &"if true {".repeat(200),
        &format!("let s = \"{}\";", "x".repeat(5000)),
        "stage a { inputs: glob(\"**/*\") outputs: [] steps {} }",
        "🦀 stage 🦀 { 🦀 }",
        "let x = if platform == windows { } else { };",
    ];
    for src in corpus {
        assert_no_panic(src, &dir);
    }

    std::panic::set_hook(prev);
    let _ = std::fs::remove_dir_all(&dir);
}
