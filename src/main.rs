//! # PM - Project Management CLI
//!
//! Hierarchical project-management tool. The CLI surface operates on a `.pm/`
//! directory of typed tickets (PM_DESIGN.md Section 4); the TUI launchers
//! (`ui`, `wf`, `menu`) currently still drive the v0.9.x `tasks.json` flow
//! and will be rebuilt on top of v2 in Phase 7.

use std::path::PathBuf;

use clap::Parser;

use project_management::cli::Cli;
use project_management::cmd::{cmd_menu, cmd_ui, cmd_wf, cmd_workflow_menu, cmd_completions, Commands};
use project_management::project::{discover_projects, get_most_recent_project, Project};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // ---- TUI launchers (existing ratatui screens, evolve in Phase 7) ----
        Commands::Menu => {
            let pm_dir = resolve_legacy_pm_dir(cli.db.as_deref());
            cmd_menu(&pm_dir);
        }
        Commands::Ui => {
            let pm_dir = resolve_legacy_pm_dir(cli.db.as_deref());
            if let Some(db) = cli.db {
                cmd_ui(&db);
            } else {
                match get_most_recent_project(&pm_dir) {
                    Ok(Some(project)) => {
                        println!("Opening recent project: {}", project.display_name);
                        cmd_ui(&project.file_path);
                    }
                    _ => cmd_menu(&pm_dir),
                }
            }
        }
        Commands::Wf => {
            let pm_dir = resolve_legacy_pm_dir(cli.db.as_deref());
            if let Some(db) = cli.db {
                cmd_wf(&db);
            } else {
                cmd_workflow_menu(&pm_dir);
            }
        }
        Commands::Completions { shell } => cmd_completions(shell),

        // ---- v2 verbs operating on `.pm/` ----
        other => {
            if let Err(e) = project_management::v2::cmd::dispatch(other, cli.pm_root) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// Resolve the legacy `~/.pm/<project>.json` directory used by the TUI
/// launchers. Used by `ui`/`wf`/`menu` only; v2 verbs use `cli.pm_root`.
fn resolve_legacy_pm_dir(explicit_db: Option<&std::path::Path>) -> PathBuf {
    if let Some(db) = explicit_db {
        return db.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let pm_dir = PathBuf::from(home).join(".pm");
    if let Err(e) = std::fs::create_dir_all(&pm_dir) {
        eprintln!("Failed to create pm directory {}: {}", pm_dir.display(), e);
        std::process::exit(1);
    }
    // Seed a default project file if none exist, so the TUI has something to
    // open. Matches the previous v0.9.x behaviour.
    if let Ok(projects) = discover_projects(&pm_dir) {
        if projects.is_empty() && !pm_dir.join("tasks.json").exists() {
            let default = Project::new("Default", &pm_dir);
            let _ = default.create_if_not_exists();
        }
    }
    pm_dir
}
