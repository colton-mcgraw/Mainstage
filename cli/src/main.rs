mod commands;
mod scaffold;

use clap::Command;

fn main() {
    let cli = Command::new("mainstage")
        .version("0.1.0")
        .author("Colton McGraw <https://github.com/ColtMcG1>")
        .about("A build and automation tool for Mainstage scripts")
        // Running `mainstage` with no subcommand executes the default pipeline.
        .subcommand_negates_reqs(true);

    let cli = commands::setup(cli);
    let matches = cli.get_matches();
    std::process::exit(commands::dispatch(&matches));
}
