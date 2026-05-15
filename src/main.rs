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

use std::path::PathBuf;

use clap::Parser;

use project_management::cli::Cli;
use project_management::cmd::*;
use project_management::db::*;

fn main() {
    let cli = Cli::parse();

    // Resolve the .pm/ workspace. The --db flag now points at the workspace
    // directory itself; in v2 the storage is the `.pm/` tree, not a single
    // JSON file. With no flag, default to `~/.pm/` so existing global-scope
    // installations keep working.
    let pm_dir = if let Some(db_path) = cli.db.as_ref() {
        db_path.clone()
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let pm_dir = PathBuf::from(home).join(".pm");
        if let Err(e) = std::fs::create_dir_all(&pm_dir) {
            eprintln!("Failed to create pm directory {}: {}", pm_dir.display(), e);
            std::process::exit(1);
        }
        pm_dir
    };

    // Handle commands that don't need a loaded Database.
    match &cli.command {
        Commands::Menu => {
            cmd_menu(&pm_dir);
            return;
        }
        Commands::Backup { all: true } => {
            cmd_backup_all(&pm_dir);
            return;
        }
        Commands::Export {
            output,
            all_projects: true,
            all,
            project,
            tag,
        } => {
            cmd_export_all(&pm_dir, output.clone(), *all, project.clone(), tag.clone());
            return;
        }
        _ => {}
    }

    // UI and Workflow open the workspace directly. The legacy
    // pick-a-project-file flow collapses into "open the workspace"; project
    // selection happens inside the TUI now via PRJ tickets.
    match &cli.command {
        Commands::Ui => {
            cmd_ui(&pm_dir);
            return;
        }
        Commands::Wf => {
            cmd_wf(&pm_dir);
            return;
        }
        _ => {}
    }

    let mut db = Database::load(&pm_dir);

    match cli.command {
        Commands::Ui => unreachable!("UI command handled above"),
        Commands::Wf => unreachable!("Workflow command handled above"),
        Commands::Add {
            title,
            template,
            desc,
            tags,
            due,
            parent,
            kind,
            priority_level,
            urgency,
            process_stage,
            issue_link,
            pr_link,
            summary,
            user_story,
            requirements,
            artifacts,
            status,
        } => cmd_add(
            &mut db,
            &pm_dir,
            title,
            template,
            desc,
            tags,
            due,
            parent,
            kind,
            priority_level,
            urgency,
            process_stage,
            issue_link,
            pr_link,
            summary,
            user_story,
            requirements,
            artifacts,
            status,
        ),

        Commands::List {
            all,
            status,
            kind,
            project,
            tags,
            due,
            tree,
            sort,
            limit,
        } => cmd_list(
            &db, all, status, kind, project, tags, due, tree, sort, limit,
        ),

        Commands::View {
            id,
            children,
            parents,
        } => cmd_view(&db, id, children, parents),

        Commands::Update {
            id,
            title,
            desc,
            due,
            parent,
            kind,
            status,
            add_tags,
            rm_tags,
            clear_due,
            clear_parent,
        } => cmd_update(
            &mut db,
            &pm_dir,
            id,
            title,
            desc,
            due,
            parent,
            kind,
            status,
            add_tags,
            rm_tags,
            clear_due,
            clear_parent,
        ),

        Commands::Complete {
            id,
            recurse,
            tag,
            project,
            status,
        } => cmd_complete(&mut db, &pm_dir, id, recurse, tag, project, status),

        Commands::Reopen { id } => cmd_reopen(&mut db, &pm_dir, id),

        Commands::Delete {
            id,
            cascade,
            tag,
            project,
            status,
        } => cmd_delete(&mut db, &pm_dir, id, cascade, tag, project, status),

        Commands::Projects => cmd_projects(&db),

        Commands::Tags => cmd_tags(&db),

        Commands::Completions { shell } => cmd_completions(shell),

        Commands::Template { action } => cmd_template(&mut db, &pm_dir, action),

        Commands::Export {
            output,
            all,
            all_projects,
            project,
            tag,
        } => {
            // all_projects: true case is handled earlier, this handles all_projects: false
            assert!(!all_projects, "all_projects case should be handled earlier");
            cmd_export(&db, output, all, project, tag);
        }

        Commands::Import { input, no_backup } => cmd_import(&mut db, &pm_dir, input, no_backup),

        Commands::Backup { all } => cmd_backup(&pm_dir, all),

        Commands::Menu => cmd_menu(&pm_dir),

        // v2 lifecycle
        Commands::Init => cmd_init(&pm_dir),
        Commands::Show { id } => cmd_show(&db, &id),
        Commands::Move {
            id,
            new_parent,
            orphan,
        } => {
            cmd_move(&mut db, &pm_dir, &id, new_parent.as_deref(), orphan);
        }

        // v2 content
        Commands::Edit { id, section } => cmd_edit(&pm_dir, &id, section.as_deref()),
        Commands::Context { id, no_memories } => cmd_context(&db, &pm_dir, &id, !no_memories),
        Commands::Materialise { id, output } => cmd_materialise(&db, &pm_dir, &id, output),
        Commands::Artifact { action } => cmd_artifact(&db, &pm_dir, action),

        // v2 metadata
        Commands::SetStatus { id, new_status } => cmd_set_status(&mut db, &pm_dir, &id, new_status),
        Commands::Priority { id, new_priority } => {
            cmd_priority(&mut db, &pm_dir, &id, new_priority)
        }
        Commands::Due { id, when } => cmd_due(&mut db, &pm_dir, &id, &when),
        Commands::Dep { id, op, dep_id } => cmd_dep(&mut db, &pm_dir, &id, &op, &dep_id),
        Commands::Tag { id, ops } => cmd_tag(&mut db, &pm_dir, &id, &ops),
        Commands::Link { id, key, url } => cmd_link(&mut db, &pm_dir, &id, &key, &url),
        Commands::Milestone { id, milestone_id } => {
            cmd_milestone(&mut db, &pm_dir, &id, &milestone_id);
        }

        // v2 views / maintenance
        Commands::Doctor { migrate } => cmd_doctor(&pm_dir, migrate),
        Commands::Search { query } => cmd_search(&pm_dir, &query),

        // Phase 6: lock protocol + activity feed
        Commands::Checkout { id, intent } => cmd_checkout(&pm_dir, &id, intent.as_deref()),
        Commands::Checkin {
            id,
            summary,
            granular,
        } => cmd_checkin(&pm_dir, &id, summary.as_deref(), granular),
        Commands::Heartbeat { id } => cmd_heartbeat(&pm_dir, &id),
        Commands::Next { agent, filter } => cmd_next(&pm_dir, agent.as_deref(), filter.as_deref()),
        Commands::Locks => cmd_locks(&pm_dir),

        // Deferred to later phases
        Commands::Tv { path } => cmd_tv(path.as_deref().unwrap_or(&pm_dir)),
        Commands::Log { id } => cmd_log(&pm_dir, &id),
        Commands::Memory { action } => cmd_memory(&mut db, &pm_dir, action),
        Commands::Mcp => cmd_mcp(&pm_dir),
    }
}
