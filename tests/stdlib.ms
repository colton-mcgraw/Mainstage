// Standard-library showcase (Phase 15).
//
// Exercises the pure / read-only standard-library modules together with the new
// integer and boolean literal types. Every binding below is evaluated at load time
// (`let` and `project` blocks run during `eval_program`), so analyzing *and*
// evaluating this file is enough to prove the calls succeed — no pipeline run needed.
//
// This file is consumed by `core/tests/examples.rs`; keep all calls valid.
import "str" as str;
import "path" as path;
import "hash" as hash;
import "json" as json;
import "fs" as fs;

project {
    name: "stdlib-demo"
    version: "1.0.0"
}

// ── Integer & boolean literals (new in Phase 15) ──────────────────────────────────
let year = 2026; // positive integer literal
let offset = -5; // negative integer literal
let ports = [8080, 8081]; // integers inside a list literal
let verbose = true; // boolean literal
let port_url = "http://localhost:${year}"; // ints render as decimals in interpolation

// ── str ───────────────────────────────────────────────────────────────────────────
let shout = str.upper("mainstage"); // "MAINSTAGE"
let slug = str.replace("a b c", " ", "-"); // "a-b-c"
let has_stage = str.contains("mainstage", "stage"); // true
let parts = str.split("x,y,z", ","); // ["x", "y", "z"]
let rejoined = str.join(parts, "/"); // "x/y/z"

// ── path ──────────────────────────────────────────────────────────────────────────
let out_path = path.join("dist", "app"); // "dist/app"
let base = path.base("src/main.rs"); // "main.rs"
let renamed = path.with_ext("src/main.rs", "o"); // "src/main.o"

// ── hash ──────────────────────────────────────────────────────────────────────────
let digest = hash.sha256("hello");

// ── json (opaque-string form) ─────────────────────────────────────────────────────
let app_name = json.get(fs.read("data/sample.json"), "name"); // "demo"
let app_port = json.get(fs.read("data/sample.json"), "port"); // "8080"
let feature0 = json.get(fs.read("data/sample.json"), "features.0"); // "build"

// ── fs (read-only) ─────────────────────────────────────────────────────────────────
let present = fs.exists("data/sample.json"); // true
let is_file = fs.is_file("data/sample.json"); // true

// ── General comparison conditions (Phase 41) ──────────────────────────────────────
// A value already held in a `let` can drive an `if` directly — no `env(...)` detour.
let release = if project.version == "1.0.0" { "ga" } else { "pre" }; // "ga"
let mode = if "stage" in parts { "found" } else { "missing" }; // "missing" (parts = [x,y,z])
let tagged = if shout contains "STAGE" { "yes" } else { "no" }; // "yes"
let no_ports = if empty(ports) { "none" } else { "some" }; // "some"
