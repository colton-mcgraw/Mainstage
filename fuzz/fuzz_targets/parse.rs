//! cargo-fuzz target for the Mainstage front end.
//!
//! Asserts the parser/analyzer/evaluator never panic on arbitrary input. Run with a
//! nightly toolchain:
//!
//! ```sh
//! cargo install cargo-fuzz
//! cargo +nightly fuzz run parse
//! ```
//!
//! Step *execution* is intentionally excluded — a fuzzed program could contain a
//! destructive `$`/`write` step — so only parsing, analysis, and read-only program
//! evaluation are driven here, matching `core/tests/fuzz.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use mainstage_core::{Source, analyze, eval_program, parse};

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let source = Source::from_str("fuzz.ms", text);
        if let Ok(program) = parse(&source) {
            let _ = analyze(&program);
            let _ = eval_program(&program, std::path::Path::new("."));
        }
    }
});
