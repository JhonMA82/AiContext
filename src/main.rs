mod agent;
mod check;
mod cli;
mod config;
mod doctor;
mod output;
mod scan;
mod search;
mod state;
mod tools;

use anyhow::Result;
use clap::Parser;
use cli::{AgentCommand, Cli, Command, ToolsCommand};

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
        Command::Doctor { json } => doctor::cmd_doctor(json),
        Command::Search {
            query,
            text,
            structure,
            impact,
            json,
        } => search::cmd_search(query, text, structure, impact, json),
        Command::Tools { cmd } => match cmd {
            ToolsCommand::Plan { json } => tools::cmd_plan(json),
            ToolsCommand::Status { json } => tools::cmd_status(json),
        },
        Command::Agent { cmd } => match cmd {
            AgentCommand::Install { agent, json } => agent::cmd_install(agent, json),
            AgentCommand::Uninstall { agent, json } => agent::cmd_uninstall(agent, json),
        },
    }
}
