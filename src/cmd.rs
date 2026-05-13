//! Top-level command tree (PM_DESIGN.md Section 8.1) and TUI launchers.
//!
//! Every CLI verb lives here. Most operate on the typed v2 store in
//! [`crate::store`]; the four TUI variants (`Ui`, `Wf`, `Menu`,
//! `Completions`) wrap the existing ratatui surface in [`crate::tui`].
//! Phase 7 will rebuild the TUI on top of v2 - until then it still reads
//! and writes the v0.9.x `~/.pm/<project>.json` databases via
//! [`crate::db::Database`].

use std::path::Path;

use clap::Subcommand;
use clap_complete::{generate, Shell};

use crate::tui::menu::MenuApp;
use crate::tui::run::run_tui;
use crate::tui::workflow::WorkflowExit;
use crate::tui::workflow_run::run_workflow_tui;
use crate::v2::cli::{ArtifactAction, KindArg, MemoryAction, TemplateAction};

/// Unified top-level subcommand tree. v2 verbs are first-class; the TUI
/// launchers and shell-completion utility sit alongside them.
#[derive(Subcommand)]
pub enum Commands {
    // -------- lifecycle --------

    /// Initialise a fresh `.pm/` in the current directory.
    Init,

    /// Create a new ticket.
    Add {
        /// Ticket title (free text; quote if it contains spaces).
        title: String,
        /// Ticket kind.
        #[arg(long, value_enum)]
        kind: KindArg,
        /// Parent address or leaf id (omit for an orphan ticket).
        #[arg(long)]
        parent: Option<String>,
        /// Override the auto-generated kebab-case slug.
        #[arg(long)]
        slug: Option<String>,
    },

    /// List tickets recorded in `state.json`.
    List {
        /// Filter by kind.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Render in tree form (defaults to flat).
        #[arg(long)]
        tree: bool,
    },

    /// Show a single ticket's metadata and rendered body.
    Show {
        /// Leaf id, address, or slugged form.
        id: String,
    },

    /// Move a ticket under a new parent.
    Move {
        /// Ticket to move (any id form).
        id: String,
        /// New parent address, or `:orphan` to detach.
        dest: String,
    },

    /// Mark a ticket complete (status: done).
    Complete { id: String },

    /// Tombstone a ticket and remove its on-disk directory.
    Delete {
        id: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        force: bool,
    },

    // -------- content --------

    /// Open the ticket's CLAUDE.md in `$EDITOR`.
    Edit {
        id: String,
        /// Position the editor cursor at a specific section heading.
        #[arg(long)]
        section: Option<String>,
    },

    /// Print the composed context view (ancestors plus this ticket).
    Context {
        id: String,
        /// Skip the linked-memories section.
        #[arg(long)]
        no_memories: bool,
    },

    /// Write the composed view to a sidecar file on disk.
    Materialise {
        id: String,
        /// Output path (defaults to `<ticket-dir>/<leaf>.composed.md`).
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },

    /// Manage a ticket's `artifacts/` directory.
    Artifact {
        #[command(subcommand)]
        action: ArtifactAction,
    },

