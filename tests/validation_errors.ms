// Intentionally INVALID script (Phase 15).
//
// Every call below is rejected during semantic analysis — before any evaluation.
// `core/tests/examples.rs` analyzes this file and asserts the analysis fails, so the
// errors here are the point: do not "fix" them.
//
// Each line is annotated with the diagnostic it is expected to produce.
import "str" as str;

// Integer literal passed where a string parameter is declared.
let a = str.upper(42); // str.upper positional argument 1 must be string, found int

// Method that does not exist on the module.
let b = str.nonexistent("x"); // module 'str' has no method 'nonexistent'

// Too many positional arguments.
let c = str.upper("x", "y"); // str.upper expects 1 positional argument(s), found 2

// Use of a module that was never imported.
let d = path.base("src/main.rs"); // use of undeclared module 'path'
