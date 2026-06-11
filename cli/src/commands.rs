//! Phase 8 — CLI.
//!
//! Wires the CLI subcommands to the `mainstage_core` runtime and renders structured
//! terminal output. Every command returns a process exit code (0 = success).

use chrono::{DateTime, Local};
use std::path::Path;

use clap::{Arg, ArgAction, Command};
use console::style;
use mainstage_core::ast::Program;
use mainstage_core::{
    AnalysisResult, EvalContext, ModuleRegistry, Permissions, Reporter, Source, analyze_with, ast,
    cache, eval_program_with, parse, run_pipeline_reported,
};

/// Default script file used by `run` / `list` / `clean` and the no-subcommand run.
const DEFAULT_SCRIPT: &str = "main.ms";

/// Build the `--file` option shared by the script-oriented subcommands.
fn file_arg() -> Arg {
    Arg::new("file")
        .short('f')
        .long("file")
        .value_name("FILE")
        .default_value(DEFAULT_SCRIPT)
        .help("Path to the .ms script")
}

/// Build a global capability-granting flag. Marked `global` so it is accepted both
/// before and after the subcommand (e.g. `mainstage --allow-run run release`).
fn capability_flag(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name).long(name).action(ArgAction::SetTrue).global(true).help(help)
}

/// Register all CLI subcommands, the top-level `--file` option, and the capability
/// flags that grant side-effecting modules permission to run.
pub fn setup(cli: Command) -> Command {
    cli.arg(file_arg())
        .arg(capability_flag("allow-run", "Allow the shell module to run external commands"))
        .arg(capability_flag("allow-net", "Allow the http module to make network requests"))
        .arg(capability_flag("allow-all", "Grant every capability (--allow-run and --allow-net)"))
        .subcommand(
            Command::new("run")
                .about("Run a named pipeline")
                .arg(Arg::new("name").required(true).help("Pipeline name to run"))
                .arg(file_arg()),
        )
        .subcommand(
            Command::new("list")
                .about("List all declared pipelines and their stages")
                .arg(file_arg()),
        )
        .subcommand(Command::new("clean").about("Clear the change-detection cache").arg(file_arg()))
        .subcommand(
            Command::new("parse")
                .about("Parse a .ms file and print its AST (debug tool)")
                .arg(Arg::new("file").required(true).help("Path to the .ms script")),
        )
        .subcommand(
            Command::new("eval")
                .about("Parse, analyze, and evaluate a .ms file; print the context (debug tool)")
                .arg(Arg::new("file").required(true).help("Path to the .ms script")),
        )
        .subcommand(
            Command::new("modules")
                .about("List available modules and their method signatures (built-in and plugin)")
                .arg(file_arg()),
        )
        .subcommand(
            Command::new("lsp")
                .about("Run the language server over stdio (for editor integration)"),
        )
}

/// Dispatch the matched command and return the process exit code.
pub fn dispatch(matches: &clap::ArgMatches) -> i32 {
    // Capability flags are global, so reading them from the top-level matches captures
    // them wherever they appear on the command line.
    let flags = flag_permissions(matches);
    match matches.subcommand() {
        Some(("run", sub)) => {
            cmd_run(file_of(sub), Some(sub.get_one::<String>("name").unwrap()), flags)
        }
        Some(("list", sub)) => cmd_list(file_of(sub), flags),
        Some(("clean", sub)) => cmd_clean(file_of(sub)),
        Some(("parse", sub)) => cmd_parse(file_of(sub)),
        Some(("eval", sub)) => cmd_eval(file_of(sub), flags),
        Some(("modules", sub)) => cmd_modules(file_of(sub), flags),
        Some(("lsp", _)) => cmd_lsp(),
        // No subcommand: run the default pipeline.
        None => cmd_run(file_of(matches), None, flags),
        Some((other, _)) => {
            eprintln!("{} unknown command '{}'", style("error:").red().bold(), other);
            2
        }
    }
}

fn file_of(matches: &clap::ArgMatches) -> &str {
    matches.get_one::<String>("file").map(String::as_str).unwrap_or(DEFAULT_SCRIPT)
}

