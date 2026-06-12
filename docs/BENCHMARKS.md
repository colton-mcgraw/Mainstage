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
parallel dependency chains. The three covered sizes are:

| Label             | Stages | Depth | Files/stage |
| ----------------- | -----: | ----: | ----------: |
| `s10_d3_f5`       |     10 |     3 |           5 |
| `s50_d5_f10`      |     50 |     5 |          10 |
| `s100_d8_f20`     |    100 |     8 |          20 |

## Benchmark groups

| Group               | Measures                                                        | Relevant to |
| ------------------- | --------------------------------------------------------------- | ----------- |
| `parse`             | `parse` source → AST                                            | —           |
| `analyze`           | `analyze_with` (semantic analysis)                              | —           |
| `eval`              | `eval_program_with` (incl. glob resolution)                     | —           |
| `run_pipeline`      | cold end-to-end run, every stage executes (cache cleared)       | Phase 24    |
| `run_pipeline_warm` | warm run, every stage hits the skip-check (cache populated)     | Phase 25    |

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
