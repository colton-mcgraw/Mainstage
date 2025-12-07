# Test Organization for `core` crate

## Purpose

- Provide a clear, maintainable map of the existing integration tests in `core/tests` and a recommended organization for new tests.
- Provide a comprehensive list of tests to cover each stage of the compiler: parsing, semantic analysis, lowering, IR, optimization, bytecode emission, VM/runtime, and plugin/host ABI.

## Guiding principles

- Don't change Cargo's expectations unless we're intentionally changing layout: Cargo treats files directly under `tests/` as integration tests.
- We therefore KEEP test files at `core/tests/*.rs` to remain discoverable, but group them by a filename prefix and document categories.
- Add small helper runner scripts to run categories by name.

## Current tests (located in `core/tests/`)

- `util_read_glob_workdir.rs` : utils / IO (renamed)
- `opt_plugin_preserve.rs` : optimizer / plugin preservation
- `lowering_forin.rs` : lowering tests (for-in lowering)
- `lowering_calls.rs` : lowering tests (calls)
- `lowering_loop_exhaustive.rs` : lowering / loops exhaustive (renamed)
- `lowering_loop.rs` : lowering / loops (renamed)
- `ir_patch.rs` : IR patching / helpers
- `ir_lowering.rs` : IR lowering integration
- `emit_failing_bytecode.rs` : regression for bytecode emission failures (renamed)
- `emit_control_and_bytecode.rs` : control-flow + bytecode emission (renamed)
- `opt_const_canon_extern_vis.rs` : optimizer const-canonicalization & externally-visible remap (renamed)
- `util_template_test.rs` : contributor template (renamed)

## Recommended filename prefixes / categories

- `lowering_` : Lowering-related tests (AST -> IR lowering)
- `ir_` : IR-level tests and helpers
- `opt_` : Optimizer tests (const-fold, const-prop, const-canon, dce, fixed-point)
- `bytecode_` or `emit_` : Bytecode emission and correctness
- `vm_` or `runtime_` : VM execution, runtime semantics
- `plugin_` : Plugin/Host ABI tests and preservation
- `regress_` : Regression tests for past bugs / fuzzer findings
- `util_` : Utility and read/write tests (filesystem, globbing)

## Mapping of existing tests into categories (for readability)

- lowering_*: `lowering_forin.rs`, `lowering_calls.rs`, `loop_lowering.rs`, `loop_lowering_exhaustive.rs`, `ir_lowering.rs`
- ir_*: `ir_patch.rs`
- opt_*: `opt_plugin_preserve.rs`, `const_canon_extern_vis.rs`
- emit_/bytecode: `failing_emit_bytecode.rs`, `control_and_bytecode.rs`
- util_/io: `read_glob_workdir.rs`

## Comprehensive test checklist (by compiler stage)

- Lexing/Parsing
  - Positive: parse small programs (expressions, statements, declarations)
  - Round-trip: parse -> pretty-print -> parse again
  - Error cases: invalid tokens and location reporting
- AST / Semantic Analysis
  - Symbol resolution tests (shadowing, hoisting, duplicates)
  - Type checks or type-like contracts (if applicable)
  - Scoped variable lifetime and capture
  - Error diagnostics (message/position)
- Lowering (AST -> IR)
  - Statement lowering (if/else, loops, for-in, switch)
  - Call lowering (normal calls, host/plugin calls, tail calls)
  - Control flow lowering and label generation
  - Check that source-level semantics are preserved for small programs
- IR correctness
  - Instruction shapes, operand encodings
  - Label placement and jump targets
  - IR invariants (e.g., no use-before-define, writer lists)
  - Tests for IR helpers and patch functions (`ir_patch.rs`)
- Optimizer (each pass + pipeline)
  - Const-fold unit tests (pure ops)
  - Const-prop tests including local propagation across labels (guards at label boundaries)
  - Const-canonicalization: ensure remapping and `externally_visible_regs` rewrite
  - DCE: ensure plugin-visible producers and host ABI values preserved
  - Fixed-point pipeline tests that run multiple passes and assert stable ops
  - Regression tests where optimizer must NOT change semantics (e.g., self-referential fold cases)
- Bytecode emission
  - Bytecode for control + stack operations
  - Emission for host/plugin calls with proper ABI mapping
  - Regression tests for previously failing emits (`failing_emit_bytecode.rs`)
- VM / Runtime
  - Small program execution tests for arithmetic, control flow, arrays, objects
  - Host and plugin call behavior, error propagation
  - Memory safety invariants for arrays and objects
- Plugin / Host ABI
  - Plugin call preservation and producers (already `opt_plugin_preserve.rs`)
  - Tests for externally-visible register remapping (e.g., `const_canon_extern_vis.rs`)
  - FFI-style behavior like passing arrays/strings across ABI
- Integration & Regression
  - End-to-end scripts from `cli` that lower, optimize (optional), and run on the VM
  - Regression tests created from bug reports or fuzzer output
- Test utilities
  - Helpers for creating small IR modules, canonical patterns, and compare functions
  - Fixtures for sample scripts under `cli/samples` or `cli/examples` if present
