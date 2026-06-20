//! Phase 8 — CLI.
//!
//! Wires the CLI subcommands to the `mainstage_core` runtime and renders structured
//! terminal output. Every command returns a process exit code (0 = success).

use chrono::{DateTime, Local};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Arg, ArgAction, Command};
use console::style;
use mainstage_core::ast::Program;
use mainstage_core::{
    AnalysisResult, AssertionResult, CancelToken, Diagnostic, Error, EvalContext, LintLevel,
    ModuleRegistry, Permissions, Plan, PlanStatus, Reporter, ReporterHandle, Source, Span,
    StageOutcome, analyze_with, ast, cache, eval_program_with, lint_plugin, parse,
    pipeline_input_paths, plan_pipeline, run_pipeline_cancellable,
};

use crate::scaffold::{self, Lang};

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
        .arg(
            Arg::new("jobs")
                .short('j')
                .long("jobs")
                .value_name("N")
                .global(true)
                .value_parser(clap::value_parser!(usize))
                .help("Max stages to run concurrently (default: host core count; 1 = sequential)"),
        )
        // Global output-control flags, accepted before or after the subcommand.
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::SetTrue)
                .global(true)
                .conflicts_with("quiet")
                .help("Print extra detail, including per-stage timings inline"),
        )
        .arg(
            Arg::new("quiet")
                .short('q')
                .long("quiet")
                .action(ArgAction::SetTrue)
                .global(true)
                .help("Suppress progress output; print only errors"),
        )
        .arg(
            Arg::new("no-color")
                .long("no-color")
                .action(ArgAction::SetTrue)
                .global(true)
                .help("Disable colored output (also honored via the NO_COLOR env var)"),
        )
        .arg(
            Arg::new("dry-run").long("dry-run").action(ArgAction::SetTrue).global(true).help(
                "Show the planned execution order and which stages would run, without executing",
            ),
        )
        .subcommand(
            Command::new("run")
                .about("Run a named pipeline")
                .arg(Arg::new("name").required(true).help("Pipeline name to run"))
                .arg(file_arg()),
        )
        .subcommand(
            Command::new("watch")
                .about("Run the pipeline, then re-run it whenever its inputs change")
                .arg(
                    Arg::new("name")
                        .help("Pipeline name to run (defaults to the default pipeline)"),
                )
                .arg(file_arg()),
        )
        .subcommand(
            Command::new("list")
                .about("List all declared pipelines and their stages")
                .arg(file_arg())
                .arg(
                    Arg::new("describe")
                        .long("describe")
                        .action(ArgAction::SetTrue)
                        .help("Show each stage's description: field, when present"),
                ),
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
        .subcommand(
            Command::new("plugin")
                .about("Author and validate external plugins")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("new")
                        .about("Scaffold a working stdio plugin skeleton")
                        .arg(
                            Arg::new("name")
                                .required(true)
                                .help("Plugin module name (may be namespaced, e.g. acme/lint)"),
                        )
                        .arg(
                            Arg::new("lang")
                                .long("lang")
                                .value_name("LANG")
                                .default_value("python")
                                .help("Plugin language: python (py) or shell (sh)"),
                        )
                        .arg(
                            Arg::new("dir")
                                .long("dir")
                                .value_name("DIR")
                                .help("Output directory (defaults to the plugin's name)"),
                        )
                        .arg(
                            Arg::new("force")
                                .long("force")
                                .action(ArgAction::SetTrue)
                                .help("Overwrite an existing output directory"),
                        ),
                )
                .subcommand(
                    Command::new("check")
                        .about("Lint a plugin against the protocol before publishing")
                        .arg(Arg::new("path").required(true).help("Path to the plugin executable")),
                ),
        )
        .subcommand(
            Command::new("format")
                .about("Format .ms scripts to canonical style")
                .arg(
                    Arg::new("files")
                        .num_args(0..)
                        .value_name("FILES")
                        .help("Scripts to format (defaults to main.ms)"),
                )
                .arg(
                    Arg::new("check")
                        .long("check")
                        .action(ArgAction::SetTrue)
                        .help("Exit non-zero if any file is not already formatted; write nothing"),
                )
                .arg(
                    Arg::new("stdout")
                        .long("stdout")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("check")
                        .help("Print formatted output to stdout instead of writing files"),
                ),
        )
}

