//! Phase 23 — Benchmarking & Profiling Harness.
//!
//! Establishes baseline timings for the language pipeline so later phases can
//! prove measurable gains:
//!
//! - **Phase 24 (parallel stage execution)** is measured by `run_pipeline`, which
//!   runs every stage of a synthetic DAG cold (cache cleared each iteration).
//! - **Phase 25 (faster change detection)** is measured by `run_pipeline_warm`,
//!   which exercises the skip-check + input hashing path with a populated cache.
//!
//! Fixtures are produced by a generator parameterized by stage count, DAG depth,
//! and files-per-stage (see [`ProjectSpec`]), so the same shapes can be reused as
//! the runtime evolves. Run with `cargo bench -p mainstage_core`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use mainstage_core::ast::Program;
use mainstage_core::{
    AnalysisResult, EvalContext, ModuleRegistry, Source, analyze_with, eval_program_with, parse,
    run_pipeline,
};

// ── Fixture generator ───────────────────────────────────────────────────────────

/// Shape of a synthetic Mainstage project.
///
/// Stages are laid out in `depth` layers of roughly equal width; every stage in a
/// layer depends on the outputs of the stage directly above it in the previous
/// layer, producing `width = ceil(stages / depth)` parallel dependency chains each
/// `depth` stages long. Each stage also globs its own `files_per_stage` input files,
/// so change-detection hashing has real work to do.
#[derive(Clone, Copy)]
struct ProjectSpec {
    stages: usize,
    depth: usize,
    files_per_stage: usize,
}

impl ProjectSpec {
    const fn new(stages: usize, depth: usize, files_per_stage: usize) -> Self {
        Self { stages, depth, files_per_stage }
    }

    /// Number of parallel dependency chains (the DAG's width).
    fn width(&self) -> usize {
        self.stages.div_ceil(self.depth.max(1))
    }

    /// A short label for benchmark ids, e.g. `s50_d5_f10`.
    fn label(&self) -> String {
        format!("s{}_d{}_f{}", self.stages, self.depth, self.files_per_stage)
    }

    /// Render the `.ms` source for this spec, rooting all paths under `base` so the
    /// generated globs and outputs resolve regardless of the process working
    /// directory. Paths use forward slashes (accepted on every platform).
    fn source(&self, base: &Path) -> String {
        let base = base.display().to_string().replace('\\', "/");
        let width = self.width();
        let mut s = String::new();

        s.push_str("project {\n    name: \"bench\"\n}\n\n");

        for i in 0..self.stages {
            s.push_str(&format!("let s{i}_in = glob(\"{base}/in/s{i}/**/*.txt\");\n"));
        }
        s.push('\n');

        s.push_str("default pipeline build {\n    stages: [");
        for i in 0..self.stages {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("s{i}"));
        }
        s.push_str("]\n}\n\n");

        for i in 0..self.stages {
            let inputs = if i < width {
                // Layer 0: a source stage with no upstream dependency.
                format!("s{i}_in")
            } else {
                // Deeper layer: depend on the stage directly above plus own files.
                format!("[s{}.outputs, s{i}_in]", i - width)
            };
            s.push_str(&format!(
                "stage s{i} {{\n    inputs: {inputs}\n    outputs: [\"{base}/out/s{i}.bin\"]\n    \
                 steps {{\n        write \"{base}/out/s{i}.bin\" content: \"s{i}\"\n    }}\n}}\n\n"
            ));
        }

        s
    }

    /// Write the `files_per_stage` input files for every stage under `base/in`.
    /// Idempotent — safe to call once per fixture directory.
    fn materialize(&self, base: &Path) {
        for i in 0..self.stages {
            let dir = base.join("in").join(format!("s{i}"));
            std::fs::create_dir_all(&dir).expect("create input dir");
            for j in 0..self.files_per_stage {
                let path = dir.join(format!("f{j}.txt"));
                std::fs::write(&path, format!("stage {i} file {j}\n")).expect("write input file");
            }
        }
    }
}

/// Spec sizes covered by the benchmarks: a small interactive script, a mid-size
/// project, and a large one with deep chains and wide filesets.
const SMALL: ProjectSpec = ProjectSpec::new(10, 3, 5);
const MEDIUM: ProjectSpec = ProjectSpec::new(50, 5, 10);
const LARGE: ProjectSpec = ProjectSpec::new(100, 8, 20);