    /// Manage per-kind section templates.
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },

    // -------- metadata --------

    /// Update a ticket's status (e.g. `open`, `in-progress`, `done`).
    Status { id: String, value: String },

    /// Update priority (`must-have`, `nice-to-have`, `cut-first`).
    Priority { id: String, value: String },

    /// Set or clear the due date (`YYYY-MM-DD`, or `none` to clear).
    Due { id: String, value: String },

    /// Manage dependency edges.
    Dep {
        id: String,
        /// `needs` (add) or `drop` (remove).
        op: String,
        other: String,
    },

    /// Add or remove tags. Each `op` is `+name` or `-name`.
    Tag { id: String, ops: Vec<String> },

    /// Set a free-form link (e.g. `github_issue owner/repo#42`).
    Link { id: String, key: String, value: String },

    /// Set the milestone leaf id (or `none` to clear).
    Milestone { id: String, value: String },

    // -------- workflow / memory / views (stubs filled in later phases) --------

    /// Memory verbs (Phase 10 stub).
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Acquire a lock on a ticket (Phase 6 stub).
    Checkout {
        id: String,
        #[arg(long)]
        intent: Option<String>,
    },

    /// Release a lock on a ticket (Phase 6 stub).
    Checkin {
        id: String,
        #[arg(long)]
        summary: Option<String>,
    },

    /// Return the next dependency-ready ticket (Phase 6 stub).
    Next {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        filter: Option<String>,
    },

    /// Show active checkouts (Phase 6 stub).
    Locks,

    /// Full-screen activity feed (Phase 9 stub).
    Tv,

    /// Git log filtered to a ticket's subtree (Phase 5 stub).
    Log { id: String },

    /// Substring search across CLAUDE.md bodies.
    Search { query: String },

    /// Reconcile `state.json` against the on-disk truth.
    Doctor,

    // -------- TUI entry points (existing screens; evolve in Phase 7) --------

    /// Launch the interactive TUI.
    Ui,

    /// Launch the workflow kanban board.
    Wf,

    /// Open the project selection menu.
    Menu,

    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

// ----------------------------- TUI launchers ------------------------------

/// Launch the task TUI against a specific project file.
pub fn cmd_ui(db_path: &Path) {
    if let Err(e) = run_tui(db_path) {
        eprintln!("UI error: {e}");
        std::process::exit(1);
    }
}

/// Launch the workflow kanban TUI; loops so users can edit tasks and return.
pub fn cmd_wf(db_path: &Path) {
    use crate::db::Database;
    use crate::tui::run::run_tui_with_edit;
    loop {
        match run_workflow_tui(db_path) {
            Ok(WorkflowExit::EditTask(task_id)) => {
                let db = Database::load(db_path);
                if db.get(task_id).is_some() {
                    if let Err(err) = run_tui_with_edit(db_path, task_id) {
                        eprintln!("Error running TUI: {}", err);
                        std::process::exit(1);
                    }
                    continue;
                }
            }
            Ok(WorkflowExit::Quit) => break,
            Err(err) => {
                eprintln!("Error running workflow TUI: {}", err);
                std::process::exit(1);
            }
        }
    }
}

/// Launch the project-selection menu TUI.
pub fn cmd_menu(pm_dir: &Path) {
    use crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io;

    enable_raw_mode().unwrap();
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut app = MenuApp::new(pm_dir.to_path_buf()).unwrap();
    let res = app.run(&mut terminal);

    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();

    if let Err(err) = res {
        println!("{:?}", err);
        std::process::exit(1);
    }
    if let Some(project) = app.get_selected_project() {
        if app.should_open_workflow() {
            println!("Opening workflow for: {}", project.display_name);
            cmd_wf(&project.file_path);
        } else {
            println!("Opening project: {}", project.display_name);
            if let Err(err) = run_tui(&project.file_path) {
                eprintln!("Error running TUI: {}", err);
                std::process::exit(1);
            }
        }
    }
}

/// Launch the menu TUI directly into workflow-selection mode.
pub fn cmd_workflow_menu(pm_dir: &Path) {
    use crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io;

    enable_raw_mode().unwrap();
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut app = MenuApp::new(pm_dir.to_path_buf()).unwrap();
    app.start_workflow_selection();
    let res = app.run(&mut terminal);

    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();

    if let Err(err) = res {
        println!("{:?}", err);
        std::process::exit(1);
    }
    if let Some(project) = app.get_selected_project() {
        if app.should_open_workflow() {
            println!("Opening workflow for: {}", project.display_name);
            cmd_wf(&project.file_path);
        }
    }
}

// ----------------------------- utilities ----------------------------------

/// Generate shell completion scripts for the unified Commands tree.
pub fn cmd_completions(shell: Shell) {
    use clap::CommandFactory;
    use crate::cli::Cli;

    let mut app = Cli::command();
    let app_name = app.get_name().to_string();
    generate(shell, &mut app, app_name, &mut std::io::stdout());
}