/// Dispatch the matched command and return the process exit code.
pub fn dispatch(matches: &clap::ArgMatches) -> i32 {
    // Resolve color handling first, so every line printed below respects it. `--no-color`
    // (or the conventional NO_COLOR env var) forces plain output; otherwise `console`
    // auto-detects whether stdout is a terminal.
    if matches.get_flag("no-color") || std::env::var_os("NO_COLOR").is_some() {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }
    let verbosity = Verbosity::from_matches(matches);

    // Capability flags are global, so reading them from the top-level matches captures
    // them wherever they appear on the command line.
    let flags = flag_permissions(matches);
    // `--jobs` is global, so it is read from the top-level matches wherever it appears.
    let jobs = matches.get_one::<usize>("jobs").copied();
    let dry_run = matches.get_flag("dry-run");
    match matches.subcommand() {
        Some(("run", sub)) => {
            let name = sub.get_one::<String>("name").map(String::as_str);
            if dry_run {
                cmd_dry_run(file_of(sub), name, flags)
            } else {
                cmd_run(file_of(sub), name, flags, jobs, verbosity)
            }
        }
        Some(("watch", sub)) => {
            let name = sub.get_one::<String>("name").map(String::as_str);
            cmd_watch(file_of(sub), name, flags, jobs, verbosity)
        }
        Some(("list", sub)) => cmd_list(file_of(sub), flags, sub.get_flag("describe")),
        Some(("clean", sub)) => cmd_clean(file_of(sub)),
        Some(("parse", sub)) => cmd_parse(file_of(sub)),
        Some(("eval", sub)) => cmd_eval(file_of(sub), flags),
        Some(("modules", sub)) => cmd_modules(file_of(sub), flags),
        Some(("lsp", _)) => cmd_lsp(),
        Some(("plugin", sub)) => match sub.subcommand() {
            Some(("new", args)) => cmd_plugin_new(args),
            Some(("check", args)) => cmd_plugin_check(args),
            _ => {
                eprintln!(
                    "{} expected `plugin new` or `plugin check`",
                    style("error:").red().bold()
                );
                2
            }
        },
        Some(("format", sub)) => {
            let files: Vec<String> = sub
                .get_many::<String>("files")
                .map(|vals| vals.cloned().collect())
                .unwrap_or_default();
            cmd_format(&files, sub.get_flag("check"), sub.get_flag("stdout"))
        }
        // No subcommand: plan or run the default pipeline.
        None if dry_run => cmd_dry_run(file_of(matches), None, flags),
        None => cmd_run(file_of(matches), None, flags, jobs, verbosity),
        Some((other, _)) => {
            eprintln!("{} unknown command '{}'", style("error:").red().bold(), other);
            2
        }
    }
}

/// How much progress output to print. Controlled by the global `--verbose` / `--quiet`
/// flags; `Normal` is the default.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verbosity {
    /// Only errors.
    Quiet,
    /// Progress markers plus an end-of-run timing summary.
    Normal,
    /// Everything in `Normal`, plus inline per-stage timings and extra detail.
    Verbose,
}