/// Derive the capabilities granted on the command line. `--allow-all` implies both.
fn flag_permissions(matches: &clap::ArgMatches) -> Permissions {
    let all = matches.get_flag("allow-all");
    Permissions {
        run: all || matches.get_flag("allow-run"),
        net: all || matches.get_flag("allow-net"),
    }
}

// ── run ─────────────────────────────────────────────────────────────────────────

fn cmd_run(file: &str, pipeline: Option<&str>, perms: Permissions) -> i32 {
    let Some((program, analysis, ctx)) = prepare(file, perms) else {
        return 1;
    };

    match pipeline {
        Some(name) => println!("{} pipeline {}", style("running").bold(), style(name).cyan()),
        None => println!("{} {}", style("running").bold(), style("default pipeline").cyan()),
    }

    let reporter = TermReporter::new();
    match run_pipeline_reported(&program, pipeline, &ctx, &analysis, &reporter) {
        Ok(()) => 0,
        // Print the conclusion for every failure mode — including ones that occur
        // before any stage runs (unknown pipeline name, dependency cycle, …), which
        // the per-stage reporter never sees.
        Err(e) => fail(e),
    }
}

// ── list ────────────────────────────────────────────────────────────────────────

fn cmd_list(file: &str, perms: Permissions) -> i32 {
    let Some((program, _, _)) = prepare(file, perms) else {
        return 1;
    };

    let pipelines: Vec<&ast::PipelineBlock> = program
        .items
        .iter()
        .filter_map(|item| match item {
            ast::Item::Pipeline(p) => Some(p),
            _ => None,
        })
        .collect();

    if pipelines.is_empty() {
        println!("no pipelines declared in {file}");
        return 0;
    }

    for p in pipelines {
        let marker =
            if p.is_default { format!(" {}", style("(default)").dim()) } else { String::new() };
        println!("{}{}", style(&p.name).cyan().bold(), marker);

        let stages = p.stages.as_ref().map(stage_names).unwrap_or_default();
        if stages.is_empty() {
            println!("  {}", style("(no stages)").dim());
        } else {
            for s in stages {
                println!("  - {s}");
            }
        }
    }
    0
}

/// Extract the bare stage-name identifiers from a pipeline `stages:` expression.
/// Non-identifier expressions (computed lists) are reported as `<dynamic>`.
fn stage_names(expr: &ast::Expr) -> Vec<String> {
    match expr {
        ast::Expr::Ident(i) => vec![i.name.clone()],
        ast::Expr::List(l) => l.items.iter().flat_map(stage_names).collect(),
        _ => vec!["<dynamic>".to_string()],
    }
}

// ── clean ───────────────────────────────────────────────────────────────────────

fn cmd_clean(file: &str) -> i32 {
    let dir = script_dir(file);
    match cache::clean(dir) {
        Ok(()) => {
            println!("{} change-detection cache", style("cleared").bold());
            0
        }
        Err(e) => fail(e),
    }
}

// ── parse / eval (debug) ─────────────────────────────────────────────────────────

fn cmd_parse(file: &str) -> i32 {
    let source = match Source::from_file(file) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    match parse(&source) {
        Ok(program) => {
            println!("{program:#?}");
            0
        }
        Err(e) => fail(e),
    }
}

fn cmd_eval(file: &str, perms: Permissions) -> i32 {
    match prepare(file, perms) {
        Some((_, _, ctx)) => {
            println!("{ctx:#?}");
            0
        }
        None => 1,
    }
}

// ── modules ──────────────────────────────────────────────────────────────────────

/// List every available module — built-in and plugins discovered under the script
/// directory — with each method rendered in call form. Granted capabilities are
/// irrelevant here: gated modules (`shell`, `http`) are always registered and listed;
/// permission is only enforced when a method is actually called.
fn cmd_modules(file: &str, _perms: Permissions) -> i32 {
    let registry = match ModuleRegistry::with_plugins(script_dir(file)) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };

    for name in registry.module_names() {
        let Some(module) = registry.get(name) else { continue };
        println!("{}", style(name).cyan().bold());
        let methods = module.methods();
        if methods.is_empty() {
            println!("  {}", style("(no methods)").dim());
        } else {
            for m in methods {
                println!("  {}", m.signature());
            }
        }
    }
    0
}

// ── lsp ──────────────────────────────────────────────────────────────────────────

