//! Top-level CLI entry struct. Parses argv into a [`Commands`] tree.

use std::path::PathBuf;

use clap::Parser;

use crate::cmd::Commands;

/// Hierarchical project-management tool with typed ids and CLAUDE.md per node.
#[derive(Parser)]
#[command(name = "pm", version, about, long_about = None)]
pub struct Cli {
    /// Override the `.pm/` root for v2 verbs. Defaults to walking up from
    /// the current working directory; honoured via the `PM_ROOT` env var.
    #[arg(long, global = true, env = "PM_ROOT")]
    pub pm_root: Option<PathBuf>,

    /// Legacy: explicit path to a v0.9.x `tasks.json` for TUI verbs.
    /// Ignored by v2 verbs.
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}
