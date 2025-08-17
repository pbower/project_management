//! # PM - Project Management CLI
//!
//! A comprehensive command-line project management tool with hierarchical task organisation
//! and an optional terminal user interface (TUI).
//!
//! ## Key Features
//!
//! - **Hierarchical Task Organisation**: Four-level hierarchy (Product → Epic → Task → Subtask) +
//! Milestone
//! - **Rich Task Metadata**: Priority, urgency, process stages, due dates, tags, and more
//! - **Multiple Interfaces**: Full CLI for automation + interactive TUI for visual management
//! - **Multi-Project Support**: Manage multiple projects with project-scoped (local .json) db files.
//! - **Local File Storage**: Simple JSON files with CSV export/import and backup functionality
//! - **Smart Navigation**: Browser-like back navigation and rapid contextual hierarchy drilling
//!
//! ## Quick Start
//!
//! ```bash
//! # Launch interactive menu
//! pm menu
//!
//! # Or launch TUI for most recent project
//! pm ui
//!
//! # Add a task via CLI
//! pm add "Implement user authentication" --project auth-system --tag backend
//!
//! # List tasks
//! pm list
//!
//! # View task details
//! pm view "Implement user authentication"
//! ```
//!
//! ## Installation
//!
//! ```bash
//! git clone <repository-url>
//! cd pm
//! cargo install --path .
//! ```
//!
//! ## Usage Patterns
//!
//! **For Individual Developers**: PM excels at organizing complex software projects from
//! high-level deliverables down to atomic implementation tasks, without the overhead of
//! web-based collaborative tools.
//!
//! **Terminal-Native Workflow**: Integrates seamlessly with development environments,
//! allowing rapid task capture and management without context switching.
//!
//! **Hierarchy Example**:
//! - **Product**: "E-commerce Platform"
//!   - **Epic**: "User Management System"
//!     - **Task**: "User Registration"
//!       - **Subtask**: "Email Validation"
//!       - **Subtask**: "Password Hashing"
//!
//! ## Key Commands
//!
//! - `pm menu` - Interactive project selection menu
//! - `pm ui` - Launch TUI for visual task management
//! - `pm add <title>` - Create new task with optional metadata
//! - `pm list` - View tasks with filtering and tree view options
//! - `pm export` - Export to CSV for reporting/backup
//! - `pm backup` - Create timestamped project backups
//!
//! Data is stored locally in `~/.pm/` with each project as a separate JSON file.
//! We recommend you source control this folder via `git init` and back it up periodically.

use std::path::{Path, PathBuf};

use clap::Parser;

pub mod cli;
pub mod cmd;
pub mod db;
pub mod fields;
pub mod project;
pub mod task;
pub mod tui {
    pub mod colors;
    pub mod app;
    pub mod enums;
    pub mod input;
    pub mod menu;
    pub mod run;
    pub mod task_form;
    pub mod utils;
    pub mod workflow;
    pub mod workflow_run;
}

use cli::Cli;
use cmd::*;
use db::*;
use project::*;