impl Verbosity {
    fn from_matches(matches: &clap::ArgMatches) -> Self {
        if matches.get_flag("quiet") {
            Verbosity::Quiet
        } else if matches.get_flag("verbose") {
            Verbosity::Verbose
        } else {
            Verbosity::Normal
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

fn cmd_run(
    file: &str,
    pipeline: Option<&str>,
    perms: Permissions,
    jobs: Option<usize>,
    verbosity: Verbosity,
) -> i32 {
    let Some((program, analysis, ctx)) = prepare(file, perms) else {
        return 1;
    };

    // Install a Ctrl-C / SIGTERM handler that requests cooperative cancellation. The
    // runner then stops launching stages, lets in-flight ones finish, and saves a
    // consistent cache before returning. A best-effort install — if a handler is already
    // registered, the run simply proceeds without interactive cancellation.
    let cancel = CancelToken::new();
    {
        let cancel = cancel.clone();
        let _ = ctrlc::set_handler(move || cancel.cancel());
    }

    run_prepared(&program, pipeline, &ctx, &analysis, jobs, verbosity, &cancel)
}

/// Run an already-prepared pipeline against `cancel`, rendering progress at `verbosity`.
/// Shared by `cmd_run` and `cmd_watch`.
fn run_prepared(
    program: &Program,
    pipeline: Option<&str>,
    ctx: &EvalContext,
    analysis: &AnalysisResult,
    jobs: Option<usize>,
    verbosity: Verbosity,
    cancel: &CancelToken,
) -> i32 {
    if verbosity != Verbosity::Quiet {
        match pipeline {
            Some(name) => println!("{} pipeline {}", style("running").bold(), style(name).cyan()),
            None => println!("{} {}", style("running").bold(), style("default pipeline").cyan()),
        }
    }

    // Default to the host core count; `--jobs 1` forces sequential execution.
    let jobs =
        jobs.unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));

    // Share one reporter between the runner's lifecycle events and the `log` step: install
    // a handle to it on the context so `log` routes through `Reporter::step_log` (honoring
    // `--quiet` and the per-stage buffered output) just like the runner's own markers.
    let reporter: Arc<dyn Reporter> = Arc::new(TermReporter::new(verbosity));
    let run_ctx = ctx.with_reporter(ReporterHandle(reporter.clone()));
    match run_pipeline_cancellable(program, pipeline, &run_ctx, analysis, &*reporter, jobs, cancel)
    {
        Ok(()) => 0,
        // Print the conclusion for every failure mode — including ones that occur
        // before any stage runs (unknown pipeline name, dependency cycle, …), which
        // the per-stage reporter never sees.
        Err(e) => fail(e),
    }
}

// ── dry-run ──────────────────────────────────────────────────────────────────────

/// Show the plan for a pipeline — the dependency waves and, per stage, whether it would
/// run or be skipped — without executing any steps. The cache is read but never written.
fn cmd_dry_run(file: &str, pipeline: Option<&str>, perms: Permissions) -> i32 {
    let Some((program, analysis, ctx)) = prepare(file, perms) else {
        return 1;
    };

    let plan = match plan_pipeline(&program, pipeline, &ctx, &analysis) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    print_plan(&plan);
    0
}

/// Render a [`Plan`] as numbered waves of stages, each tagged `run` or `skip`. Stages in
/// the same wave have no ordering dependency and would execute concurrently.
fn print_plan(plan: &Plan) {
    println!("{} pipeline {}", style("dry run:").bold(), style(&plan.pipeline).cyan());

    if plan.waves.is_empty() {
        println!("  {}", style("(no stages)").dim());
        return;
    }

    let (mut runs, mut skips) = (0usize, 0usize);
    for (i, wave) in plan.waves.iter().enumerate() {
        // A wave header is only useful when there is concurrency to convey; with a single
        // wave or single-stage waves the numbering still clarifies execution order.
        println!("{}", style(format!("  wave {}", i + 1)).dim());
        for stage in wave {
            match stage.status {
                PlanStatus::Run => {
                    runs += 1;
                    println!(
                        "    {} {} {}",
                        style("▶").cyan(),
                        style(&stage.name).bold(),
                        style("(run)").cyan()
                    );
                }
                PlanStatus::Skip => {
                    skips += 1;
                    println!(
                        "    {} {} {}",
                        style("•").dim(),
                        stage.name,
                        style("(skip, up to date)").dim()
                    );
                }
            }
        }
    }
    println!("\n{} {} to run, {} to skip", style("plan:").bold(), runs, skips);
}

// ── watch ────────────────────────────────────────────────────────────────────────

/// How often `watch` polls its tracked files for changes.
const WATCH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

/// Run the pipeline, then re-run it whenever any of its resolved input files — or the
/// script itself — changes on disk. Polls file modification times (and the modification
/// times of their parent directories, so newly added or removed files are noticed too)
/// rather than using OS file-watch APIs, keeping the implementation dependency-free and
/// portable. Runs until interrupted with Ctrl-C.
fn cmd_watch(
    file: &str,
    pipeline: Option<&str>,
    perms: Permissions,
    jobs: Option<usize>,
    verbosity: Verbosity,
) -> i32 {
    // A single cancellation token, shared with one Ctrl-C handler installed for the whole
    // watch session: the first Ctrl-C cancels an in-flight run; once idle, it ends watch.
    let cancel = CancelToken::new();
    {
        let cancel = cancel.clone();
        let _ = ctrlc::set_handler(move || cancel.cancel());
    }

    loop {
        // Re-evaluate the program each iteration so changes to globs, lets, or the
        // pipeline itself take effect. A preparation error is reported but does not end
        // watch — fix the script and it re-runs on the next change.
        if cancel.is_cancelled() {
            break;
        }
        match prepare(file, perms) {
            Some((program, analysis, ctx)) => {
                let _ = run_prepared(&program, pipeline, &ctx, &analysis, jobs, verbosity, &cancel);
                // A Ctrl-C during the run cancels it and ends the watch session.
                if cancel.is_cancelled() {
                    break;
                }

                // Determine the files to watch: every stage's resolved inputs, plus the
                // script file. Failure to compute the plan (e.g. an unresolved reference)
                // falls back to watching just the script so edits still trigger a re-run.
                let mut watched =
                    pipeline_input_paths(&program, pipeline, &ctx, &analysis).unwrap_or_default();
                watched.push(std::path::PathBuf::from(file));

                if verbosity != Verbosity::Quiet {
                    println!(
                        "\n{} {} file(s); press Ctrl-C to stop",
                        style("watching").bold().blue(),
                        watched.len()
                    );
                }

                if !wait_for_change(&watched, &cancel) {
                    break; // Ctrl-C while idle: stop watching.
                }
                if verbosity != Verbosity::Quiet {
                    println!("{}", style("change detected — re-running").blue());
                }
            }
            None => {
                // The script failed to load/parse. Watch just the script and retry when
                // it changes, so the user can fix the error in place.
                let watched = vec![std::path::PathBuf::from(file)];
                if verbosity != Verbosity::Quiet {
                    println!(
                        "\n{} {}; press Ctrl-C to stop",
                        style("watching").bold().blue(),
                        file
                    );
                }
                if !wait_for_change(&watched, &cancel) {
                    break;
                }
            }
        }
    }

    if verbosity != Verbosity::Quiet {
        println!("{}", style("watch stopped").dim());
    }
    0
}

/// A snapshot of the size and modification time of each tracked path (and missing paths,
/// recorded as `None`) used to detect changes between polls.
fn watch_snapshot(paths: &[std::path::PathBuf]) -> Vec<Option<(u64, std::time::SystemTime)>> {
    paths
        .iter()
        .flat_map(|p| {
            // Track the path itself and its parent directory; a directory's mtime changes
            // when entries are added or removed, catching files that appear or vanish.
            let parent = p.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf());
            std::iter::once(p.clone()).chain(parent)
        })
        .map(|p| std::fs::metadata(&p).ok().and_then(|m| Some((m.len(), m.modified().ok()?))))
        .collect()
}

