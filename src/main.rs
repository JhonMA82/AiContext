mod check;
mod cli;
mod config;
mod output;
mod scan;
mod state;
mod tools;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            output::print_error(&e, false);
            5
        }
    };
    std::process::exit(code);
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Init {
            profile,
            non_interactive,
            json,
        } => state::cmd_init(profile, non_interactive, json),
        Command::Scan { json } => scan::cmd_scan(json),
        Command::Sync { check, json } => state::cmd_sync(check, json),
        Command::Status { json } => state::cmd_status(json),
        Command::Check { json } => check::cmd_check(json),
    }
}