fn main() {
    let cli = Cli::parse();

    // Determine PM directory
    let pm_dir = if let Some(db_path) = cli.db.as_ref() {
        db_path.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf()
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let pm_dir = PathBuf::from(home).join(".pm");
        if let Err(e) = std::fs::create_dir_all(&pm_dir) {
            eprintln!("Failed to create pm directory {}: {}", pm_dir.display(), e);
            std::process::exit(1);
        }
        pm_dir
    };

    // Handle commands that don't need a specific project first
    match &cli.command {
        Commands::Menu => {
            cmd_menu(&pm_dir);
            return;
        },
        Commands::Backup { all: true } => {
            cmd_backup_all(&pm_dir);
            return;
        },
        Commands::Export { output, all_projects: true, all, project, tag } => {
            cmd_export_all(&pm_dir, output.clone(), *all, project.clone(), tag.clone());
            return;
        },
        _ => {}
    }

    // Handle UI and Workflow commands specially to support recent project logic
    match &cli.command {
        Commands::Ui => {
            // For UI command, try to open most recent project or fall back to menu
            if cli.db.is_some() {
                // If user specified --db, use that specific project
                let db_path = cli.db.unwrap();
                cmd_ui(&db_path);
            } else {
                // Auto-select most recent project or show menu
                match get_most_recent_project(&pm_dir) {
                    Ok(Some(project)) => {
                        println!("Opening recent project: {}", project.display_name);
                        cmd_ui(&project.file_path);
                    },
                    _ => {
                        // No recent projects found, show menu
                        cmd_menu(&pm_dir);
                    }
                }
            }
            return;
        },
        Commands::Wf => {
            // For Workflow command, show workflow selection menu
            if cli.db.is_some() {
                // If user specified --db, use that specific project
                let db_path = cli.db.unwrap();
                cmd_wf(&db_path);
            } else {
                // Show workflow project selection menu
                cmd_workflow_menu(&pm_dir);
            }
            return;
        },
        _ => {}
    }

    // For all other commands, determine the database file to use
    let db_path = cli.db.unwrap_or_else(|| {
        // Check if there's a legacy tasks.json file
        let legacy_path = pm_dir.join("tasks.json");
        if legacy_path.exists() {
            legacy_path
        } else {
            // Try to find any project file, or default to a default project
            match discover_projects(&pm_dir) {
                Ok(projects) if !projects.is_empty() => {
                    projects[0].file_path.clone()
                },
                _ => {
                    // Create a default project
                    let default_project = Project::new("Default", &pm_dir);
                    if let Err(e) = default_project.create_if_not_exists() {
                        eprintln!("Failed to create default project: {}", e);
                        std::process::exit(1);
                    }
                    default_project.file_path
                }
            }
        }
    });

    let mut db = Database::load(&db_path);

    match cli.command {
        Commands::Ui => unreachable!("UI command handled above"),
        Commands::Wf => unreachable!("Workflow command handled above"),
        Commands::Add {
            title, template, desc, project, tags, due, parent, kind, priority_level,
            urgency, process_stage, issue_link, pr_link, summary, user_story,
            requirements, artifacts, status,
        } => cmd_add(&mut db, &db_path, title, template, desc, project, tags, due, parent,
                     kind, priority_level, urgency, process_stage, issue_link,
                     pr_link, summary, user_story, requirements, artifacts, status),

        Commands::List { all, status, kind, project, tags, due, tree, sort, limit } =>
            cmd_list(&db, all, status, kind, project, tags, due, tree, sort, limit),

        Commands::View { id, children, parents } => cmd_view(&db, id, children, parents),

        Commands::Update { id, title, desc, project, due, parent, kind, status,
                          add_tags, rm_tags, clear_due, clear_parent } =>
            cmd_update(&mut db, &db_path, id, title, desc, project, due, parent, kind,
                      status, add_tags, rm_tags, clear_due, clear_parent),

        Commands::Complete { id, recurse, tag, project, status } =>
            cmd_complete(&mut db, &db_path, id, recurse, tag, project, status),

        Commands::Reopen { id } => cmd_reopen(&mut db, &db_path, id),

        Commands::Delete { id, cascade, tag, project, status } =>
            cmd_delete(&mut db, &db_path, id, cascade, tag, project, status),

        Commands::Projects => cmd_projects(&db),

        Commands::Tags => cmd_tags(&db),

        Commands::Completions { shell } => cmd_completions(shell),

        Commands::Template { action } => cmd_template(&mut db, &db_path, action),

        Commands::Export { output, all, all_projects, project, tag } => {
            // all_projects: true case is handled earlier, this handles all_projects: false
            assert!(!all_projects, "all_projects case should be handled earlier");
            cmd_export(&db, output, all, project, tag);
        },

        Commands::Import { input, no_backup } =>
            cmd_import(&mut db, &db_path, input, no_backup),

        Commands::Backup { all } =>
            cmd_backup(&db_path, all),

        Commands::Menu => cmd_menu(&db_path.parent().unwrap_or_else(|| Path::new("."))),
    }
}
