use std::path::PathBuf;

use clap::Parser;

use crate::cmd::Commands;

/// Simple, file-backed task manager CLI.
/// Storage defaults to ./tasks.json or a path passed via --db.
#[derive(Parser)]
#[command(name = "taskcli", version, about = "Daily task management CLI")]
pub struct Cli {
    /// Path to the JSON database file.
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}
