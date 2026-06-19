# Benchmarks

Mainstage uses [criterion](https://github.com/bheisler/criterion.rs) to track the
performance of the language pipeline. The harness lives in
[`core/benches/pipeline.rs`](../core/benches/pipeline.rs) and exists so the
Performance & Scalability work (Goals 4, Phases 24–25) can prove measurable gains
against a recorded baseline.

## Running

```sh
# Full run (stable numbers, longer)
cargo bench -p mainstage_core --bench pipeline

# Quick run (faster, noisier)
cargo bench -p mainstage_core --bench pipeline -- --warm-up-time 1 --measurement-time 3
```

HTML reports are written to `target/criterion/`.

## Fixtures

Benchmarks run against synthetic projects produced by a generator parameterized by:

- **stage count** — total number of stages,
- **DAG depth** — how many layers the dependency graph has, and
- **files-per-stage** — input files each stage globs and hashes.

Stages are arranged into `depth` layers of width `ceil(stages / depth)`; each stage
depends on the stage directly above it in the previous layer, forming `width`
parallel dependency chains. Specs also carry a per-file **content size** (KiB); the
tiny default keeps front-end and cold benchmarks I/O-free, while the large-content
specs make per-file reading dominate so the Phase 25 fast path is measurable. The
covered sizes are:

| Label              | Stages | Depth | Files/stage | KiB/file |
| ------------------ | -----: | ----: | ----------: | -------: |
| `s10_d3_f5`        |     10 |     3 |           5 |    tiny  |
| `s50_d5_f10`       |     50 |     5 |          10 |    tiny  |
| `s100_d8_f20`      |    100 |     8 |          20 |    tiny  |
| `s30_d5_f8_k64`    |     30 |     5 |           8 |      64  |
| `s40_d5_f8_k128`   |     40 |     5 |           8 |     128  |

## Benchmark groups

| Group               | Measures                                                        | Relevant to |
| ------------------- | --------------------------------------------------------------- | ----------- |
| `parse`             | `parse` source → AST                                            | —           |
| `analyze`           | `analyze_with` (semantic analysis)                              | —           |
| `eval`              | `eval_program_with` (incl. glob resolution)                     | —           |
| `run_pipeline`      | cold end-to-end run, every stage executes (cache cleared)       | Phase 24    |
| `run_pipeline_warm` | warm run, every stage hits the skip-check (cache populated)     | Phase 25    |
| `run_pipeline_warm_large` | warm run over large input files — exposes the fast path   | Phase 25    |
| `run_pipeline_incremental_edit` | one input edited, `for`-loop re-runs only that file | Phase 38    |
| `run_pipeline_incremental_full` | same fixture, cache cleared — every iteration re-runs | Phase 38    |

The Phase 38 pair shares a single-stage `for file in inputs { copy … }` fixture
(`f<N>_k<KiB>`: N input files of KiB each), so the only difference is whether the
cache lets unchanged files' iterations be skipped.

## Baseline

Recorded on the Phase 23 reference machine with the **quick** settings above
(`--warm-up-time 1 --measurement-time 3`). Values are the criterion median estimate.
These numbers are hardware-specific — re-record locally before comparing.

| Benchmark                       | Median |
| ------------------------------- | ------ |
| `parse/s10_d3_f5`               | 912 µs |
| `parse/s50_d5_f10`              | 19.8 ms |
| `parse/s100_d8_f20`             | 78.2 ms |
| `analyze/s10_d3_f5`             | 6.06 µs |
| `analyze/s50_d5_f10`            | 42.5 µs |
| `analyze/s100_d8_f20`           | 97.5 µs |
| `eval/s10_d3_f5`                | 252 µs |
| `eval/s50_d5_f10`               | 2.13 ms |
| `eval/s100_d8_f20`              | 4.82 ms |
| `run_pipeline/s10_d3_f5`        | 1.66 ms |
| `run_pipeline/s50_d5_f10`       | 30.9 ms |
| `run_pipeline_warm/s50_d5_f10`  | 20.9 ms |
| `run_pipeline_warm/s100_d8_f20` | 145 ms |

Note: `parse` time grows super-linearly with script size, and `run_pipeline_warm`
(the change-detection skip path) dominates large warm runs — both are signals for
the optimization work in Phases 24–25.

## Phase 25 result: mtime/size fast path

Phase 25 short-circuits the per-file SHA-256 on the warm skip-check: when a file's
size and modification time match the cached fingerprint, its content hash is reused
instead of re-reading the file, and the remaining hashing is parallelized. The
benefit scales with file size — on the tiny-file `run_pipeline_warm` specs the
difference stays within noise, so the win is measured by `run_pipeline_warm_large`.

Measured before (legacy read+hash) and after (fast path) on the **same machine** with
the quick settings, atop the Phase 24 parallel runner. Re-record locally to compare.

| Benchmark                            | Before  | After   | Speedup |
| ------------------------------------ | ------- | ------- | ------- |
| `run_pipeline_warm_large/s30_d5_f8_k64`  (~15 MiB) | 5.83 ms | 2.91 ms | ~2.0× |
| `run_pipeline_warm_large/s40_d5_f8_k128` (~40 MiB) | 12.7 ms | 4.29 ms | ~3.0× |

The gap widens with input size, confirming the cost moved off the file-read path.
On unchanged inputs the warm run now scales with the number of files (one `stat`
each) rather than their total bytes.

## Phase 38 result: per-file incremental change detection

When a stage is not whole-stage fresh — some input changed — but it ran
successfully before and its declared outputs are all present, each `for file in
inputs { … }` iteration whose input file is byte-for-byte unchanged is skipped:
its output is already current. Editing one source therefore re-runs one iteration
instead of the whole stage.

Measured on the **same machine** with the default settings (`run_pipeline_incremental_*`
share one `for`-loop fixture; `_edit` changes a single file per run, `_full` clears
the cache so every iteration runs — the pre-Phase-38 whole-stage cost).

| Fixture (files × KiB) | Whole-stage (`_full`) | Single-file edit (`_edit`) | Speedup |
| --------------------- | --------------------- | -------------------------- | ------- |
| `f40_k64`  (40 × 64 KiB) | 19.3 ms | 1.39 ms | ~14× |
| `f80_k64`  (80 × 64 KiB) | 31.0 ms | 1.63 ms | ~19× |

The incremental run's cost is dominated by fingerprinting all inputs (one `stat`
each, via the Phase 25 fast path) plus the single changed file's work, so the win
grows with the number of unchanged files. The cache format is unchanged — per-file
metadata recorded since Phase 25 is exactly what the skip needs.

**Scope.** Incremental skipping associates each output with the single input file
that produced it (the `for file in inputs` iteration), so it assumes an output
depends only on its corresponding input — it does not track cross-file
dependencies (e.g. a shared header). Editing such a shared input changes the
stage's digest but not the per-file hash of the other sources, so their iterations
are still skipped; model genuinely shared inputs as their own stage, or run
`mainstage clean` to force a full rebuild.
