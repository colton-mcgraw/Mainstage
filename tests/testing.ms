// Phase 39 — Test Harness example.
//
// A `test` stage is never cached and tallies its `expect` / `assert` steps into a
// pass/fail count rather than collapsing to a single exit code. Every assertion runs
// (a failure does not short-circuit the stage), and the stage fails if any assertion
// fails. Run it with `mainstage run check`.
//
// This example uses only `assert` (value comparisons) so it is fully portable and runs
// without any external tools; see docs/GRAMMAR.md for the `expect` command-assertion
// forms (exit status and captured-output matching, with an optional timeout).
project {
    name: "demo"
    version: "1.2.0"
}

default pipeline check {
    stages: [unit]
}

// All assertions here hold, so the stage reports "tests: 6 passed".
stage unit {
    test: true

    steps {
        // Equality and substring matching, both with interpolation support.
        assert "${project.name}" equals "demo"
        assert "${project.version}" contains "1.2"
        assert "release-${project.name}" equals "release-demo"
        // Phase 45 matchers: absence, prefix/suffix, and an anchored glob pattern.
        assert "${project.name}" not_contains "debug"
        assert "${project.version}" starts_with "1."
        assert "${project.version}" matches "1.*.0"
    }
}