/// Launch the language server over stdio. Blocks until the editor client
/// disconnects, then exits successfully. This is the editor entry point.
fn cmd_lsp() -> i32 {
    mainstage_lsp::run_stdio();
    0
}

// ── Shared pipeline preparation ──────────────────────────────────────────────────

/// Load, parse, analyze, and evaluate `file`. On any error, print it and return
/// `None`. On success, return the program, its analysis, and the eval context.
///
/// `flag_perms` are the capabilities granted on the command line; they are unioned
/// with any declared in the manifest `[permissions]` block, so a capability granted
/// by either source is in effect for the run.
fn prepare(file: &str, flag_perms: Permissions) -> Option<(Program, AnalysisResult, EvalContext)> {
    let source = match Source::from_file(file) {
        Ok(s) => s,
        Err(e) => {
            fail(e);
            return None;
        }
    };
    let program = match parse(&source) {
        Ok(p) => p,
        Err(e) => {
            fail(e);
            return None;
        }
    };
    let manifest_perms = match Permissions::from_manifest(script_dir(file)) {
        Ok(p) => p,
        Err(e) => {
            fail(e);
            return None;
        }
    };
    // Construct the registry once and share it between analysis and evaluation so
    // both agree on the set of available modules. Plugins discovered under the
    // script directory are spawned here and live for the rest of the run. The granted
    // capabilities are the union of the manifest's and the command line's.
    let registry = match ModuleRegistry::with_plugins(script_dir(file)) {
        Ok(r) => r.with_permissions(manifest_perms.union(flag_perms)),
        Err(e) => {
            fail(e);
            return None;
        }
    };
    let analysis = match analyze_with(&program, &registry) {
        Ok(a) => a,
        Err(e) => {
            fail(e);
            return None;
        }
    };
    let ctx = match eval_program_with(&program, script_dir(file), registry) {
        Ok(c) => c,
        Err(e) => {
            fail(e);
            return None;
        }
    };
    Some((program, analysis, ctx))
}

/// Directory containing the script — the root for globs and the cache.
fn script_dir(file: &str) -> &Path {
    Path::new(file).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// Print a core error (its `Display` is already user-facing and prefixed) and
/// return the failure exit code.
fn fail(e: mainstage_core::Error) -> i32 {
    eprintln!("{e}");
    1
}

// ── Terminal reporter ────────────────────────────────────────────────────────────

fn format_duration(d: chrono::TimeDelta) -> String {
    let ms = d.num_milliseconds();
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// Renders pipeline progress to the terminal with status glyphs.
struct TermReporter {
    start_time: DateTime<Local>,
}

impl TermReporter {
    fn new() -> Self {
        Self { start_time: Local::now() }
    }
}

impl Reporter for TermReporter {
    fn stage_start(&self, stage: &str) {
        println!("{} {}", style("▶").cyan(), style(stage).bold());
    }

    fn stage_skipped(&self, stage: &str) {
        println!("{} {} {}", style("•").dim(), stage, style("(up to date)").dim());
    }

    fn stage_passed(&self, stage: &str) {
        println!("{} {}", style("✓").green(), stage);
    }

    fn stage_failed(&self, stage: &str, error: &mainstage_core::Error, allow_failure: bool) {
        if allow_failure {
            println!("{} {} {}", style("!").yellow(), stage, style("(failure allowed)").yellow());
        } else {
            println!("{} {}", style("✗").red(), style(stage).red());
            eprintln!("  {error}");
        }
    }

    fn stage_cancelled(&self, stage: &str) {
        println!("{} {} {}", style("⊘").yellow(), stage, style("(cancelled)").yellow());
    }

    fn pipeline_finished(&self, pipeline: &str, failed_stage: Option<&str>) {
        let elapsed = Local::now().signed_duration_since(self.start_time);
        let elapsed_str = format_duration(elapsed);
        // Only the success banner is rendered here; failures (including those with no
        // failing stage) are reported by `cmd_run` from the returned error, avoiding a
        // redundant summary line.
        if failed_stage.is_none() {
            println!(
                "\n{} {}",
                style("✓").green().bold(),
                style(format!("pipeline '{pipeline}' succeeded in {elapsed_str}")).green()
            );
        }
    }
}
