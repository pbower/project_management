use std::path::PathBuf;

use clap::Parser;

use crate::cmd::Commands;

/// Simple, file-backed task manager CLI.
/// Storage defaults to ./tasks.json or a path passed via --db.
#[derive(Parser)]
#[command(name = "pm", version, about = "Local-first project management CLI with a hierarchical TUI, agent-ready CLAUDE.md context, artifact tracking, three-tier memory, and an MCP server.")]
pub struct Cli {
    /// Path to the JSON database file.
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}
