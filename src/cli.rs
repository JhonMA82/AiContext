use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "aicontext",
    version,
    about = "Deterministic repository intelligence CLI"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize .engineering/ context in the current repo (idempotent)
    Init {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long, default_value_t = false)]
        non_interactive: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Collect deterministic facts about the repo (read-only)
    Scan {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Refresh only the generated block of PROJECT_STATE.md
    Sync {
        #[arg(long, default_value_t = false)]
        check: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Compact human/machine status
    Status {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Deterministic completion gate (non-zero exit on real failure)
    Check {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Diagnose CLI, config, state and tooling with exact remediation
    Doctor {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Layered search: knowledge, text (tgrep/rg/git-grep), AST, impact
    Search {
        /// Literal text or symbol to look for (AST pattern with --structure)
        query: String,
        /// Force literal text search (default)
        #[arg(long, default_value_t = false)]
        text: bool,
        /// Treat query as an ast-grep structural pattern
        #[arg(long, default_value_t = false)]
        structure: bool,
        /// Callers/callees/impact (CodeGraph when enabled, else degraded)
        #[arg(long, default_value_t = false)]
        impact: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// External tool detection and recommendations (read-only)
    Tools {
        #[command(subcommand)]
        cmd: ToolsCommand,
    },
    /// Install or remove agent skills managed by AIContext
    Agent {
        #[command(subcommand)]
        cmd: AgentCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// Detect useful tooling and explain why (never installs)
    Plan {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show availability, versions and ownership of known tools
    Status {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// Install the aicontext-adopt skill plus the minimal AGENTS.md pointer
    Install {
        /// Target agent (v0.1: pi)
        agent: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Remove only files owned by AIContext (never foreign files)
    Uninstall {
        /// Target agent (v0.1: pi)
        agent: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}
