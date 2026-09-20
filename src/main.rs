mod adapters;
mod agent;
mod check;
mod cli;
mod config;
mod doctor;
mod output;
mod scan;
mod search;
mod self_update;
mod state;
mod tools;
mod tools_install;

use anyhow::Result;
use clap::Parser;
use cli::{AgentCommand, Cli, Command, SelfCommand, ToolsCommand};

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            output::print_error(&e, false);
            // Spec §39: a missing required tool is exit 3, distinct from
            // generic failures (5). Any other error keeps 5.
            let missing_tool = e
                .downcast_ref::<output::AiError>()
                .is_some_and(|a| a.code() == "MISSING_REQUIRED_TOOL");
            if missing_tool {
                3
            } else {
                5
            }
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
            recursive,
        } => state::cmd_init(profile, non_interactive, json, recursive),
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
            scope,
            subproject,
            json,
        } => search::cmd_search(query, text, structure, impact, json, scope, subproject),
        Command::Tools { cmd } => match cmd {
            ToolsCommand::Plan { json } => tools::cmd_plan(json),
            ToolsCommand::Status { json } => tools::cmd_status(json),
            ToolsCommand::Install {
                tool,
                recommended,
                yes,
                json,
            } => tools_install::cmd_install(tool, recommended, yes, json),
            ToolsCommand::Update { yes, json } => tools_install::cmd_update(yes, json),
            ToolsCommand::Uninstall {
                tool,
                managed,
                yes,
                json,
            } => tools_install::cmd_uninstall(tool, managed, yes, json),
        },
        Command::Agent { cmd } => match cmd {
            AgentCommand::Install { agent, json } => agent::cmd_install(agent, json),
            AgentCommand::Uninstall { agent, json } => agent::cmd_uninstall(agent, json),
        },
        Command::SelfMgmt { cmd } => match cmd {
            SelfCommand::Update { check, yes, json } => self_update::cmd_update(check, yes, json),
            SelfCommand::Uninstall { managed, yes, json } => {
                self_update::cmd_uninstall(managed, yes, json)
            }
        },
        Command::Completion { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "aicontext",
                &mut std::io::stdout(),
            );
            Ok(0)
        }
    }
}
