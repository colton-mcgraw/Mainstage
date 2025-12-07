//! file: cli/src/main.rs
//! description: command-line interface for MainStage.
//!
//! This binary provides user-facing commands to build, analyze and run
//! MainStage scripts. It wires together the `mainstage_core` APIs, performs
//! plugin discovery, and exposes subcommands for common developer workflows.
//!
use clap::{Arg, ArgMatches, Command};
use console::style;
use log::{Level, error, info};
use mainstage_core::VM;
use std::io::Write;
use std::path::PathBuf;

mod disassembler;
mod commands;

fn main() {
    // Initialize logger with a clean, human-friendly format and colored level tags.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            let lvl = match record.level() {
                Level::Error => style("error").red().bold(),
                Level::Warn => style("warn").yellow().bold(),
                Level::Info => style("info").green().bold(),
                Level::Debug => style("debug").cyan(),
                Level::Trace => style("trace").magenta(),
            };
            writeln!(buf, "{}: {}", lvl, record.args())
        })
        .init();

    let cli = Command::new("MainStage")
        .version("0.1.0")
        .author("Colton McGraw <https://github.com/ColtMcG1>")
        .about("A CLI for MainStage");

    let cli = setup_cli(cli).arg(
        Arg::new("plugin-dir")
            .help("Directory to load plugins from")
            .short('P')
            .long("plugin-dir")
            .value_parser(clap::value_parser!(String))
            .value_name("DIR")
            .global(true),
    )
    .arg(
        Arg::new("stl")
            .help("Select STL plugin(s) to load (repeatable)")
            .long("stl")
            .value_name("NAME")
            .value_parser(clap::value_parser!(String))
            .action(clap::ArgAction::Append)
            .global(true),
    )
    .arg(
        Arg::new("no-stl")
            .help("Do not auto-load the default STL plugin(s)")
            .long("no-stl")
            .action(clap::ArgAction::SetTrue)
            .global(true),
    );

    let matches = cli.get_matches();

    // VM plugin discovery (CLI may override the directory)
    // Resolve plugin-dir against the original CLI CWD, independent of later CWD changes.
    let mut vm = VM::new(vec![]);
    let orig_cli_cwd = std::env::current_dir().ok();
    let plugin_dir: Option<PathBuf> = matches
        .get_one::<String>("plugin-dir")
        .map(|s| {
            let p = PathBuf::from(s);
            if p.is_absolute() {
                p
            } else if let Some(ref cwd) = orig_cli_cwd {
                cwd.join(p)
            } else {
                p
            }
        });
    match vm.discover_plugins(plugin_dir.as_ref()) {
        Ok(n) => info!("Discovered {} plugin manifest(s)", n),
        Err(e) => error!("Plugin discovery failed: {}", e),
    }

    // Clone descriptors map for analyzer usage during CLI commands.
    let manifests_map = vm.plugin_descriptors();

    dispatch_commands(&matches, &manifests_map);
}

/// Sets up the CLI with subcommands and arguments.
/// This function configures the command-line interface using the `clap` crate.
/// It defines subcommands for analyzing scripts and generating reports.
fn setup_cli(cli: Command) -> Command {
    // Subcommands registered via command modules
    let cli = cli.subcommand(
        Command::new("build")
            .about("Build the specified script file")
            .arg(
                Arg::new("file")
                    .help("The script file to build")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("dump")
                    .help("Specify the dump stage")
                    .short('d')
                    .long("dump")
                    .value_parser(clap::value_parser!(String))
                    .value_name("STAGE"),
            )
            .arg(
                Arg::new("optimize")
                    .help("Enable IR optimization")
                    .short('O')
                    .long("optimize")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("output")
                    .help("Specify the output file")
                    .short('o')
                    .long("output")
                    .value_parser(clap::value_parser!(String))
                    .value_name("FILE"),
            ),
    )
    .subcommand(
        Command::new("run")
            .about("Run a script file")
            .arg(
                Arg::new("file")
                    .help("The script file to run")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("optimize")
                    .help("Enable IR optimization")
                    .short('O')
                    .long("optimize")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("trace")
                    .help("Enable tracing of script execution")
                    .short('t')
                    .long("trace")
                    .action(clap::ArgAction::SetTrue),
            ),
    )
    .subcommand(
        Command::new("inspect")
            .about("Disassemble a .msx file")
            .arg(
                Arg::new("file")
                    .help("The .msx file to disassemble")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("output")
                    .help("Specify the output file for disassembly")
                    .short('o')
                    .long("output")
                    .value_parser(clap::value_parser!(String))
                    .value_name("FILE"),
            ),
    )
    .subcommand(
        Command::new("verify-manifest")
            .about("Verify plugin manifests against runtime-registered functions")
            .arg(
                Arg::new("module")
                    .help("Optional plugin module name to verify; verifies all when omitted")
                    .value_parser(clap::value_parser!(String))
                    .value_name("MODULE")
                    .required(false)
                    .index(1)
            )
    );
    cli
}

/// Dispatches the command based on the parsed arguments.
/// This function matches the subcommand used and calls the appropriate handler.
fn dispatch_commands(
    matches: &ArgMatches,
    manifests: &std::collections::HashMap<String, mainstage_core::vm::plugin::PluginDescriptor>,
) {
    match matches.subcommand() {
        Some(("build", sub_m)) => commands::build::handle(sub_m, manifests),

        Some(("run", sub_m)) => commands::run::handle(sub_m, manifests, matches),
        Some(("inspect", sub_m)) => commands::inspect::handle(sub_m),
        Some(("verify-manifest", sub_m)) => commands::verify::handle(sub_m, manifests),
        _ => {
            error!("No valid subcommand was used. Use --help for more information.");
        }
    }
}