/// Block until any tracked path changes (returning `true`) or cancellation is requested
/// (returning `false`). Polls at [`WATCH_POLL_INTERVAL`].
fn wait_for_change(paths: &[std::path::PathBuf], cancel: &CancelToken) -> bool {
    let baseline = watch_snapshot(paths);
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        std::thread::sleep(WATCH_POLL_INTERVAL);
        if cancel.is_cancelled() {
            return false;
        }
        if watch_snapshot(paths) != baseline {
            return true;
        }
    }
}

// ── list ────────────────────────────────────────────────────────────────────────

fn cmd_list(file: &str, perms: Permissions, describe: bool) -> i32 {
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

    // Map each stage name to its explicit `depends_on` edges so the listing can show the
    // ordering that the `stages:` membership list alone does not convey.
    let depends_on: std::collections::HashMap<&str, Vec<&str>> = program
        .items
        .iter()
        .filter_map(|item| match item {
            ast::Item::Stage(s) => {
                Some((s.name.as_str(), s.depends_on.iter().map(|d| d.name.as_str()).collect()))
            }
            _ => None,
        })
        .collect();

    // Stage descriptions, surfaced under `--describe` so a multi-stage build is navigable.
    let descriptions: std::collections::HashMap<&str, &str> = program
        .items
        .iter()
        .filter_map(|item| match item {
            ast::Item::Stage(s) => s.description.as_deref().map(|d| (s.name.as_str(), d)),
            _ => None,
        })
        .collect();

    for p in pipelines {
        let marker =
            if p.is_default { format!(" {}", style("(default)").dim()) } else { String::new() };
        println!("{}{}", style(&p.name).cyan().bold(), marker);

        let stages = p.stages.as_ref().map(stage_names).unwrap_or_default();
        if stages.is_empty() {
            println!("  {}", style("(no stages)").dim());
        } else {
            for s in stages {
                match depends_on.get(s.as_str()) {
                    Some(deps) if !deps.is_empty() => {
                        let after = style(format!("(after {})", deps.join(", "))).dim();
                        println!("  - {s} {after}");
                    }
                    _ => println!("  - {s}"),
                }
                if describe && let Some(desc) = descriptions.get(s.as_str()) {
                    println!("      {}", style(desc).dim());
                }
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

// ── plugin ─────────────────────────────────────────────────────────────────────

/// Scaffold a working plugin skeleton. The generated plugin already answers
/// `describe` and a sample `call`, so it passes `plugin check` immediately.
fn cmd_plugin_new(args: &clap::ArgMatches) -> i32 {
    let name = args.get_one::<String>("name").map(String::as_str).unwrap_or_default();
    let dir = args.get_one::<String>("dir").map(String::as_str);
    let force = args.get_flag("force");

    let lang_str = args.get_one::<String>("lang").map(String::as_str).unwrap_or("python");
    let Some(lang) = Lang::parse(lang_str) else {
        eprintln!(
            "{} unknown --lang '{lang_str}' (expected 'python' or 'shell')",
            style("error:").red().bold()
        );
        return 2;
    };

    match scaffold::new_plugin(name, dir, lang, force) {
        Ok(script) => {
            scaffold::print_next_steps(name, &script);
            0
        }
        Err(e) => {
            eprintln!("{} {e}", style("error:").red().bold());
            1
        }
    }
}

/// Lint a plugin against the wire protocol. Spawns it, sends `describe`, and reports
/// errors (the plugin is broken) and warnings (it works but breaks a convention).
/// Exits non-zero when any error is found, so it doubles as a CI/pre-publish gate.
fn cmd_plugin_check(args: &clap::ArgMatches) -> i32 {
    let path = args.get_one::<String>("path").map(String::as_str).unwrap_or_default();
    let exe = Path::new(path);
    // Run the plugin from its own directory, matching how discovery spawns it.
    let script_dir = exe.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));

    let report = lint_plugin(exe, script_dir);

    println!("{} {}", style("checking").bold(), path);
    if let Some(name) = &report.module_name {
        println!("  module {} · {} method(s)", style(name).cyan(), report.method_count);
    }

    for finding in &report.findings {
        match finding.level {
            LintLevel::Error => {
                println!("  {} {}", style("error").red().bold(), finding.message)
            }
            LintLevel::Warning => {
                println!("  {} {}", style("warning").yellow().bold(), finding.message)
            }
        }
    }

    if report.has_errors() {
        eprintln!("{} plugin has protocol errors", style("failed:").red().bold());
        1
    } else if report.is_clean() {
        println!("{} plugin conforms to the protocol", style("ok:").green().bold());
        0
    } else {
        // Warnings only — usable, but worth addressing before publishing.
        println!("{} plugin is usable (warnings above)", style("ok:").green().bold());
        0
    }
}

// ── format ─────────────────────────────────────────────────────────────────────

/// Format one or more scripts to canonical style.
///
/// Default: rewrite each file in place. `--check`: write nothing and exit non-zero
/// when any file is not already formatted (a CI gate). `--stdout`: print the
/// formatted output without writing. A parse error in any file fails the command.
fn cmd_format(files: &[String], check: bool, stdout: bool) -> i32 {
    let owned;
    let files: &[String] = if files.is_empty() {
        owned = vec![DEFAULT_SCRIPT.to_string()];
        &owned
    } else {
        files
    };

    let mut exit = 0;
    let mut unformatted = 0;
    for file in files {
        let source = match Source::from_file(file) {
            Ok(s) => s,
            Err(e) => {
                fail(e);
                exit = 1;
                continue;
            }
        };
        let formatted = match mainstage_core::format(&source) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{} {}", style("error in").red().bold(), style(file).bold());
                fail(e);
                exit = 1;
                continue;
            }
        };

        if stdout {
            print!("{formatted}");
        } else if check {
            if formatted != source.text {
                println!("{} {}", style("would reformat").yellow(), file);
                unformatted += 1;
            }
        } else if formatted != source.text {
            if let Err(e) = std::fs::write(file, &formatted) {
                eprintln!("{} writing '{}': {}", style("error:").red().bold(), file, e);
                exit = 1;
            } else {
                println!("{} {}", style("formatted").green(), file);
            }
        }
    }

    if check && unformatted > 0 {
        eprintln!("{} {unformatted} file(s) need formatting", style("error:").red().bold());
        return 1;
    }
    exit
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
    // Lower `matrix` stages into their concrete variants before analysis, evaluation, and
    // scheduling, so every later stage sees ordinary stages (Phase 37).
    let program = match mainstage_core::expand_matrix(&program) {
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

/// Render a core error to stderr — with a source snippet and caret underline when the
/// diagnostic carries a span — and return the failure exit code.
fn fail(e: mainstage_core::Error) -> i32 {
    match &e {
        // I/O errors have no source location to point at; print as-is.
        Error::Io { .. } => eprintln!("{e}"),
        Error::Parse(diags) | Error::Semantic(diags) | Error::Eval(diags) => {
            for (i, d) in diags.iter().enumerate() {
                if i > 0 {
                    eprintln!();
                }
                render_diagnostic(d);
            }
        }
    }
    1
}

/// Render one diagnostic: the message, a `--> file:line:col` locator, a source snippet
/// with a caret underline, and any supplementary notes.
fn render_diagnostic(d: &Diagnostic) {
    eprintln!("{} {}", style("error:").red().bold(), d.message);
    if let Some(span) = &d.span {
        eprintln!("  {} {}", style("-->").blue().bold(), span);
        render_snippet(span);
    }
    for note in &d.notes {
        eprintln!("  {} note: {note}", style("=").blue().bold());
    }
}

/// Print the source line(s) covered by `span` with a caret underline beneath the offending
/// span on the first line, rustc-style. Best-effort: if the file can't be read, nothing is
/// printed (the `-->` locator already names the position).
fn render_snippet(span: &Span) {
    let Ok(text) = std::fs::read_to_string(&span.file) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    // Spans are 1-based and inclusive of the start line; clamp to what the file holds.
    if span.line_start == 0 || span.line_start > lines.len() {
        return;
    }
    let last = span.line_end.min(lines.len());
    let gutter = last.to_string().len();
    let bar = style("|").blue().bold();

    eprintln!("  {:>gutter$} {}", "", bar, gutter = gutter);
    for n in span.line_start..=last {
        let line = lines[n - 1];
        eprintln!("  {} {} {}", style(format!("{n:>gutter$}")).blue().bold(), bar, line);
        if n == span.line_start {
            // Underline from col_start. For a single-line span, span the columns exactly;
            // for a multi-line span, underline to the end of this first line.
            let start = span.col_start.saturating_sub(1);
            let end = if span.line_start == span.line_end {
                span.col_end.max(span.col_start + 1)
            } else {
                line.chars().count() + 1
            };
            let width = end.saturating_sub(span.col_start).max(1);
            let pad = " ".repeat(start);
            let caret = "^".repeat(width);
            eprintln!(
                "  {:>gutter$} {} {}{}",
                "",
                bar,
                pad,
                style(caret).red().bold(),
                gutter = gutter
            );
        }
    }
}

// ── Terminal reporter ────────────────────────────────────────────────────────────

/// Format a millisecond duration compactly: `ms`, `s` (one decimal), or `m s`.
fn format_millis(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        let secs = ms / 1_000;
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// A single stage's recorded outcome and wall-clock duration, used to build the
/// end-of-run timing summary.
struct StageTiming {
    name: String,
    outcome: StageOutcome,
    elapsed: Duration,
}

/// Renders pipeline progress to the terminal with status glyphs, honoring the configured
/// [`Verbosity`] and accumulating per-stage timings for an end-of-run summary.
struct TermReporter {
    start_time: DateTime<Local>,
    verbosity: Verbosity,
    /// Per-stage timings, recorded as stages settle. Behind a `Mutex` so the reporter is
    /// `Sync` and shareable across the runner's worker threads.
    timings: Mutex<Vec<StageTiming>>,
}

impl TermReporter {
    fn new(verbosity: Verbosity) -> Self {
        Self { start_time: Local::now(), verbosity, timings: Mutex::new(Vec::new()) }
    }

    fn quiet(&self) -> bool {
        self.verbosity == Verbosity::Quiet
    }

    fn verbose(&self) -> bool {
        self.verbosity == Verbosity::Verbose
    }
}

impl Reporter for TermReporter {
    fn step_log(&self, out: &mut dyn Write, message: &str) {
        // A `log` step is progress output: suppressed in quiet mode, shown otherwise.
        if self.quiet() {
            return;
        }
        let _ = writeln!(out, "  {} {}", style("›").cyan(), message);
    }

    fn stage_start(&self, out: &mut dyn Write, stage: &str) {
        if self.quiet() {
            return;
        }
        let _ = writeln!(out, "{} {}", style("▶").cyan(), style(stage).bold());
    }

    fn stage_skipped(&self, out: &mut dyn Write, stage: &str) {
        if self.quiet() {
            return;
        }
        let _ = writeln!(out, "{} {} {}", style("•").dim(), stage, style("(up to date)").dim());
    }

    fn stage_passed(&self, out: &mut dyn Write, stage: &str) {
        // In verbose mode the pass marker is emitted by `stage_finished` with the elapsed
        // time appended; here we only render it for the normal (non-verbose) case.
        if self.quiet() || self.verbose() {
            return;
        }
        let _ = writeln!(out, "{} {}", style("✓").green(), stage);
    }

    fn stage_failed(
        &self,
        out: &mut dyn Write,
        stage: &str,
        error: &mainstage_core::Error,
        allow_failure: bool,
    ) {
        if allow_failure {
            // A tolerated failure is informational; suppress it in quiet mode.
            if self.quiet() {
                return;
            }
            let _ = writeln!(
                out,
                "{} {} {}",
                style("!").yellow(),
                stage,
                style("(failure allowed)").yellow()
            );
        } else {
            // A real failure is always shown, even in quiet mode. Write the error alongside
            // the marker so a stage's output stays one atomic block under concurrency.
            let _ = writeln!(out, "{} {}", style("✗").red(), style(stage).red());
            let _ = writeln!(out, "  {error}");
        }
    }

    fn stage_cancelled(&self, out: &mut dyn Write, stage: &str) {
        if self.quiet() {
            return;
        }
        let _ =
            writeln!(out, "{} {} {}", style("⊘").yellow(), stage, style("(cancelled)").yellow());
    }

    fn stage_tests(&self, out: &mut dyn Write, _stage: &str, results: &[AssertionResult]) {
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;

        // Per-assertion detail. Passing lines are progress (suppressed when quiet); failing
        // lines — with their reason — are always shown, even in quiet mode.
        for r in results {
            if r.passed {
                if !self.quiet() {
                    let _ =
                        writeln!(out, "  {} {}", style("✓").green(), style(&r.description).dim());
                }
            } else {
                let _ = writeln!(out, "  {} {}", style("✗").red(), r.description);
                if let Some(detail) = &r.detail {
                    let _ = writeln!(out, "      {}", style(detail).dim());
                }
            }
        }

        // The `--quiet`-aware summary line: always printed on failure; on success only when
        // not quiet.
        if failed > 0 {
            let _ = writeln!(
                out,
                "  {} {}",
                style("✗").red().bold(),
                style(format!("tests: {failed} failed, {passed} passed")).red()
            );
        } else if !self.quiet() {
            let _ = writeln!(
                out,
                "  {} {}",
                style("✓").green().bold(),
                style(format!("tests: {passed} passed")).green()
            );
        }
    }

    fn stage_finished(
        &self,
        out: &mut dyn Write,
        stage: &str,
        outcome: StageOutcome,
        elapsed: Duration,
    ) {
        // In verbose mode, render the pass marker here so the elapsed time can be shown
        // inline (the plain `stage_passed` marker is suppressed above).
        if self.verbose() && outcome == StageOutcome::Passed {
            let _ = writeln!(
                out,
                "{} {} {}",
                style("✓").green(),
                stage,
                style(format!("({})", format_millis(elapsed.as_millis()))).dim()
            );
        }
        self.timings.lock().unwrap().push(StageTiming {
            name: stage.to_string(),
            outcome,
            elapsed,
        });
    }

    fn pipeline_finished(&self, out: &mut dyn Write, pipeline: &str, failed_stage: Option<&str>) {
        if self.quiet() {
            return;
        }
        self.render_timing_summary(out);

        let elapsed = Local::now().signed_duration_since(self.start_time);
        let elapsed_str = format_millis(elapsed.num_milliseconds().max(0) as u128);
        // Only the success banner is rendered here; failures (including those with no
        // failing stage) are reported by `cmd_run` from the returned error, avoiding a
        // redundant summary line.
        if failed_stage.is_none() {
            let _ = writeln!(
                out,
                "{} {}",
                style("✓").green().bold(),
                style(format!("pipeline '{pipeline}' succeeded in {elapsed_str}")).green()
            );
        }
    }
}

impl TermReporter {
    /// Print a per-stage timing table beneath the run, aligned on stage name. Nothing is
    /// printed when no stage produced a timing (e.g. an empty pipeline).
    fn render_timing_summary(&self, out: &mut dyn Write) {
        let timings = self.timings.lock().unwrap();
        if timings.is_empty() {
            return;
        }
        let width = timings.iter().map(|t| t.name.len()).max().unwrap_or(0);
        let _ = writeln!(out, "\n{}", style("timing summary").bold());
        for t in timings.iter() {
            let (glyph, suffix) = match t.outcome {
                StageOutcome::Passed => (style("✓").green(), String::new()),
                StageOutcome::Failed => (style("✗").red(), String::new()),
                StageOutcome::Skipped => {
                    (style("•").dim(), format!("  {}", style("(up to date)").dim()))
                }
            };
            let _ = writeln!(
                out,
                "  {} {:<width$}  {}{}",
                glyph,
                t.name,
                format_millis(t.elapsed.as_millis()),
                suffix,
                width = width,
            );
        }
    }
}
