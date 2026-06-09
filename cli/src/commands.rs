use clap::{Arg, Command};
use mainstage_core::{analyze, eval_program, parse, Source};

/// Register all CLI subcommands on `cli` and return the augmented [`Command`].
pub fn setup(cli: Command) -> Command {
    cli.subcommand(
        Command::new("parse")
            .about("Parse a .ms file and print its AST (debug tool)")
            .arg(Arg::new("file").required(true).help("Path to the .ms script")),
    )
    .subcommand(
        Command::new("eval")
            .about("Parse, analyze, and evaluate a .ms file; print the resulting context (debug tool)")
            .arg(Arg::new("file").required(true).help("Path to the .ms script")),
    )
}

/// Dispatch the matched subcommand, running it to completion.
///
/// Exits the process with a non-zero code on any error.
pub fn dispatch(matches: &clap::ArgMatches) {
    match matches.subcommand() {
        Some(("parse", sub)) => cmd_parse(sub),
        Some(("eval", sub))  => cmd_eval(sub),
        _ => {
            eprintln!("No subcommand provided. Run with --help for usage.");
            std::process::exit(1);
        }
    }
}

fn cmd_parse(matches: &clap::ArgMatches) {
    let file: &String = matches.get_one("file").unwrap();

    let source = match Source::from_file(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    match parse(&source) {
        Ok(program) => println!("{:#?}", program),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_eval(matches: &clap::ArgMatches) {
    let file: &String = matches.get_one("file").unwrap();

    let source = match Source::from_file(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let program = match parse(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = analyze(&program) {
        eprintln!("{}", e);
        std::process::exit(1);
    }

    let script_dir = std::path::Path::new(file)
        .parent()
        .unwrap_or(std::path::Path::new("."));

    match eval_program(&program, script_dir) {
        Ok(ctx) => println!("{:#?}", ctx),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}