// ── Temp-directory helpers ──────────────────────────────────────────────────────

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique, freshly-created temp directory tagged for this run.
fn fresh_dir(tag: &str) -> PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ms_bench_{tag}_{nanos}_{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A materialized fixture: parsed program, analysis, registry, and its on-disk root.
struct Fixture {
    program: Program,
    analysis: AnalysisResult,
    registry: ModuleRegistry,
    dir: PathBuf,
}

impl Fixture {
    /// Build a fixture for `spec`: write its input files and parse + analyze its
    /// source once. The returned `dir` owns the on-disk files for the bench's life.
    fn build(spec: ProjectSpec, tag: &str) -> Self {
        let dir = fresh_dir(tag);
        spec.materialize(&dir);
        let src = spec.source(&dir);
        let program = parse(&Source::from_str(dir.join("main.ms"), src)).expect("parse fixture");
        let registry = ModuleRegistry::standard();
        let analysis = analyze_with(&program, &registry).expect("analyze fixture");
        Self { program, analysis, registry, dir }
    }

    fn eval(&self) -> EvalContext {
        eval_program_with(&self.program, &self.dir, self.registry.clone()).expect("eval fixture")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ── Front-end benchmarks: parse, analyze, eval ──────────────────────────────────

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for spec in [SMALL, MEDIUM, LARGE] {
        // Source rendering is independent of the on-disk files for parsing.
        let src = spec.source(Path::new("/bench"));
        let source = Source::from_str("main.ms", src);
        group.bench_with_input(BenchmarkId::from_parameter(spec.label()), &source, |b, source| {
            b.iter(|| parse(std::hint::black_box(source)).expect("parse"));
        });
    }
    group.finish();
}

fn bench_analyze(c: &mut Criterion) {
    let mut group = c.benchmark_group("analyze");
    let registry = ModuleRegistry::standard();
    for spec in [SMALL, MEDIUM, LARGE] {
        let program =
            parse(&Source::from_str("main.ms", spec.source(Path::new("/bench")))).expect("parse");
        group.bench_with_input(
            BenchmarkId::from_parameter(spec.label()),
            &program,
            |b, program| {
                b.iter(|| analyze_with(std::hint::black_box(program), &registry).expect("analyze"));
            },
        );
    }
    group.finish();
}

fn bench_eval(c: &mut Criterion) {
    let mut group = c.benchmark_group("eval");
    for spec in [SMALL, MEDIUM, LARGE] {
        // Eval resolves globs against the fixture's real input files.
        let fixture = Fixture::build(spec, "eval");
        group.bench_with_input(
            BenchmarkId::from_parameter(spec.label()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    eval_program_with(
                        std::hint::black_box(&fixture.program),
                        &fixture.dir,
                        fixture.registry.clone(),
                    )
                    .expect("eval")
                });
            },
        );
    }
    group.finish();
}

// ── End-to-end runner benchmarks ────────────────────────────────────────────────

/// Remove a run's outputs and change-detection cache so the next run is cold.
fn reset_run(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.join(".mainstage"));
    let _ = std::fs::remove_dir_all(dir.join("out"));
}

/// Cold full-pipeline execution — every stage runs each iteration (cache cleared in
/// setup). This is the headline number Phase 24 (parallel execution) improves.
fn bench_run_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("run_pipeline");
    group.sample_size(10);
    for spec in [SMALL, MEDIUM] {
        let fixture = Fixture::build(spec, "run");
        group.bench_with_input(
            BenchmarkId::from_parameter(spec.label()),
            &fixture,
            |b, fixture| {
                b.iter_batched(
                    || {
                        reset_run(&fixture.dir);
                        fixture.eval()
                    },
                    |ctx| run_pipeline(&fixture.program, None, &ctx, &fixture.analysis),
                    BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

/// Warm execution — the cache is populated once, so every stage hits the skip-check
/// (input hashing + output-existence test) and is skipped. This is the path Phase 25
/// (faster change detection) improves.
fn bench_run_pipeline_warm(c: &mut Criterion) {
    let mut group = c.benchmark_group("run_pipeline_warm");
    group.sample_size(10);
    for spec in [MEDIUM, LARGE] {
        let fixture = Fixture::build(spec, "warm");
        // Prime the cache and outputs with one full run so subsequent runs skip.
        reset_run(&fixture.dir);
        run_pipeline(&fixture.program, None, &fixture.eval(), &fixture.analysis)
            .expect("warm-up run");
        group.bench_with_input(
            BenchmarkId::from_parameter(spec.label()),
            &fixture,
            |b, fixture| {
                b.iter_batched(
                    || fixture.eval(),
                    |ctx| run_pipeline(&fixture.program, None, &ctx, &fixture.analysis),
                    BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_parse,
    bench_analyze,
    bench_eval,
    bench_run_pipeline,
    bench_run_pipeline_warm
);
criterion_main!(benches);
