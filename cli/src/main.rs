mod commands;
mod scaffold;

use clap::Command;

fn main() {
    let cli = Command::new("mainstage")
        // Sourced from the crate version (cli/Cargo.toml) so `--version` and the
        // package never drift; bumping the manifest is the single source of truth.
        .version(env!("CARGO_PKG_VERSION"))
        .author("Colton McGraw <https://github.com/colton-mcgraw>")
        .about("A build and automation tool for Mainstage scripts")
        // Running `mainstage` with no subcommand executes the default pipeline.
        .subcommand_negates_reqs(true);

    let cli = commands::setup(cli);
    let matches = cli.get_matches();
    std::process::exit(commands::dispatch(&matches));
}
