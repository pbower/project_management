use std::path::PathBuf;

use clap::Parser;

use crate::cmd::Commands;

/// SpaceCell Thunder CLI. Hierarchical project management cockpit with an
/// embedded kanban board, three-tier memory, and an MCP server for AI
/// agents.
#[derive(Parser)]
#[command(name = "spacecell", version, about = "SpaceCell Thunder")]
pub struct Cli {
    /// Workspace directory (defaults to `~/.pm/`). The `.pm/` tree under
    /// this path is the storage root.
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    /// CLI subcommand. Omit to launch the main shell.
    #[command(subcommand)]
    pub command: Option<Commands>,
}
