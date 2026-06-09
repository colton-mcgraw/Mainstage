mod commands;

use clap::Command;

fn main() {
    let cli = Command::new("mainstage")
        .version("0.1.0")
        .author("Colton McGraw <https://github.com/ColtMcG1>")
        .about("A build and automation tool for Mainstage scripts")
        .subcommand_required(true)
        .arg_required_else_help(true);

    let cli = commands::setup(cli);
    let matches = cli.get_matches();
    commands::dispatch(&matches);
}
