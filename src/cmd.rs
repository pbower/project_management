//! Command implementations for the CLI interface.
//!
//! This module contains all the command handlers that implement the various
//! subcommands available in the CLI, from basic CRUD operations to complex
//! hierarchical queries and the TUI interface.

use clap::Subcommand;
use clap_complete::{generate, Shell};

use crate::db::*;
use crate::fields::*;
use crate::mcp::server::run as run_mcp_server;
use crate::memory::store::MemoryContext;
use crate::memory::{
    lookup_by_name, promote_memory, write_memory, MemoryFile, MemoryHit, MemoryType, Scope,
};
use crate::store::front_matter::MemoryRef;
use crate::store::id::{IdInput, LeafId};
use crate::store::migrate::kind_to_prefix;
use crate::task::{Task, TaskTemplate};
use crate::tui::menu::MenuApp;
use crate::tui::run::{run_activity_view, run_tui, run_tui_with_edit};
use crate::tui::workflow::WorkflowExit;
use crate::tui::workflow_run::run_workflow_tui;
use chrono::{Local, NaiveDate, TimeZone, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum Commands {
    /// Launch the interactive UI interface.
    Ui,

    /// Launch the workflow kanban board interface.
    Wf,

    /// Add a new task.
    Add {
        /// Short title for the task.
        title: String,
        /// Use a template for default values.
        #[arg(long)]
        template: Option<String>,
        /// Optional longer description.
        #[arg(long)]
        desc: Option<String>,
        /// Comma-separated tags. May be repeated.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Due date: YYYY-MM-DD, "today", "tomorrow", or "in Nd".
        #[arg(long)]
        due: Option<String>,
        /// Parent task ID or name.
        #[arg(long)]
        parent: Option<String>,
        /// Item kind: product | epic | task | subtask | milestone.
        #[arg(long, value_enum, default_value_t = Kind::Task)]
        kind: Kind,
        /// Priority level: must-have | nice-to-have | cut-first.
        #[arg(long, value_enum)]
        priority_level: Option<Priority>,
        /// Urgency: urgent-important | urgent-not-important | not-urgent-important | not-urgent-not-important.
        #[arg(long, value_enum)]
        urgency: Option<Urgency>,
        /// Process stage: ideation | design | prototyping | implementation | testing | refinement | release.
        #[arg(long, value_enum)]
        process_stage: Option<ProcessStage>,
        /// Issue link (URL).
        #[arg(long)]
        issue_link: Option<String>,
        /// PR link (URL).
        #[arg(long)]
        pr_link: Option<String>,
        /// Summary (one-line description).
        #[arg(long)]
        summary: Option<String>,
        /// User story.
        #[arg(long)]
        user_story: Option<String>,
        /// Requirements specification.
        #[arg(long)]
        requirements: Option<String>,
        /// Artifacts (file paths, comma-separated).
        #[arg(long)]
        artifacts: Vec<String>,
        /// Status: open | in-progress | done.
        #[arg(long, value_enum, default_value_t = Status::Open)]
        status: Status,
    },

    /// List tasks with optional filters.
    List {
        /// Include completed tasks.
        #[arg(long)]
        all: bool,
        /// Filter by status.
        #[arg(long, value_enum)]
        status: Option<Status>,
        /// Filter by kind.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Filter by project.
        #[arg(long)]
        project: Option<String>,
        /// Filter by tag. May be repeated. Accepts comma-separated.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Due filter: today | this-week | overdue | none.
        #[arg(long, value_enum)]
        due: Option<DueFilter>,
        /// Render as a tree across parent-child relationships.
        #[arg(long)]
        tree: bool,
        /// Sort key.
        #[arg(long, value_enum, default_value_t = SortKey::Due)]
        sort: SortKey,
        /// Limit number of rows printed.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// View a single task by ID or name.
    View {
        /// Task ID or name to view
        id: String,
        /// Show child subtree.
        #[arg(long)]
        children: bool,
        /// Show ancestor chain.
        #[arg(long)]
        parents: bool,
    },

    /// Update fields on a task.
    Update {
        /// Task ID or name to update
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long)]
        due: Option<String>,
        /// Parent task ID or name.
        #[arg(long)]
        parent: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        #[arg(long, value_enum)]
        status: Option<Status>,
        /// Add tags. May be repeated and comma-separated.
        #[arg(long = "add-tag")]
        add_tags: Vec<String>,
        /// Remove tags. May be repeated and comma-separated.
        #[arg(long = "rm-tag")]
        rm_tags: Vec<String>,
        /// Clear due date.
        #[arg(long)]
        clear_due: bool,
        /// Clear parent.
        #[arg(long)]
        clear_parent: bool,
    },

    /// Mark a task done.
    Complete {
        /// Task ID or name to complete (mutually exclusive with bulk options)
        id: Option<String>,
        /// Also mark all descendants done.
        #[arg(long)]
        recurse: bool,
        /// Complete all tasks with this tag
        #[arg(long)]
        tag: Option<String>,
        /// Complete all tasks in this project
        #[arg(long)]
        project: Option<String>,
        /// Complete all tasks with this status
        #[arg(long, value_enum)]
        status: Option<Status>,
    },

    /// Reopen a task (status open).
    Reopen {
        /// Task ID or name to reopen
        id: String,
    },

    /// Delete a task by ID or name.
    Delete {
        /// Task ID or name to delete (mutually exclusive with bulk options)
        id: Option<String>,
        /// Cascade into all descendants.
        #[arg(long)]
        cascade: bool,
        /// Delete all tasks with this tag
        #[arg(long)]
        tag: Option<String>,
        /// Delete all tasks in this project
        #[arg(long)]
        project: Option<String>,
        /// Delete all tasks with this status
        #[arg(long, value_enum)]
        status: Option<Status>,
    },

    /// List distinct projects.
    Projects,

    /// List distinct tags and counts.
    Tags,

    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Manage task templates.
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },

    /// Export tasks to CSV format.
    Export {
        /// Output file path (default: tasks.csv)
        #[arg(long, short)]
        output: Option<String>,
        /// Include completed tasks
        #[arg(long)]
        all: bool,
        /// Export all projects instead of just current project
        #[arg(long)]
        all_projects: bool,
        /// Filter by project
        #[arg(long)]
        project: Option<String>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
    },

    /// Import tasks from CSV format.
    Import {
        /// Input CSV file path
        input: String,
        /// Skip creating backup before import
        #[arg(long)]
        no_backup: bool,
    },

    /// Create timestamped backup of current project or all projects.
    Backup {
        /// Backup all projects instead of just current
        #[arg(long)]
        all: bool,
    },

    /// Open project main menu (interactive mode).
    Menu,

    // ----- v2 lifecycle verbs -----
    /// Initialise a `.pm/` workspace in the current directory.
    Init,

    /// Show a ticket's front-matter and CLAUDE.md sections.
    Show {
        /// Ticket id (e.g. `TSK7`, `PRJ1-PRD3-EPC7-TSK22`).
        id: String,
    },

    /// Move a ticket under a different parent.
    #[command(name = "move")]
    Move {
        /// Ticket id to move.
        id: String,
        /// New parent id; pass `--orphan` to clear the parent.
        new_parent: Option<String>,
        /// Promote the ticket to orphan-scope (no parent).
        #[arg(long)]
        orphan: bool,
    },

    // ----- v2 content verbs -----
    /// Open a ticket's CLAUDE.md in `$EDITOR`.
    Edit {
        /// Ticket id.
        id: String,
        /// Position the cursor at this section heading (e.g. `--section "User Story"`).
        #[arg(long)]
        section: Option<String>,
    },

    /// Print the composed CLAUDE.md chain for a ticket.
    Context {
        /// Ticket id.
        id: String,
        /// Suppress the linked memories section.
        #[arg(long)]
        no_memories: bool,
    },

    /// Write a composed view to disk next to the ticket.
    Materialise {
        /// Ticket id.
        id: String,
        /// Output file path. Defaults to `<ticket-dir>/COMPOSED.md`.
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Artifact directory management.
    Artifact {
        #[command(subcommand)]
        action: ArtifactAction,
    },

    // ----- v2 metadata verbs -----
    /// Set a ticket's status.
    SetStatus {
        /// Ticket id.
        id: String,
        /// New status.
        #[arg(value_enum)]
        new_status: Status,
    },

    /// Set a ticket's priority.
    Priority {
        /// Ticket id.
        id: String,
        /// New priority.
        #[arg(value_enum)]
        new_priority: Priority,
    },

    /// Set a ticket's due date.
    Due {
        /// Ticket id.
        id: String,
        /// Due date in any form `parse_due_input` accepts (e.g. `"next friday"`).
        when: String,
    },

    /// Manage a ticket's dependencies.
    Dep {
        /// Ticket id.
        id: String,
        /// Operation: `needs` or `remove`.
        op: String,
        /// Dependency id (the ticket the operation targets).
        dep_id: String,
    },

    /// Add or remove tags on a ticket.
    Tag {
        /// Ticket id.
        id: String,
        /// Tag ops in the form `+foo` to add, `-bar` to remove. Values may
        /// start with `-` because clap is told these are not flags.
        #[arg(allow_hyphen_values = true)]
        ops: Vec<String>,
    },

    /// Set a key in a ticket's `links:` map.
    Link {
        /// Ticket id.
        id: String,
        /// Link key (e.g. `github_issue`).
        key: String,
        /// Link value (URL or `owner/repo#issue`).
        url: String,
    },

    /// Attach a ticket to a milestone.
    Milestone {
        /// Ticket id.
        id: String,
        /// Milestone id (`MLSn`).
        milestone_id: String,
    },

    // ----- v2 views and maintenance -----
    /// Rebuild state.json from the on-disk tree. Pass `--migrate` to import a
    /// legacy `tasks.json` archive into the workspace via the bridge.
    Doctor {
        /// Run the legacy `tasks.json` migration into the current workspace.
        #[arg(long)]
        migrate: bool,
    },

    /// Search CLAUDE.md content across the workspace.
    Search {
        /// Substring or regex pattern.
        query: String,
    },

    /// Acquire a soft lock on a ticket.
    Checkout {
        /// Ticket id.
        id: String,
        /// Optional intent string recorded on the lock.
        #[arg(long)]
        intent: Option<String>,
    },

    /// Release a lock and commit work in progress.
    Checkin {
        /// Ticket id.
        id: String,
        /// Summary written to the activity feed.
        #[arg(long)]
        summary: Option<String>,
        /// Keep the individual checkout-span commits instead of squashing
        /// them into a single checkin commit.
        #[arg(long)]
        granular: bool,
    },

    /// Refresh the heartbeat on a held lock so it does not go stale.
    Heartbeat {
        /// Ticket id.
        id: String,
    },

    /// Return the next ready task for an agent.
    Next {
        /// Acting agent name (defaults to `PM_AGENT_ID`).
        #[arg(long)]
        agent: Option<String>,
        /// Filter expression.
        #[arg(long)]
        filter: Option<String>,
    },

    /// List active locks across the workspace.
    Locks,

    /// Open the full-screen activity feed (Mode 3 renderer in a standalone
    /// loop). Defaults to the current workspace; pass a path to monitor a
    /// different `.pm/` directory.
    Tv {
        /// Path to the `.pm/` directory (or any directory that contains
        /// `.pm/`). Defaults to the resolved `pm_dir`.
        #[arg(value_name = "PATH")]
        path: Option<std::path::PathBuf>,
    },

    /// Filter git log to the ticket's slice of the tree.
    Log {
        /// Ticket id.
        id: String,
    },

    /// Memory tier management (user / project / ticket scopes).
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Run the stdio MCP server. Exposes the 14-tool surface over JSON-RPC
    /// 2.0; see docs/mcp.md for the catalogue. Runs until stdin closes.
    Mcp,
}

#[derive(Subcommand)]
pub enum ArtifactAction {
    /// Drop a file into a ticket's `artifacts/` directory and sweep.
    Add {
        /// Ticket id.
        id: String,
        /// Path to the file to add.
        path: PathBuf,
        /// Description for the artifact entry.
        #[arg(long)]
        desc: Option<String>,
    },
    /// Rename an artifact, preserving its description.
    Rename {
        /// Ticket id.
        id: String,
        /// Existing filename.
        old: String,
        /// New filename.
        new: String,
    },
    /// List artifacts for a ticket.
    List {
        /// Ticket id.
        id: String,
    },
}

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Link a memory to a ticket.
    Link {
        /// Ticket id.
        id: String,
        /// Memory name.
        name: String,
    },
    /// Unlink a memory from a ticket.
    Unlink {
        /// Ticket id.
        id: String,
        /// Memory name.
        name: String,
    },
    /// List memories linked to a ticket.
    List {
        /// Ticket id.
        id: String,
    },
    /// Write a memory at the given scope.
    Write {
        /// Scope: `user`, `project`, or `ticket`. The `user` scope is
        /// reserved for Claude Code's auto-memory store; PM rejects direct
        /// writes there. Use `pm memory promote --to user` to demote a
        /// project memory into the user tier.
        #[arg(long)]
        scope: String,
        /// Memory type: `feedback`, `project`, `reference`, or `user`.
        #[arg(long)]
        ty: String,
        /// Memory name (file basename, no extension).
        #[arg(long)]
        name: String,
        /// Optional description recorded in the file's front-matter.
        #[arg(long)]
        desc: Option<String>,
        /// Ticket id (required when `--scope ticket`).
        #[arg(long)]
        ticket: Option<String>,
        /// Project leaf, e.g. `PRJ1` (required when `--scope project` and
        /// the workspace has more than one project).
        #[arg(long)]
        project: Option<String>,
        /// Memory body content.
        content: String,
    },
    /// Promote a memory between scopes.
    Promote {
        /// Memory name.
        name: String,
        /// Target scope.
        #[arg(long)]
        to: String,
    },
    /// Print a memory's contents.
    Show {
        /// Memory name.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum TemplateAction {
    /// Save a task as a template.
    Save {
        /// Task ID or name to save as template
        task_id: String,
        /// Template name
        template_name: String,
    },
    /// List all available templates.
    List,
    /// Delete a template.
    Delete {
        /// Template name to delete
        template_name: String,
    },
    /// Open a per-kind section template in `$EDITOR`.
    Edit {
        /// Ticket kind (project|product|epic|task|subtask|milestone).
        kind: String,
    },
    /// Re-apply the section template to an existing ticket, preserving any
    /// content under sections that match the template by name.
    Apply {
        /// Ticket id.
        id: String,
    },
    /// Create a new template from scratch.
    Create {
        /// Template name
        name: String,
        /// Title template (can include {title} placeholder)
        #[arg(long)]
        title_template: Option<String>,
        /// Description template
        #[arg(long)]
        description: Option<String>,
        /// Default tags (comma-separated)
        #[arg(long)]
        tags: Option<String>,
        /// Default kind
        #[arg(long, value_enum, default_value_t = Kind::Task)]
        kind: Kind,
        /// Default priority
        #[arg(long, value_enum)]
        priority: Option<Priority>,
        /// Default urgency
        #[arg(long, value_enum)]
        urgency: Option<Urgency>,
        /// Default process stage
        #[arg(long, value_enum)]
        process_stage: Option<ProcessStage>,
        /// Default status
        #[arg(long, value_enum, default_value_t = Status::Open)]
        status: Status,
    },
}

/// Launch the terminal user interface.
pub fn cmd_ui(db_path: &Path) {
    if let Err(e) = run_tui(db_path) {
        eprintln!("UI error: {e}");
        std::process::exit(1);
    }
}

/// Add a new task to the database.
pub fn cmd_add(
    db: &mut Database,
    db_path: &Path,
    title: String,
    template: Option<String>,
    desc: Option<String>,
    tags: Vec<String>,
    due: Option<String>,
    parent: Option<String>,
    kind: Kind,
    priority_level: Option<Priority>,
    urgency: Option<Urgency>,
    process_stage: Option<ProcessStage>,
    issue_link: Option<String>,
    pr_link: Option<String>,
    summary: Option<String>,
    user_story: Option<String>,
    requirements: Option<String>,
    artifacts: Vec<String>,
    status: Status,
) {
    // Apply template defaults if specified
    let (
        task_kind,
        final_tags,
        final_priority,
        final_urgency,
        final_process_stage,
        final_status,
        final_desc,
    ) = if let Some(template_name) = template {
        let template = db
            .state
            .templates
            .iter()
            .find(|t| t.name == template_name)
            .cloned();

        match template {
            Some(tmpl) => {
                let template_tags = if tags.is_empty() {
                    tmpl.tags
                } else {
                    split_and_normalise_tags(&tags)
                };
                (
                    if kind == Kind::Task { tmpl.kind } else { kind },
                    template_tags,
                    priority_level.or(tmpl.priority_level),
                    urgency.or(tmpl.urgency),
                    process_stage.or(tmpl.process_stage),
                    if status == Status::Open {
                        tmpl.status
                    } else {
                        status
                    },
                    desc.or(tmpl.description_template.clone()),
                )
            }
            None => {
                eprintln!("Template '{}' not found", template_name);
                std::process::exit(1);
            }
        }
    } else {
        (
            kind,
            split_and_normalise_tags(&tags),
            priority_level,
            urgency,
            process_stage,
            status,
            desc,
        )
    };

    let now_utc = Utc::now().timestamp();
    let id = db.allocate_id(kind_to_prefix(task_kind));

    // Resolve and validate parent
    let parent_id = if let Some(parent_str) = parent {
        match resolve_task_identifier(&parent_str, db) {
            Ok(pid) => {
                if pid == id {
                    eprintln!("Parent cannot equal child.");
                    std::process::exit(1);
                }

                // Check hierarchy rules
                if let Some(parent_task) = db.get(pid) {
                    if !validate_hierarchy(parent_task.kind, task_kind) {
                        eprintln!("Invalid hierarchy: {} cannot be child of {}. Valid hierarchy: Project > Product > Epic > Task > Subtask",
                            format_kind(task_kind), format_kind(parent_task.kind));
                        std::process::exit(1);
                    }
                }
                Some(pid)
            }
            Err(e) => {
                eprintln!("Error resolving parent: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let due = due.as_deref().and_then(parse_due_input);
    let artifacts_list = artifacts
        .iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let task = Task {
        id,
        title,
        summary,
        description: final_desc,
        user_story,
        requirements,
        tags: final_tags,
        deps: Vec::new(),
        milestone: None,
        memories: Vec::new(),
        due,
        parent: parent_id,
        kind: task_kind,
        status: final_status,
        priority_level: final_priority,
        urgency: final_urgency,
        process_stage: final_process_stage,
        issue_link,
        pr_link,
        artifacts: artifacts_list,
        created_at_utc: now_utc,
        updated_at_utc: now_utc,
    };
    let title_for_msg = task.title.clone();
    db.tasks.push(task);
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
    commit_or_warn(
        db_path,
        &commit_subject_for(id, "add", Some(&title_for_msg)),
    );
    emit_or_warn(db_path, "add", Some(id), Some(&title_for_msg));
    println!("Added task {}", id);
}

/// List tasks with optional filtering and sorting.
pub fn cmd_list(
    db: &Database,
    all: bool,
    status: Option<Status>,
    kind: Option<Kind>,
    project: Option<String>,
    tags: Vec<String>,
    due: Option<DueFilter>,
    tree: bool,
    sort: SortKey,
    limit: Option<usize>,
) {
    let tags = split_and_normalise_tags(&tags);
    let today = Local::now().date_naive();
    let (week_start, week_end) = start_end_of_this_week(today);

    let mut filtered: Vec<&Task> = db
        .tasks
        .iter()
        .filter(|t| {
            if !all && t.status == Status::Done {
                return false;
            }
            if let Some(s) = status {
                if t.status != s {
                    return false;
                }
            }
            if let Some(k) = kind {
                if t.kind != k {
                    return false;
                }
            }
            if let Some(ref p) = project {
                if project_label(db, t) != *p {
                    return false;
                }
            }
            if !tags.is_empty() {
                let tagset: BTreeSet<_> = t.tags.iter().cloned().collect();
                for tg in &tags {
                    if !tagset.contains(tg) {
                        return false;
                    }
                }
            }
            if let Some(df) = due {
                match df {
                    DueFilter::Today => {
                        if t.due != Some(today) {
                            return false;
                        }
                    }
                    DueFilter::ThisWeek => {
                        if let Some(d) = t.due {
                            if d < week_start || d > week_end {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    DueFilter::Overdue => {
                        if let Some(d) = t.due {
                            if d >= today {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    DueFilter::None => {
                        if t.due.is_some() {
                            return false;
                        }
                    }
                }
            }
            true
        })
        .collect();

    match sort {
        SortKey::Due => filtered.sort_by_key(|t| (t.due.unwrap_or(NaiveDate::MAX), t.id)),
        SortKey::Priority => {
            filtered.sort_by(|a, b| {
                // Sort by priority_level first (MustHave=0, NiceToHave=1, CutFirst=2, None=3)
                let a_priority = match a.priority_level {
                    Some(Priority::MustHave) => 0,
                    Some(Priority::NiceToHave) => 1,
                    Some(Priority::CutFirst) => 2,
                    None => 3,
                };
                let b_priority = match b.priority_level {
                    Some(Priority::MustHave) => 0,
                    Some(Priority::NiceToHave) => 1,
                    Some(Priority::CutFirst) => 2,
                    None => 3,
                };

                // Then by urgency (UrgentImportant=0, UrgentNotImportant=1, NotUrgentImportant=2, NotUrgentNotImportant=3, None=4)
                let a_urgency = match a.urgency {
                    Some(Urgency::UrgentImportant) => 0,
                    Some(Urgency::UrgentNotImportant) => 1,
                    Some(Urgency::NotUrgentImportant) => 2,
                    Some(Urgency::NotUrgentNotImportant) => 3,
                    None => 4,
                };
                let b_urgency = match b.urgency {
                    Some(Urgency::UrgentImportant) => 0,
                    Some(Urgency::UrgentNotImportant) => 1,
                    Some(Urgency::NotUrgentImportant) => 2,
                    Some(Urgency::NotUrgentNotImportant) => 3,
                    None => 4,
                };

                // Finally by ID for stable sort
                a_priority
                    .cmp(&b_priority)
                    .then(a_urgency.cmp(&b_urgency))
                    .then(a.id.cmp(&b.id))
            });
        }
        SortKey::Id => filtered.sort_by_key(|t| t.id),
    }

    if let Some(n) = limit {
        filtered.truncate(n);
    }

    if tree {
        // Compute depths for indentation using ancestry in the full DB.
        let mut depth_map: HashMap<LeafId, usize> = HashMap::new();
        for t in &db.tasks {
            let mut depth = 0usize;
            let mut cur = t.parent;
            while let Some(pid) = cur {
                depth += 1;
                cur = db.get(pid).and_then(|p| p.parent);
                if depth > 64 {
                    break; // cycle guard
                }
            }
            depth_map.insert(t.id, depth);
        }
        print_table(db, &filtered, Some(&depth_map));
    } else {
        print_table(db, &filtered, None);
    }
}

/// View detailed information about a specific task.
pub fn cmd_view(db: &Database, id: String, children: bool, parents: bool) {
    let task_id = match resolve_task_identifier(&id, db) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Error resolving task: {}", e);
            std::process::exit(1);
        }
    };

    let Some(task) = db.get(task_id).cloned() else {
        eprintln!("Task {} not found.", task_id);
        std::process::exit(1);
    };
    let today = Local::now().date_naive();
    let project_for_view = project_label(db, &task);
    println!("ID:           {}", task.id);
    println!("Title:        {}", task.title);
    println!("Kind:         {}", format_kind(task.kind));
    println!("Status:       {}", format_status(task.status));
    println!("Priority:     {}", format_priority(task.priority_level));
    println!("Project:      {}", project_for_view);
    println!(
        "Due:          {}",
        match task.due {
            Some(d) => format!("{d} ({})", format_due_relative(Some(d), today)),
            None => "-".into(),
        }
    );
    println!(
        "Parent:       {}",
        task.parent
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!(
        "Tags:         {}",
        if task.tags.is_empty() {
            "-".into()
        } else {
            task.tags.join(",")
        }
    );
    println!(
        "Created UTC:  {}",
        Utc.timestamp_opt(task.created_at_utc, 0)
            .single()
            .unwrap()
            .to_rfc3339()
    );
    println!(
        "Updated UTC:  {}",
        Utc.timestamp_opt(task.updated_at_utc, 0)
            .single()
            .unwrap()
            .to_rfc3339()
    );
    println!(
        "Description:\n{}\n",
        task.description.unwrap_or_else(|| "-".into())
    );

    let child_map = build_children_map(&db.tasks);

    if parents {
        let chain = collect_ancestors(task_id, db);
        if chain.is_empty() {
            println!("Ancestors: -");
        } else {
            println!(
                "Ancestors (closest first): {}",
                chain
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
    }

    if children {
        println!("Children:");
        if let Some(_children) = child_map.get(&task_id) {
            // Depth-first print.
            let idx = db.index();
            pub fn dfs(
                id: LeafId,
                child_map: &BTreeMap<LeafId, Vec<LeafId>>,
                idx: &HashMap<LeafId, usize>,
                db: &Database,
                depth: usize,
            ) {
                if let Some(children) = child_map.get(&id) {
                    for &c in children {
                        if let Some(&i) = idx.get(&c) {
                            let t = &db.tasks[i];
                            println!(
                                "{}- {} [{}] ({})",
                                "  ".repeat(depth),
                                t.title,
                                format_status(t.status),
                                t.id
                            );
                            dfs(c, child_map, idx, db, depth + 1);
                        }
                    }
                }
            }
            dfs(task_id, &child_map, &idx, db, 1);
        } else {
            println!("  -");
        }
    }
}

/// Update an existing task's fields.
pub fn cmd_update(
    db: &mut Database,
    db_path: &Path,
    id: String,
    title: Option<String>,
    desc: Option<String>,
    due: Option<String>,
    parent: Option<String>,
    kind: Option<Kind>,
    status: Option<Status>,
    add_tags: Vec<String>,
    rm_tags: Vec<String>,
    clear_due: bool,
    clear_parent: bool,
) {
    let task_id = match resolve_task_identifier(&id, db) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Error resolving task: {}", e);
            std::process::exit(1);
        }
    };

    // Resolve parent if provided
    let parent_id = if let Some(parent_str) = parent {
        match resolve_task_identifier(&parent_str, db) {
            Ok(pid) => Some(pid),
            Err(e) => {
                eprintln!("Error resolving parent: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    // Validate parent exists and won't cause cycles before getting mutable borrow
    if let Some(pid) = parent_id {
        if pid == task_id {
            eprintln!("Parent cannot equal child.");
            std::process::exit(1);
        }
        if db.get(pid).is_none() {
            eprintln!("Parent ID {pid} does not exist.");
            std::process::exit(1);
        }
        // Detect cycle.
        let mut cur = Some(pid);
        let mut hops = 0;
        while let Some(p) = cur {
            if p == task_id {
                eprintln!("Setting parent would create a cycle.");
                std::process::exit(1);
            }
            cur = db.get(p).and_then(|x| x.parent);
            hops += 1;
            if hops > 64 {
                break;
            }
        }
    }

    // Store values needed for hierarchy validation
    let (final_parent, final_kind) = {
        let Some(t) = db.get_mut(task_id) else {
            eprintln!("Task {} not found.", task_id);
            std::process::exit(1);
        };
        if let Some(s) = title {
            t.title = s;
        }
        if let Some(d) = desc {
            t.description = if d.is_empty() { None } else { Some(d) };
        }
        if clear_due {
            t.due = None;
        }
        if let Some(ds) = due {
            t.due = parse_due_input(&ds);
            if t.due.is_none() {
                eprintln!(
                    "Unrecognised due date. Use YYYY-MM-DD, 'today', 'tomorrow', or 'in Nd'."
                );
                std::process::exit(1);
            }
        }
        if clear_parent {
            t.parent = None;
        }
        if let Some(pid) = parent_id {
            t.parent = Some(pid);
        }
        if let Some(k) = kind {
            t.kind = k;
        }
        if let Some(s) = status {
            t.status = s;
        }

        (t.parent, t.kind)
    };

    // Validate hierarchy after kind/parent updates
    if let Some(parent_id) = final_parent {
        if let Some(parent_task) = db.get(parent_id) {
            if !validate_hierarchy(parent_task.kind, final_kind) {
                eprintln!("Invalid hierarchy: {} cannot be child of {}. Valid hierarchy: Project > Product > Epic > Task > Subtask",
                    format_kind(final_kind), format_kind(parent_task.kind));
                std::process::exit(1);
            }
        }
    }

    // Get mutable borrow again for tag updates
    let Some(t) = db.get_mut(task_id) else {
        eprintln!("Task {} not found.", task_id);
        std::process::exit(1);
    };
    let mut add = split_and_normalise_tags(&add_tags);
    let rm = split_and_normalise_tags(&rm_tags)
        .into_iter()
        .collect::<HashSet<_>>();
    if !add.is_empty() || !rm.is_empty() {
        // Merge tags.
        let mut set = t.tags.iter().cloned().collect::<BTreeSet<_>>();
        for a in add.drain(..) {
            set.insert(a);
        }
        for r in rm {
            set.remove(&r);
        }
        t.tags = set.into_iter().collect();
    }

    t.updated_at_utc = Utc::now().timestamp();
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
    commit_or_warn(db_path, &commit_subject_for(task_id, "update", None));
    emit_or_warn(db_path, "update", Some(task_id), None);
    println!("Updated task {}", task_id);
}

/// Mark a task as completed, optionally completing all descendants.
pub fn cmd_complete(
    db: &mut Database,
    db_path: &Path,
    id: Option<String>,
    recurse: bool,
    tag: Option<String>,
    project: Option<String>,
    status_filter: Option<Status>,
) {
    // Validate that exactly one option is provided
    let option_count = [
        id.is_some(),
        tag.is_some(),
        project.is_some(),
        status_filter.is_some(),
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    if option_count != 1 {
        eprintln!("Error: Must specify exactly one of --id, --tag, --project, or --status");
        std::process::exit(1);
    }

    let mut to_mark: HashSet<LeafId> = HashSet::new();

    if let Some(id_str) = id {
        // Single task completion
        let task_id = match resolve_task_identifier(&id_str, db) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error resolving task: {}", e);
                std::process::exit(1);
            }
        };

        let Some(_) = db.get(task_id) else {
            eprintln!("Task {} not found.", task_id);
            std::process::exit(1);
        };

        to_mark.insert(task_id);
        if recurse {
            let child_map = build_children_map(&db.tasks);
            collect_descendants(task_id, &child_map, &mut to_mark);
        }
    } else {
        // Bulk completion
        for task in &db.tasks {
            let matches = if let Some(ref tag_filter) = tag {
                task.tags.iter().any(|t| t == tag_filter)
            } else if let Some(ref project_filter) = project {
                project_label(db, task) == *project_filter
            } else if let Some(status_val) = status_filter {
                task.status == status_val
            } else {
                false
            };

            if matches {
                to_mark.insert(task.id);
            }
        }

        if to_mark.is_empty() {
            println!("No tasks found matching the criteria.");
            return;
        }

        // Show what will be completed
        println!("Will complete {} task(s):", to_mark.len());
        for &task_id in &to_mark {
            if let Some(task) = db.get(task_id) {
                println!("  {} - {}", task_id, task.title);
            }
        }
    }
    let completed = to_mark.clone();
    for tid in to_mark {
        if let Some(t) = db.get_mut(tid) {
            t.status = Status::Done;
            t.updated_at_utc = Utc::now().timestamp();
        }
    }
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
    let summary = if completed.len() == 1 {
        let only = *completed.iter().next().expect("len checked");
        commit_subject_for(only, "complete", None)
    } else {
        format!("pm: complete batch ({} tickets)", completed.len())
    };
    commit_or_warn(db_path, &summary);
    // One event per completed ticket so the feed credits each id.
    for tid in &completed {
        emit_or_warn(db_path, "complete", Some(*tid), None);
    }
    println!("Marked done.");
}

/// Reopen a completed task by setting its status to Open.
pub fn cmd_reopen(db: &mut Database, db_path: &Path, id: String) {
    let task_id = match resolve_task_identifier(&id, db) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Error resolving task: {}", e);
            std::process::exit(1);
        }
    };

    let Some(t) = db.get_mut(task_id) else {
        eprintln!("Task {} not found.", task_id);
        std::process::exit(1);
    };
    t.status = Status::Open;
    t.updated_at_utc = Utc::now().timestamp();
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
    commit_or_warn(db_path, &commit_subject_for(task_id, "reopen", None));
    emit_or_warn(db_path, "reopen", Some(task_id), None);
    println!("Reopened {}", task_id);
}

/// Delete a task, optionally cascading to all descendants.
pub fn cmd_delete(
    db: &mut Database,
    db_path: &Path,
    id: Option<String>,
    cascade: bool,
    tag: Option<String>,
    project: Option<String>,
    status_filter: Option<Status>,
) {
    // Validate that exactly one option is provided
    let option_count = [
        id.is_some(),
        tag.is_some(),
        project.is_some(),
        status_filter.is_some(),
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    if option_count != 1 {
        eprintln!("Error: Must specify exactly one of --id, --tag, --project, or --status");
        std::process::exit(1);
    }

    let mut to_delete: HashSet<LeafId> = HashSet::new();

    if let Some(id_str) = id {
        // Single task deletion
        let task_id = match resolve_task_identifier(&id_str, db) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error resolving task: {}", e);
                std::process::exit(1);
            }
        };

        let Some(_) = db.get(task_id) else {
            eprintln!("Task {} not found.", task_id);
            std::process::exit(1);
        };

        let child_map = build_children_map(&db.tasks);
        let mut children: HashSet<LeafId> = HashSet::new();
        collect_descendants(task_id, &child_map, &mut children);
        if !children.is_empty() && !cascade {
            eprintln!(
                "Task {} has {} descendant(s). Use --cascade to delete all.",
                task_id,
                children.len()
            );
            std::process::exit(1);
        }
        to_delete = children;
        to_delete.insert(task_id);
    } else {
        // Bulk deletion
        for task in &db.tasks {
            let matches = if let Some(ref tag_filter) = tag {
                task.tags.iter().any(|t| t == tag_filter)
            } else if let Some(ref project_filter) = project {
                project_label(db, task) == *project_filter
            } else if let Some(status_val) = status_filter {
                task.status == status_val
            } else {
                false
            };

            if matches {
                to_delete.insert(task.id);
            }
        }

        if to_delete.is_empty() {
            println!("No tasks found matching the criteria.");
            return;
        }

        // Show what will be deleted
        println!("Will delete {} task(s):", to_delete.len());
        for &task_id in &to_delete {
            if let Some(task) = db.get(task_id) {
                println!("  {} - {}", task_id, task.title);
            }
        }
    }

    let ids = to_delete;
    let count = ids.len();
    let first = ids.iter().next().copied();
    // Snapshot the ids before they are removed so the feed can credit each.
    let deleted: Vec<crate::store::LeafId> = ids.iter().copied().collect();
    db.remove_ids(&ids);
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
    let summary = match (count, first) {
        (1, Some(id)) => commit_subject_for(id, "delete", None),
        (n, _) => format!("pm: delete batch ({n} tickets)"),
    };
    commit_or_warn(db_path, &summary);
    for id in &deleted {
        emit_or_warn(db_path, "delete", Some(*id), None);
    }
    println!("Deleted.");
}

/// List all distinct project names derived from each task's parent chain.
/// A task without a Project ancestor is bucketed under `-`.
pub fn cmd_projects(db: &Database) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in &db.tasks {
        let key = project_label(db, t);
        *counts.entry(key).or_default() += 1;
    }
    println!("{:<16} {}", "Project", "Count");
    for (p, c) in counts {
        println!("{:<16} {}", truncate(&p, 16), c);
    }
}

/// List all distinct tags with their usage counts.
pub fn cmd_tags(db: &Database) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in &db.tasks {
        for tag in &t.tags {
            *counts.entry(tag.clone()).or_default() += 1;
        }
    }
    println!("{:<16} {}", "Tag", "Count");
    for (tag, c) in counts {
        println!("{:<16} {}", truncate(&tag, 16), c);
    }
}

/// Generate shell completion scripts.
pub fn cmd_completions(shell: Shell) {
    use crate::cli::Cli;
    use clap::CommandFactory;

    let mut app = Cli::command();
    let app_name = app.get_name().to_string();
    generate(shell, &mut app, app_name, &mut std::io::stdout());
}

/// Handle template management commands.
pub fn cmd_template(db: &mut Database, db_path: &Path, action: TemplateAction) {
    match action {
        TemplateAction::Save {
            task_id,
            template_name,
        } => {
            let task_id_resolved = match resolve_task_identifier(&task_id, db) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("Error resolving task: {}", e);
                    std::process::exit(1);
                }
            };

            let Some(task) = db.get(task_id_resolved) else {
                eprintln!("Task {} not found.", task_id_resolved);
                std::process::exit(1);
            };

            // Check if template already exists
            if db.state.templates.iter().any(|t| t.name == template_name) {
                eprintln!(
                    "Template '{}' already exists. Use a different name.",
                    template_name
                );
                std::process::exit(1);
            }

            let template = TaskTemplate {
                name: template_name.clone(),
                title_template: Some(task.title.clone()),
                description_template: task.description.clone(),
                tags: task.tags.clone(),
                kind: task.kind,
                priority_level: task.priority_level,
                urgency: task.urgency,
                process_stage: task.process_stage,
                status: task.status,
            };

            db.state.templates.push(template);

            if let Err(e) = db.save(db_path) {
                eprintln!("Failed to save database: {}", e);
                std::process::exit(1);
            }

            println!(
                "Saved template '{}' from task {}",
                template_name, task_id_resolved
            );
        }

        TemplateAction::List => {
            if db.state.templates.is_empty() {
                println!("No templates found.");
                return;
            }

            println!("{:<20} {:<10} {:<12}", "Name", "Kind", "Status");
            for template in &db.state.templates {
                println!(
                    "{:<20} {:<10} {:<12}",
                    truncate(&template.name, 20),
                    format_kind(template.kind),
                    format_status(template.status),
                );
            }
        }

        TemplateAction::Delete { template_name } => {
            let initial_len = db.state.templates.len();
            db.state.templates.retain(|t| t.name != template_name);

            if db.state.templates.len() == initial_len {
                eprintln!("Template '{}' not found.", template_name);
                std::process::exit(1);
            }

            if let Err(e) = db.save(db_path) {
                eprintln!("Failed to save database: {}", e);
                std::process::exit(1);
            }

            println!("Deleted template '{}'", template_name);
        }

        TemplateAction::Create {
            name,
            title_template,
            description,
            tags,
            kind,
            priority,
            urgency,
            process_stage,
            status,
        } => {
            // Check if template already exists
            if db.state.templates.iter().any(|t| t.name == name) {
                eprintln!("Template '{}' already exists. Use a different name.", name);
                std::process::exit(1);
            }

            let template_tags = if let Some(tags_str) = tags {
                split_and_normalise_tags(&[tags_str])
            } else {
                Vec::new()
            };

            let template = TaskTemplate {
                name: name.clone(),
                title_template,
                description_template: description,
                tags: template_tags,
                kind,
                priority_level: priority,
                urgency,
                process_stage,
                status,
            };

            db.state.templates.push(template);

            if let Err(e) = db.save(db_path) {
                eprintln!("Failed to save database: {}", e);
                std::process::exit(1);
            }

            println!("Created template '{}'", name);
        }
        TemplateAction::Edit { kind } => {
            cmd_template_edit(db_path, &kind);
        }
        TemplateAction::Apply { id } => {
            cmd_template_apply(db, db_path, &id);
        }
    }
}

/// Open a per-kind section template in `$EDITOR`. Resolves through the
/// override chain (`.pm/templates/<kind>.md`, then `~/.pm-templates/<kind>.md`,
/// then the built-in default). If the chosen file does not exist on disk yet,
/// the built-in default is copied into `.pm/templates/<kind>.md` so the user
/// has something to edit.
pub fn cmd_template_edit(pm_dir: &Path, kind: &str) {
    use crate::store::id::TypePrefix;
    use crate::store::templates;

    let prefix = match kind.to_lowercase().as_str() {
        "project" => TypePrefix::Project,
        "product" => TypePrefix::Product,
        "epic" => TypePrefix::Epic,
        "task" => TypePrefix::Task,
        "subtask" => TypePrefix::Subtask,
        "milestone" => TypePrefix::Milestone,
        other => {
            eprintln!("template edit: unknown kind {other:?}; expected project|product|epic|task|subtask|milestone");
            std::process::exit(1);
        }
    };

    let target = pm_dir
        .join("templates")
        .join(format!("{}.md", templates::template_stem(prefix)));
    if !target.exists() {
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&target, templates::builtin(prefix)) {
            eprintln!(
                "template edit: could not seed override at {}: {e}",
                target.display()
            );
            std::process::exit(1);
        }
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
    match std::process::Command::new(&editor).arg(&target).status() {
        Ok(st) if st.success() => println!("Saved template at {}", target.display()),
        Ok(st) => {
            eprintln!("template edit: $EDITOR exited with status {st}");
            std::process::exit(st.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("template edit: could not launch {editor}: {e}");
            std::process::exit(1);
        }
    }
}

/// Re-apply the per-kind section template to an existing ticket. Existing
/// content under sections that match the template by name is preserved; user-
/// added sections are kept at the tail of the body.
pub fn cmd_template_apply(db: &mut Database, pm_dir: &Path, id: &str) {
    use crate::store::templates;

    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("template apply: ticket not found: {id}");
            std::process::exit(1);
        }
    };
    let Some(entry) = db.state.items.get(&leaf) else {
        eprintln!("template apply: state.json has no entry for {id}");
        std::process::exit(1);
    };
    let claude_path = pm_dir
        .join(&entry.path)
        .join(crate::store::claude_md::CLAUDE_MD);
    let mut ticket = match crate::store::Ticket::read(&claude_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "template apply: could not read {}: {e}",
                claude_path.display()
            );
            std::process::exit(1);
        }
    };

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let resolved = templates::resolve(leaf.prefix(), pm_dir, home.as_deref());
    ticket.apply_template(&resolved.content);

    let parent = claude_path.parent().expect("ticket dir");
    if let Err(e) = ticket.write_to(parent) {
        eprintln!("template apply: write failed: {e}");
        std::process::exit(1);
    }
    let stem = templates::template_stem(leaf.prefix());
    commit_or_warn(
        pm_dir,
        &commit_subject_for(leaf, "template apply", Some(stem)),
    );
    emit_or_warn(pm_dir, "template-apply", Some(leaf), Some(stem));
    println!("Applied {stem} template to {leaf}");
}

/// Export tasks to CSV format for external analysis and time tracking.
pub fn cmd_export(
    db: &Database,
    output: Option<String>,
    all: bool,
    project: Option<String>,
    tag: Option<String>,
) {
    let output_path = output.unwrap_or_else(|| "tasks.csv".to_string());

    // Filter tasks
    let tasks: Vec<&Task> = db
        .tasks
        .iter()
        .filter(|task| {
            // Include completed tasks only if --all is specified
            if !all && task.status == Status::Done {
                return false;
            }

            // Project filter derives the project label from the parent chain.
            if let Some(ref proj_filter) = project {
                if project_label(db, task) != *proj_filter {
                    return false;
                }
            }

            // Tag filter
            if let Some(ref tag_filter) = tag {
                if !task.tags.iter().any(|t| t == tag_filter) {
                    return false;
                }
            }

            true
        })
        .collect();

    // Create CSV content
    let mut csv_content = String::new();

    // CSV Header
    csv_content.push_str("ID,Title,Kind,Status,Priority,Urgency,ProcessStage,Project,Tags,Due,Parent,CreatedUTC,UpdatedUTC,Description\n");

    // CSV Rows
    let task_count = tasks.len();
    for task in &tasks {
        let priority = task
            .priority_level
            .map(|p| format_priority(Some(p)))
            .unwrap_or("-");
        let urgency = task.urgency.map(|u| format_urgency(Some(u))).unwrap_or("-");
        let process_stage = task
            .process_stage
            .map(|ps| format_process_stage(Some(ps)))
            .unwrap_or("-");
        let project_col = project_label(db, task);
        let tags = if task.tags.is_empty() {
            "-".to_string()
        } else {
            task.tags.join(";")
        };
        let due = task.due.map(|d| d.to_string()).unwrap_or("-".to_string());
        let parent = task
            .parent
            .map(|p| p.to_string())
            .unwrap_or("-".to_string());
        let created = chrono::Utc
            .timestamp_opt(task.created_at_utc, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let updated = chrono::Utc
            .timestamp_opt(task.updated_at_utc, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let description = task.description.as_deref().unwrap_or("-");

        // Escape CSV fields that contain commas or quotes
        let escape_csv = |s: &str| {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_string()
            }
        };

        csv_content.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            task.id,
            escape_csv(&task.title),
            format_kind(task.kind),
            format_status(task.status),
            escape_csv(&priority),
            escape_csv(&urgency),
            escape_csv(&process_stage),
            escape_csv(&project_col),
            escape_csv(&tags),
            escape_csv(&due),
            escape_csv(&parent),
            escape_csv(&created),
            escape_csv(&updated),
            escape_csv(description)
        ));
    }

    // Write to file
    match std::fs::write(&output_path, csv_content) {
        Ok(_) => {
            println!("Exported {} task(s) to {}", task_count, output_path);
        }
        Err(e) => {
            eprintln!("Failed to write CSV file: {}", e);
            std::process::exit(1);
        }
    }
}

/// Create a timestamped backup of the database file.
pub fn create_backup(db_path: &Path) -> Result<String, std::io::Error> {
    if !db_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Database file does not exist",
        ));
    }

    let parent_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let backup_dir = parent_dir.join("backup");

    // Create backup directory if it doesn't exist
    fs::create_dir_all(&backup_dir)?;

    // Generate timestamp for backup filename
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let db_filename = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tasks.json");

    let backup_filename = format!("{}_{}", timestamp, db_filename);
    let backup_path = backup_dir.join(backup_filename);

    // Copy the database file to backup location
    fs::copy(db_path, &backup_path)?;

    Ok(backup_path.to_string_lossy().to_string())
}

/// Import tasks from CSV format with automatic backup.
pub fn cmd_import(db: &mut Database, db_path: &Path, input: String, no_backup: bool) {
    // Create backup unless explicitly disabled
    if !no_backup {
        match create_backup(db_path) {
            Ok(backup_path) => {
                println!("Created backup: {}", backup_path);
            }
            Err(e) => {
                eprintln!("Warning: Failed to create backup: {}", e);
                print!("Continue without backup? (y/N): ");
                use std::io::{self, Write};
                io::stdout().flush().unwrap();

                let mut response = String::new();
                if io::stdin().read_line(&mut response).is_err()
                    || !response.trim().to_lowercase().starts_with('y')
                {
                    println!("Import cancelled.");
                    return;
                }
            }
        }
    }

    // Read CSV file
    let csv_content = match fs::read_to_string(&input) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Failed to read CSV file '{}': {}", input, e);
            std::process::exit(1);
        }
    };

    let lines: Vec<&str> = csv_content.lines().collect();
    if lines.is_empty() {
        eprintln!("CSV file is empty");
        std::process::exit(1);
    }

    // Parse header to validate format
    let expected_header = "ID,Title,Kind,Status,Priority,Urgency,ProcessStage,Project,Tags,Due,Parent,CreatedUTC,UpdatedUTC,Description";
    if lines[0] != expected_header {
        eprintln!(
            "Invalid CSV header. Expected:\n{}\nGot:\n{}",
            expected_header, lines[0]
        );
        std::process::exit(1);
    }

    let mut imported_count = 0;
    let mut skipped_count = 0;

    // Process each CSV row (skip header)
    for (line_num, line) in lines.iter().skip(1).enumerate() {
        let line_num = line_num + 2; // +2 because we skip header and line numbers are 1-based

        // Simple CSV parsing (handles quoted fields)
        let fields = parse_csv_line(line);
        if fields.len() != 14 {
            eprintln!(
                "Warning: Line {} has {} fields, expected 14. Skipping.",
                line_num,
                fields.len()
            );
            skipped_count += 1;
            continue;
        }

        // Parse fields. The legacy ID column is ignored; the new id is
        // allocated through `db.allocate_id` so the v2 counters stay
        // authoritative. The Project column (fields[7]) is read but not stored
        // since Task.project has been dropped; project membership derives from
        // the parent chain.
        let title = fields[1].clone();
        let kind = parse_kind(&fields[2]);
        let status = parse_status(&fields[3]);
        let priority = parse_priority(&fields[4]);
        let urgency = parse_urgency(&fields[5]);
        let process_stage = parse_process_stage(&fields[6]);
        let tags = if fields[8] == "-" {
            Vec::new()
        } else {
            fields[8].split(';').map(|s| s.to_string()).collect()
        };
        let due = if fields[9] == "-" {
            None
        } else {
            NaiveDate::parse_from_str(&fields[9], "%Y-%m-%d").ok()
        };
        let parent = if fields[10] == "-" {
            None
        } else {
            fields[10].parse::<IdInput>().ok().map(|input| input.leaf())
        };
        let description = if fields[13] == "-" {
            None
        } else {
            Some(fields[13].clone())
        };

        if title.is_empty() {
            eprintln!("Warning: Line {} has empty title. Skipping.", line_num);
            skipped_count += 1;
            continue;
        }

        // Check if task with same title already exists
        if db.tasks.iter().any(|t| t.title == title) {
            eprintln!(
                "Warning: Task with title '{}' already exists. Skipping.",
                title
            );
            skipped_count += 1;
            continue;
        }

        let new_task = Task {
            id: db.allocate_id(kind_to_prefix(kind)),
            title,
            summary: None, // CSV doesn't include summary field
            description,
            user_story: None,   // CSV doesn't include user_story field
            requirements: None, // CSV doesn't include requirements field
            tags,
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
            due,
            parent,
            kind,
            status,
            priority_level: priority,
            urgency,
            process_stage,
            issue_link: None,      // CSV doesn't include issue_link field
            pr_link: None,         // CSV doesn't include pr_link field
            artifacts: Vec::new(), // CSV doesn't include artifacts field
            created_at_utc: Utc::now().timestamp(),
            updated_at_utc: Utc::now().timestamp(),
        };

        db.tasks.push(new_task);
        imported_count += 1;
    }

    // Save database
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save database: {}", e);
        std::process::exit(1);
    }

    println!(
        "Import completed. {} tasks imported, {} skipped.",
        imported_count, skipped_count
    );
}

/// Simple CSV line parser that handles quoted fields.
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    // Escaped quote
                    current_field.push('"');
                    chars.next(); // consume the second quote
                } else {
                    // Toggle quote state
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                // Field separator
                fields.push(current_field);
                current_field = String::new();
            }
            _ => {
                current_field.push(ch);
            }
        }
    }

    // Don't forget the last field
    fields.push(current_field);
    fields
}

/// Create a backup command implementation.
pub fn cmd_backup(db_path: &Path, all: bool) {
    if all {
        let pm_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
        cmd_backup_all(pm_dir);
        return;
    }

    match create_backup(db_path) {
        Ok(backup_path) => {
            println!("Backup created: {}", backup_path);
        }
        Err(e) => {
            eprintln!("Failed to create backup: {}", e);
            std::process::exit(1);
        }
    }
}

/// Backup all projects in the PM directory.
pub fn cmd_backup_all(pm_dir: &Path) {
    use crate::project::{discover_projects, get_legacy_project};

    let mut projects = discover_projects(pm_dir).unwrap_or_else(|e| {
        eprintln!("Failed to discover projects: {}", e);
        std::process::exit(1);
    });

    // Add legacy project if it exists
    if let Some(legacy) = get_legacy_project(pm_dir) {
        projects.push(legacy);
    }

    if projects.is_empty() {
        println!("No projects found to backup.");
        return;
    }

    let mut success_count = 0;
    let total_count = projects.len();

    for project in &projects {
        match create_backup(&project.file_path) {
            Ok(backup_path) => {
                println!("Backed up {}: {}", project.display_name, backup_path);
                success_count += 1;
            }
            Err(e) => {
                eprintln!("Failed to backup {}: {}", project.display_name, e);
            }
        }
    }

    println!(
        "Backup completed: {}/{} projects backed up successfully.",
        success_count, total_count
    );
}

/// Export all projects to CSV format.
pub fn cmd_export_all(
    pm_dir: &Path,
    output: Option<String>,
    include_completed: bool,
    project_filter: Option<String>,
    tag_filter: Option<String>,
) {
    use crate::project::{discover_projects, get_legacy_project};

    let mut projects = discover_projects(pm_dir).unwrap_or_else(|e| {
        eprintln!("Failed to discover projects: {}", e);
        std::process::exit(1);
    });

    // Add legacy project if it exists
    if let Some(legacy) = get_legacy_project(pm_dir) {
        projects.push(legacy);
    }

    if projects.is_empty() {
        println!("No projects found to export.");
        return;
    }

    // Apply project filter if specified
    if let Some(ref proj_filter) = project_filter {
        projects.retain(|p| p.name == *proj_filter || p.display_name == *proj_filter);
        if projects.is_empty() {
            eprintln!("No projects found matching filter: {}", proj_filter);
            std::process::exit(1);
        }
    }

    let output_path = output.unwrap_or_else(|| "all_projects.csv".to_string());
    // Collect (project, task, derived_project_label) so the CSV row writer
    // does not need the db handle later.
    let mut all_rows: Vec<(crate::project::Project, Task, String)> = Vec::new();

    // Collect tasks from all projects
    for project in &projects {
        let db = project.load_database();
        for task in &db.tasks {
            // Apply filters
            if !include_completed && task.status == Status::Done {
                continue;
            }

            if let Some(ref tag_filter) = tag_filter {
                if !task.tags.iter().any(|t| t == tag_filter) {
                    continue;
                }
            }

            let project_col = project_label(&db, task);
            all_rows.push((project.clone(), task.clone(), project_col));
        }
    }

    // Create CSV content
    let mut csv_content = String::new();

    // CSV Header (add project name column)
    csv_content.push_str("ProjectName,ID,Title,Kind,Status,Priority,Urgency,ProcessStage,Project,Tags,Due,Parent,CreatedUTC,UpdatedUTC,Description\n");

    // CSV Rows
    let task_count = all_rows.len();
    for (project, task, project_col) in &all_rows {
        let priority = task
            .priority_level
            .map(|p| format_priority(Some(p)))
            .unwrap_or("-");
        let urgency = task.urgency.map(|u| format_urgency(Some(u))).unwrap_or("-");
        let process_stage = task
            .process_stage
            .map(|ps| format_process_stage(Some(ps)))
            .unwrap_or("-");
        let tags = if task.tags.is_empty() {
            "-".to_string()
        } else {
            task.tags.join(";")
        };
        let due = task.due.map(|d| d.to_string()).unwrap_or("-".to_string());
        let parent = task
            .parent
            .map(|p| p.to_string())
            .unwrap_or("-".to_string());
        let created = chrono::Utc
            .timestamp_opt(task.created_at_utc, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let updated = chrono::Utc
            .timestamp_opt(task.updated_at_utc, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let description = task.description.as_deref().unwrap_or("-");

        // Escape CSV fields that contain commas or quotes
        let escape_csv = |s: &str| {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_string()
            }
        };

        csv_content.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            escape_csv(&project.display_name),
            task.id,
            escape_csv(&task.title),
            format_kind(task.kind),
            format_status(task.status),
            escape_csv(&priority),
            escape_csv(&urgency),
            escape_csv(&process_stage),
            escape_csv(project_col),
            escape_csv(&tags),
            escape_csv(&due),
            escape_csv(&parent),
            escape_csv(&created),
            escape_csv(&updated),
            escape_csv(description)
        ));
    }

    // Write to file
    match std::fs::write(&output_path, csv_content) {
        Ok(_) => {
            println!(
                "Exported {} task(s) from {} project(s) to {}",
                task_count,
                projects.len(),
                output_path
            );
        }
        Err(e) => {
            eprintln!("Failed to write CSV file: {}", e);
            std::process::exit(1);
        }
    }
}

/// Launch the workflow project selection menu.
pub fn cmd_workflow_menu(pm_dir: &Path) {
    use crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io;

    // Setup terminal
    enable_raw_mode().unwrap();
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).unwrap();

    // Create and run menu app starting in workflow selection
    let mut app = MenuApp::new(pm_dir.to_path_buf()).unwrap();
    app.start_workflow_selection();
    let res = app.run(&mut terminal);

    // Restore terminal
    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();

    if let Err(err) = res {
        println!("{:?}", err);
        std::process::exit(1);
    }

    // Check what was selected
    if let Some(project) = app.get_selected_project() {
        if app.should_open_workflow() {
            println!("Opening workflow for: {}", project.display_name);
            cmd_wf(&project.file_path);
        }
    }
}

/// Launch the project selection menu.
pub fn cmd_menu(pm_dir: &Path) {
    use crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use std::io;

    // Setup terminal
    enable_raw_mode().unwrap();
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).unwrap();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).unwrap();

    // Create and run menu app
    let mut app = MenuApp::new(pm_dir.to_path_buf()).unwrap();
    let res = app.run(&mut terminal);

    // Restore terminal
    disable_raw_mode().unwrap();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).unwrap();
    terminal.show_cursor().unwrap();

    if let Err(err) = res {
        println!("{:?}", err);
        std::process::exit(1);
    }

    // Check what was selected
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

/// Launch the workflow kanban board interface.
pub fn cmd_wf(db_path: &Path) {
    loop {
        match run_workflow_tui(db_path) {
            Ok(WorkflowExit::EditTask(task_id)) => {
                // User wants to edit a task
                let db = Database::load(db_path);
                if let Some(_task) = db.get(task_id) {
                    // Run the TUI with the task pre-selected for editing
                    if let Err(err) = run_tui_with_edit(db_path, task_id) {
                        eprintln!("Error running TUI: {}", err);
                        std::process::exit(1);
                    }
                    // After editing, return to workflow
                    continue;
                }
            }
            Ok(WorkflowExit::Quit) => {
                // Normal exit
                break;
            }
            Err(err) => {
                eprintln!("Error running workflow TUI: {}", err);
                std::process::exit(1);
            }
        }
    }
}

// =============================================================================
// v2 verb handlers - skeletons populated by Phase 4 tasks 39-42
// =============================================================================

/// `pm init`: scaffold a `.pm/` workspace in the current directory and ensure
/// a git repository encloses it. If `pm_dir` is already inside an existing
/// repo, that repo is reused; otherwise a fresh repo is initialised at
/// `pm_dir`. An initial commit (`pm: init`) records the scaffold so
/// subsequent state-mutating commits have a parent.
pub fn cmd_init(pm_dir: &Path) {
    use crate::store::layout::Layout;
    let layout = Layout::at(pm_dir);
    if let Err(e) = layout.init() {
        eprintln!("init failed: {e}");
        std::process::exit(1);
    }

    if let Err(e) = crate::store::git::ensure_repo(pm_dir) {
        eprintln!("init: git repository setup failed: {e}");
        std::process::exit(1);
    }
    commit_or_warn(pm_dir, "pm: init");
    emit_or_warn(pm_dir, "init", None, None);

    println!("Initialised .pm/ workspace at {}", pm_dir.display());
}

/// Commit any staged workspace changes under `pm_dir` with `message`. Logs a
/// warning on failure rather than aborting, since the on-disk state is
/// already saved and a failed commit should not propagate as a CLI error.
fn commit_or_warn(pm_dir: &Path, message: &str) {
    if let Err(e) = crate::store::git::commit_workspace(pm_dir, message) {
        eprintln!("warning: git commit failed: {e}");
    }
}

/// Build a commit subject for a ticket mutation. Wraps
/// [`crate::store::git::subject`] so handlers only need the verb and optional
/// summary.
fn commit_subject_for(leaf: crate::store::LeafId, verb: &str, summary: Option<&str>) -> String {
    crate::store::git::subject(&leaf, verb, summary)
}

/// `pm show <id>`: print front-matter and section names for a ticket.
pub fn cmd_show(db: &Database, id: &str) {
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("ticket not found: {id}");
            std::process::exit(1);
        }
    };
    let Some(task) = db.get(leaf) else {
        eprintln!("ticket not found: {id}");
        std::process::exit(1);
    };
    println!("{} {}", task.id, task.title);
    println!("  kind: {:?}", task.kind);
    println!("  status: {:?}", task.status);
    if let Some(p) = task.parent {
        println!("  parent: {p}");
    }
    if !task.tags.is_empty() {
        println!("  tags: {}", task.tags.join(", "));
    }
    if let Some(d) = task.due {
        println!("  due: {d}");
    }
    if let Some(ref text) = task.description {
        println!("\n# Description\n{text}");
    }
}

/// Resolve a user-supplied id string against the loaded Database. Accepts the
/// v2 forms understood by [`crate::store::id::IdInput`] (leaf, address, with
/// trailing labels). Returns the canonical `LeafId` if it appears in the db.
fn resolve_v2_id(input: &str, db: &Database) -> Option<crate::store::LeafId> {
    use crate::store::id::IdInput;
    let parsed: IdInput = input.parse().ok()?;
    let leaf = parsed.leaf();
    if db.get(leaf).is_some() {
        Some(leaf)
    } else {
        None
    }
}

/// `pm move <id> [<new_parent>] [--orphan]`: re-parent a ticket. Either a
/// `<new_parent>` id or `--orphan` must be supplied. The new parent's `Kind`
/// must accept this ticket's `Kind` per [`validate_hierarchy`]. After the
/// in-memory update the database is saved, which writes the ticket's
/// `CLAUDE.md` to its new addressed path and rewrites `state.json`.
pub fn cmd_move(
    db: &mut Database,
    pm_dir: &Path,
    id: &str,
    new_parent: Option<&str>,
    orphan: bool,
) {
    use crate::store::id::AddressId;
    use crate::store::layout::Layout;

    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("move: ticket not found: {id}");
            std::process::exit(1);
        }
    };

    if orphan && new_parent.is_some() {
        eprintln!("move: pass either `<new_parent>` or `--orphan`, not both.");
        std::process::exit(1);
    }
    if !orphan && new_parent.is_none() {
        eprintln!("move: supply a new parent id or `--orphan`.");
        std::process::exit(1);
    }

    let target_parent: Option<crate::store::LeafId> = if orphan {
        None
    } else {
        let raw = new_parent.expect("checked above");
        match resolve_v2_id(raw, db) {
            Some(p) if p != leaf => Some(p),
            Some(_) => {
                eprintln!("move: parent cannot equal the ticket itself.");
                std::process::exit(1);
            }
            None => {
                eprintln!("move: parent not found: {raw}");
                std::process::exit(1);
            }
        }
    };

    let task_kind = db.get(leaf).expect("resolved above").kind;
    if let Some(parent_id) = target_parent {
        let parent_kind = db.get(parent_id).expect("resolved above").kind;
        if !validate_hierarchy(parent_kind, task_kind) {
            eprintln!(
                "move: invalid hierarchy: {} cannot be child of {}. \
                 Valid order is Project > Product > Epic > Task > Subtask.",
                format_kind(task_kind),
                format_kind(parent_kind),
            );
            std::process::exit(1);
        }
    }

    // Remember the prior absolute directory so it can be cleaned up after the
    // save writes the new location.
    let old_abs_dir = db
        .state
        .items
        .get(&leaf)
        .map(|entry| pm_dir.join(&entry.path));

    // Capture old address chain for alias bookkeeping.
    let old_address = old_address_for(db, leaf);

    // Apply the move in memory.
    if let Some(task) = db.get_mut(leaf) {
        task.parent = target_parent;
        task.updated_at_utc = Utc::now().timestamp();
    }

    if let Err(e) = db.save(pm_dir) {
        eprintln!("move: save failed: {e}");
        std::process::exit(1);
    }

    // Clean up the now-vacated directory if it differs from where the save
    // landed. Saved state.items has the new path; compare against the old.
    let new_abs_dir = db.state.items.get(&leaf).map(|e| pm_dir.join(&e.path));
    if let (Some(old), Some(new)) = (old_abs_dir.as_ref(), new_abs_dir.as_ref()) {
        if old != new && old.exists() {
            if let Err(e) = fs::remove_dir_all(old) {
                eprintln!(
                    "move: warning - could not remove old directory {}: {e}",
                    old.display()
                );
            }
        }
    }

    // Record an alias so the old address-form keeps resolving.
    if let Some(old) = old_address {
        if let Some(new) = old_address_for(db, leaf) {
            if old != new {
                let layout = Layout::at(pm_dir);
                let aliases_path = layout.aliases_path();
                let mut aliases = crate::store::Aliases::load(&aliases_path).unwrap_or_default();
                aliases.add(old.to_string(), new.to_string());
                if let Err(e) = aliases.save(&aliases_path) {
                    eprintln!("move: warning - could not write alias: {e}");
                }
            }
        }
    }

    let dest_label = target_parent
        .map(|p| p.to_string())
        .unwrap_or_else(|| "(orphan)".into());
    commit_or_warn(
        pm_dir,
        &commit_subject_for(leaf, "move", Some(&format!("-> {dest_label}"))),
    );
    emit_or_warn(
        pm_dir,
        "move",
        Some(leaf),
        Some(&format!("-> {dest_label}")),
    );
    println!("Moved {leaf} -> {dest_label}");

    // Suppress unused-import warning on `AddressId` if no other site brings it.
    let _ = std::marker::PhantomData::<AddressId>;
}

/// Compute the current address chain (parent->child) for a leaf, if every
/// ancestor in the chain is present in the database.
fn old_address_for(db: &Database, leaf: crate::store::LeafId) -> Option<crate::store::AddressId> {
    let mut chain = Vec::new();
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(id) = cursor {
        if guard > 16 {
            return None;
        }
        guard += 1;
        let task = db.get(id)?;
        chain.push(task.id);
        cursor = task.parent;
    }
    chain.reverse();
    crate::store::AddressId::new(chain).ok()
}

/// `pm edit <id> [--section <name>]`: open the ticket's CLAUDE.md in `$EDITOR`.
/// When `section` is supplied, supported editors (nvim, vim, nano, helix,
/// emacs) position the cursor at the matching `# Section` heading. Unknown
/// editors get the file opened at the top.
pub fn cmd_edit(pm_dir: &Path, id: &str, section: Option<&str>) {
    let layout = crate::store::layout::Layout::at(pm_dir);
    let state = crate::store::state::State::load(&layout.state_path()).unwrap_or_default();
    let leaf = match id.parse::<crate::store::IdInput>() {
        Ok(input) => input.leaf(),
        Err(e) => {
            eprintln!("edit: bad id {id}: {e}");
            std::process::exit(1);
        }
    };
    let Some(entry) = state.items.get(&leaf) else {
        eprintln!("edit: ticket not found in state.json: {id}");
        std::process::exit(1);
    };
    let claude_path = pm_dir
        .join(&entry.path)
        .join(crate::store::claude_md::CLAUDE_MD);
    if !claude_path.exists() {
        eprintln!("edit: missing CLAUDE.md at {}", claude_path.display());
        std::process::exit(1);
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
    let mut cmd = std::process::Command::new(&editor);
    if let Some(name) = section {
        let bin = std::path::Path::new(&editor)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        match bin {
            "nvim" | "vim" | "vi" => {
                cmd.arg(format!("+/^# {name}"));
            }
            "nano" | "emacs" | "helix" | "hx" => {
                if let Some(line) = find_section_line(&claude_path, name) {
                    cmd.arg(format!("+{line}"));
                }
            }
            _ => {}
        }
    }
    cmd.arg(&claude_path);
    match cmd.status() {
        Ok(st) if st.success() => {}
        Ok(st) => {
            eprintln!("edit: $EDITOR exited with status {st}");
            std::process::exit(st.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("edit: failed to launch {editor}: {e}");
            std::process::exit(1);
        }
    }
}

/// Locate the 1-indexed line number of a `# Section` heading in a markdown
/// file. Returns `None` if the file cannot be read or the heading is absent.
fn find_section_line(path: &Path, section: &str) -> Option<usize> {
    let content = fs::read_to_string(path).ok()?;
    let needle = format!("# {section}");
    for (i, line) in content.lines().enumerate() {
        if line.trim() == needle {
            return Some(i + 1);
        }
    }
    None
}

/// `pm context <id>`: print the composed CLAUDE.md chain from the root
/// ancestor down to the target ticket. Section bodies are printed verbatim;
/// the trailing `@artifacts/ARTIFACTS.md` line is suppressed because the
/// composed view is for reading rather than re-import.
///
/// When `include_memories` is true (the default; `--no-memories` opts out)
/// the output appends a `## Linked memories (<LEAF>)` section listing every
/// `MemoryRef` from the target ticket's front-matter. Each memory is shown
/// with its tier tag, description, body, and an `@`-import line pointing at
/// the file's absolute path so Claude Code's loader can pull it in.
pub fn cmd_context(db: &Database, pm_dir: &Path, id: &str, include_memories: bool) {
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("context: ticket not found: {id}");
            std::process::exit(1);
        }
    };

    // Walk parent chain, root first.
    let mut chain: Vec<crate::store::LeafId> = Vec::new();
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(cid) = cursor {
        if guard > 16 {
            break;
        }
        guard += 1;
        let Some(task) = db.get(cid) else {
            break;
        };
        chain.push(task.id);
        cursor = task.parent;
    }
    chain.reverse();

    // Keep the leaf ticket's front-matter around so we can render its
    // `memories:` list after the chain has finished printing.
    let mut leaf_ticket: Option<crate::store::Ticket> = None;

    for id in chain {
        let Some(entry) = db.state.items.get(&id) else {
            continue;
        };
        let claude_path = pm_dir
            .join(&entry.path)
            .join(crate::store::claude_md::CLAUDE_MD);
        let Ok(ticket) = crate::store::Ticket::read(&claude_path) else {
            continue;
        };
        let task = db.get(id).expect("resolved above");
        println!(
            "# {} - {} ({})",
            format_kind(task.kind).to_uppercase(),
            task.title,
            id
        );
        for section in &ticket.body.sections {
            println!();
            println!("# {}", section.name);
            print!("{}", section.body);
        }
        println!();
        println!("---");
        println!();
        if id == leaf {
            leaf_ticket = Some(ticket);
        }
    }

    if include_memories {
        if let Some(ticket) = leaf_ticket.as_ref() {
            if !ticket.front_matter.memories.is_empty() {
                render_linked_memories_section(db, pm_dir, leaf, ticket);
            }
        }
    }
}

/// Append the "Linked memories" composed-view section. Each entry resolves
/// to a file path through the [`MemoryContext`] for the ticket; the path is
/// emitted as an absolute `@`-import line so Claude Code's loader can pull
/// in the content even when the consumer is not cwd-rooted under the
/// workspace.
fn render_linked_memories_section(
    db: &Database,
    pm_dir: &Path,
    leaf: LeafId,
    ticket: &crate::store::Ticket,
) {
    let entry = match db.state.items.get(&leaf) {
        Some(e) => e,
        None => return,
    };
    let ticket_dir = pm_dir.join(&entry.path);

    let ctx = MemoryContext {
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        pm_root: pm_dir.to_path_buf(),
        active_project: project_ancestor(db, leaf),
        active_ticket_dir: Some(ticket_dir),
    };

    println!("## Linked memories ({leaf})");
    println!();

    for memref in &ticket.front_matter.memories {
        let name = memref_name(memref);
        let (scope, location) = match memref {
            MemoryRef::User(_) => (Scope::User, ctx.user_path(name)),
            MemoryRef::Project(_) => match ctx.project_path(name) {
                Ok(loc) => (Scope::Project, loc),
                Err(e) => {
                    println!("### {name} [project]");
                    println!();
                    println!("(unresolved: {e})");
                    println!();
                    println!("---");
                    println!();
                    continue;
                }
            },
            MemoryRef::Ticket(_) => match ctx.ticket_path(name) {
                Ok(loc) => (Scope::Ticket, loc),
                Err(e) => {
                    println!("### {name} [ticket]");
                    println!();
                    println!("(unresolved: {e})");
                    println!();
                    println!("---");
                    println!();
                    continue;
                }
            },
        };

        println!("### {name} [{}]", scope.as_str());

        match MemoryFile::read(&location.file) {
            Ok(mf) => {
                if let Some(desc) = &mf.front_matter.description {
                    println!();
                    println!("> {desc}");
                }
                println!();
                print!("{}", mf.body);
                if !mf.body.ends_with('\n') {
                    println!();
                }
            }
            Err(e) => {
                println!();
                println!("(missing on disk: {e})");
            }
        }

        println!();
        println!("@{}", location.file.display());
        println!();
        println!("---");
        println!();
    }
}

/// `pm materialise <id> [--output <path>]`: write the composed CLAUDE.md
/// chain to disk so non-Claude tools can read a single self-contained file.
pub fn cmd_materialise(db: &Database, pm_dir: &Path, id: &str, output: Option<PathBuf>) {
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("materialise: ticket not found: {id}");
            std::process::exit(1);
        }
    };
    let Some(entry) = db.state.items.get(&leaf) else {
        eprintln!("materialise: state.json has no entry for {id}");
        std::process::exit(1);
    };

    // Capture stdout via a temp buffer by re-using cmd_context's output, then
    // write to the target file.
    let mut buf = Vec::new();
    {
        use std::io::Write;
        let mut cursor = Some(leaf);
        let mut chain: Vec<crate::store::LeafId> = Vec::new();
        let mut guard = 0;
        while let Some(cid) = cursor {
            if guard > 16 {
                break;
            }
            guard += 1;
            let Some(task) = db.get(cid) else {
                break;
            };
            chain.push(task.id);
            cursor = task.parent;
        }
        chain.reverse();

        for id in chain {
            let Some(entry) = db.state.items.get(&id) else {
                continue;
            };
            let claude_path = pm_dir
                .join(&entry.path)
                .join(crate::store::claude_md::CLAUDE_MD);
            let Ok(ticket) = crate::store::Ticket::read(&claude_path) else {
                continue;
            };
            let task = db.get(id).expect("resolved above");
            writeln!(
                buf,
                "# {} - {} ({})",
                format_kind(task.kind).to_uppercase(),
                task.title,
                id
            )
            .ok();
            for section in &ticket.body.sections {
                writeln!(buf).ok();
                writeln!(buf, "# {}", section.name).ok();
                buf.extend_from_slice(section.body.as_bytes());
            }
            writeln!(buf).ok();
            writeln!(buf, "---").ok();
            writeln!(buf).ok();
        }
    }

    let target = output.unwrap_or_else(|| pm_dir.join(&entry.path).join("COMPOSED.md"));
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&target, &buf) {
        eprintln!("materialise: write failed: {e}");
        std::process::exit(1);
    }
    println!("Wrote composed view to {}", target.display());
}

/// `pm artifact ...`: thin wrapper over `store::artifacts`.
pub fn cmd_artifact(db: &Database, pm_dir: &Path, action: ArtifactAction) {
    let resolve = |id: &str| -> (crate::store::LeafId, PathBuf) {
        let leaf = match resolve_v2_id(id, db) {
            Some(l) => l,
            None => {
                eprintln!("artifact: ticket not found: {id}");
                std::process::exit(1);
            }
        };
        let Some(entry) = db.state.items.get(&leaf) else {
            eprintln!("artifact: state.json has no entry for {id}");
            std::process::exit(1);
        };
        let dir = pm_dir.join(&entry.path).join("artifacts");
        let _ = fs::create_dir_all(&dir);
        (leaf, dir)
    };

    match action {
        ArtifactAction::Add { id, path, desc } => {
            let (leaf, artifacts_dir) = resolve(&id);
            let file_name = match path.file_name() {
                Some(n) => n.to_owned(),
                None => {
                    eprintln!("artifact add: source has no file name: {}", path.display());
                    std::process::exit(1);
                }
            };
            let target = artifacts_dir.join(&file_name);
            if let Err(e) = fs::copy(&path, &target) {
                eprintln!("artifact add: copy failed: {e}");
                std::process::exit(1);
            }
            // Sweep so the new file is in ARTIFACTS.md.
            if let Err(e) = crate::store::artifacts::sweep_dir(&artifacts_dir, leaf) {
                eprintln!("artifact add: sweep failed: {e}");
                std::process::exit(1);
            }
            if let Some(desc_text) = desc {
                let index_path = artifacts_dir.join(crate::store::artifacts::ARTIFACTS_MD);
                if let Ok(mut idx) = crate::store::ArtifactsIndex::load(&index_path) {
                    if let Some(entry) = idx.find_mut(&file_name.to_string_lossy()) {
                        entry.desc = desc_text;
                    }
                    let _ = idx.save(&index_path);
                }
            }
            let name = file_name.to_string_lossy().into_owned();
            commit_or_warn(
                pm_dir,
                &commit_subject_for(leaf, "artifact add", Some(&name)),
            );
            emit_or_warn(pm_dir, "artifact-add", Some(leaf), Some(&name));
            println!("Added artifact {name} to {leaf}");
        }
        ArtifactAction::Rename { id, old, new } => {
            let (leaf, artifacts_dir) = resolve(&id);
            if let Err(e) = crate::store::artifacts::rename_artifact(&artifacts_dir, &old, &new) {
                eprintln!("artifact rename: {e}");
                std::process::exit(1);
            }
            let detail = format!("{old} -> {new}");
            commit_or_warn(
                pm_dir,
                &commit_subject_for(leaf, "artifact rename", Some(&detail)),
            );
            emit_or_warn(pm_dir, "artifact-rename", Some(leaf), Some(&detail));
            println!("Renamed {old} -> {new} on {leaf}");
        }
        ArtifactAction::List { id } => {
            let (_leaf, artifacts_dir) = resolve(&id);
            let index_path = artifacts_dir.join(crate::store::artifacts::ARTIFACTS_MD);
            match crate::store::ArtifactsIndex::load(&index_path) {
                Ok(idx) => {
                    if idx.entries.is_empty() {
                        println!("(no artifacts)");
                    } else {
                        for entry in idx.entries {
                            let desc = if entry.desc.is_empty() {
                                "-"
                            } else {
                                entry.desc.as_str()
                            };
                            println!("  {}  ({})  [{}]", entry.file, desc, entry.tags.join(","));
                        }
                    }
                }
                Err(_) => println!("(no artifacts)"),
            }
        }
    }
}

/// Mutate a ticket's front-matter in memory, persist via Database::save, and
/// record a structured commit. The `label` doubles as both the user-facing
/// status message and the verb in the commit subject.
fn mutate_task<F>(db: &mut Database, pm_dir: &Path, id: &str, label: &str, f: F)
where
    F: FnOnce(&mut Task),
{
    mutate_task_with_summary(db, pm_dir, id, label, None, f)
}

fn mutate_task_with_summary<F>(
    db: &mut Database,
    pm_dir: &Path,
    id: &str,
    label: &str,
    summary: Option<&str>,
    f: F,
) where
    F: FnOnce(&mut Task),
{
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("{label}: ticket not found: {id}");
            std::process::exit(1);
        }
    };
    if let Some(task) = db.get_mut(leaf) {
        f(task);
        task.updated_at_utc = Utc::now().timestamp();
    }
    if let Err(e) = db.save(pm_dir) {
        eprintln!("{label}: save failed: {e}");
        std::process::exit(1);
    }
    commit_or_warn(pm_dir, &commit_subject_for(leaf, label, summary));
    emit_or_warn(pm_dir, label, Some(leaf), summary);
    println!("{label}: {leaf} updated.");
}

/// `pm set-status <id> <new-status>`: update front-matter status.
pub fn cmd_set_status(db: &mut Database, pm_dir: &Path, id: &str, new_status: Status) {
    mutate_task(db, pm_dir, id, "status", |task| task.status = new_status);
}

/// `pm priority <id> <priority>`: set front-matter priority.
pub fn cmd_priority(db: &mut Database, pm_dir: &Path, id: &str, new_priority: Priority) {
    mutate_task(db, pm_dir, id, "priority", |task| {
        task.priority_level = Some(new_priority);
    });
}

/// `pm due <id> <when>`: parse the human input and store as a `NaiveDate`.
pub fn cmd_due(db: &mut Database, pm_dir: &Path, id: &str, when: &str) {
    let parsed = match parse_due_input(when) {
        Some(d) => d,
        None => {
            eprintln!("due: could not parse {when:?}; try `today`, `next friday`, `in 1w`, or `YYYY-MM-DD`.");
            std::process::exit(1);
        }
    };
    mutate_task(db, pm_dir, id, "due", |task| task.due = Some(parsed));
}

/// `pm dep <id> needs|remove <dep_id>`: add or remove a dependency edge.
pub fn cmd_dep(db: &mut Database, pm_dir: &Path, id: &str, op: &str, dep_id: &str) {
    let dep = match dep_id.parse::<crate::store::IdInput>() {
        Ok(input) => input.leaf(),
        Err(e) => {
            eprintln!("dep: bad dep id {dep_id}: {e}");
            std::process::exit(1);
        }
    };
    match op.to_lowercase().as_str() {
        "needs" | "add" | "+" => {
            mutate_task(db, pm_dir, id, "dep needs", |task| {
                if !task.deps.contains(&dep) {
                    task.deps.push(dep);
                }
            });
        }
        "remove" | "rm" | "-" => {
            mutate_task(db, pm_dir, id, "dep remove", |task| {
                task.deps.retain(|d| d != &dep);
            });
        }
        other => {
            eprintln!("dep: unknown op {other:?}; expected `needs` or `remove`.");
            std::process::exit(1);
        }
    }
}

/// `pm tag <id> +foo -bar`: add and remove tags. Operations apply in order.
pub fn cmd_tag(db: &mut Database, pm_dir: &Path, id: &str, ops: &[String]) {
    if ops.is_empty() {
        eprintln!("tag: supply at least one op (e.g. `+infra`, `-draft`).");
        std::process::exit(1);
    }
    mutate_task(db, pm_dir, id, "tag", |task| {
        for op in ops {
            if let Some(name) = op.strip_prefix('+') {
                let normalised = normalise_tag(name);
                if !normalised.is_empty() && !task.tags.contains(&normalised) {
                    task.tags.push(normalised);
                }
            } else if let Some(name) = op.strip_prefix('-') {
                let normalised = normalise_tag(name);
                task.tags.retain(|t| t != &normalised);
            } else {
                eprintln!("tag: skipping {op:?} - prefix with + to add or - to remove.");
            }
        }
        task.tags.sort();
        task.tags.dedup();
    });
}

/// `pm link <id> <key> <url>`: set an external link on a ticket. Recognises
/// `issue`/`issue_link` and `pr`/`pr_link` as syntactic sugar for the
/// dedicated `Task` fields; other keys land in the front-matter `links` map
/// once that pathway is wired (Phase 11). For now, any other key is rejected
/// with a clear message.
pub fn cmd_link(db: &mut Database, pm_dir: &Path, id: &str, key: &str, url: &str) {
    let normalised = key.to_lowercase();
    match normalised.as_str() {
        "issue" | "issue_link" | "github_issue" => {
            mutate_task(db, pm_dir, id, "link issue", |task| {
                task.issue_link = Some(url.to_string());
            });
        }
        "pr" | "pr_link" | "github_pr" => {
            mutate_task(db, pm_dir, id, "link pr", |task| {
                task.pr_link = Some(url.to_string());
            });
        }
        other => {
            eprintln!("link: only `issue` and `pr` keys are wired through Task in Phase 4; got {other:?}.");
            std::process::exit(1);
        }
    }
}

/// `pm milestone <id> <MLSn>`: attach a ticket to a milestone.
pub fn cmd_milestone(db: &mut Database, pm_dir: &Path, id: &str, milestone_id: &str) {
    let mls = match milestone_id.parse::<crate::store::IdInput>() {
        Ok(input) => input.leaf(),
        Err(e) => {
            eprintln!("milestone: bad milestone id {milestone_id}: {e}");
            std::process::exit(1);
        }
    };
    if mls.prefix() != crate::store::TypePrefix::Milestone {
        eprintln!("milestone: expected an MLS-prefixed id, got {mls}.");
        std::process::exit(1);
    }
    if db.get(mls).is_none() {
        eprintln!("milestone: target milestone {mls} not found in workspace.");
        std::process::exit(1);
    }
    mutate_task(db, pm_dir, id, "milestone", |task| {
        task.milestone = Some(mls)
    });
}

/// `pm doctor [--migrate]`: rebuild `state.json` from disk and (with the
/// `--migrate` flag) import any legacy `tasks.json` files into the workspace
/// via the Phase 3.5 bridge.
pub fn cmd_doctor(pm_dir: &Path, migrate: bool) {
    if migrate {
        run_doctor_migrate(pm_dir);
    }
    run_doctor_rebuild(pm_dir);
    run_doctor_reap_locks(pm_dir);
}

/// Reap any stale locks as part of `pm doctor`, mirroring `pm locks`. A lock
/// whose heartbeat is older than its TTL is removed and a `lock-reaped` event
/// is recorded.
fn run_doctor_reap_locks(pm_dir: &Path) {
    match crate::store::locks::reap_stale(pm_dir, Utc::now()) {
        Ok(reaped) => {
            for id in &reaped {
                println!("doctor: reaped stale lock on {id}");
                emit_or_warn(pm_dir, "lock-reaped", Some(*id), None);
            }
        }
        Err(e) => eprintln!("doctor: could not reap stale locks: {e}"),
    }
}

fn run_doctor_rebuild(pm_dir: &Path) {
    use crate::store::claude_md::CLAUDE_MD;
    use crate::store::layout::Layout;
    use crate::store::state::{ItemEntry, State};
    use crate::store::Ticket;

    let layout = Layout::at(pm_dir);
    if !layout.is_initialised() {
        eprintln!("doctor: no .pm/ workspace at {}", pm_dir.display());
        std::process::exit(1);
    }

    let existing = State::load(&layout.state_path()).unwrap_or_else(|_| State::fresh());
    let mut rebuilt = State::fresh();
    // Preserve counters and tombstones; only items are rebuilt from disk.
    rebuilt.next = existing.next.clone();
    rebuilt.tombstones = existing.tombstones.clone();
    rebuilt.templates = existing.templates.clone();

    let mut found = 0usize;
    let mut added = 0usize;
    let mut removed_paths: Vec<String> = Vec::new();

    walk_tickets(&layout.root, &mut |abs_path: &Path| {
        let claude_path = abs_path.join(CLAUDE_MD);
        if !claude_path.exists() {
            return;
        }
        let Ok(ticket) = Ticket::read(&claude_path) else {
            return;
        };
        let leaf = ticket.front_matter.id;
        let Ok(rel) = abs_path.strip_prefix(&layout.root) else {
            return;
        };
        found += 1;
        if !existing.items.contains_key(&leaf) {
            added += 1;
        }
        rebuilt.items.insert(
            leaf,
            ItemEntry {
                path: rel.to_path_buf(),
            },
        );
        // Bump the counter past this leaf so future allocations skip it.
        let entry = rebuilt.next.entry(leaf.prefix()).or_insert(1);
        if leaf.number() >= *entry {
            *entry = leaf.number() + 1;
        }
    });

    for (leaf, entry) in &existing.items {
        if !rebuilt.items.contains_key(leaf) {
            removed_paths.push(entry.path.display().to_string());
        }
    }

    if let Err(e) = rebuilt.save(&layout.state_path()) {
        eprintln!("doctor: failed to write state.json: {e}");
        std::process::exit(1);
    }

    if added > 0 || !removed_paths.is_empty() {
        commit_or_warn(
            pm_dir,
            &format!(
                "pm: doctor rebuild (+{added}/-{} entries)",
                removed_paths.len()
            ),
        );
        emit_or_warn(
            pm_dir,
            "doctor",
            None,
            Some(&format!("+{added}/-{} entries", removed_paths.len())),
        );
    }

    println!(
        "doctor: scanned {found} tickets at {}",
        layout.root.display()
    );
    if added > 0 {
        println!("  added {added} entries to state.json");
    }
    if !removed_paths.is_empty() {
        println!("  removed {} stale entries", removed_paths.len());
        for p in &removed_paths {
            println!("    - {p}");
        }
    }
    if added == 0 && removed_paths.is_empty() {
        println!("  state.json was already up to date.");
    }
}

fn run_doctor_migrate(pm_dir: &Path) {
    use crate::store::layout::Layout;
    use crate::store::migrate::MigrationPlan;

    let layout = Layout::at(pm_dir);
    layout.init().map(|_| ()).unwrap_or_else(|e| {
        eprintln!("doctor --migrate: layout init failed: {e}");
        std::process::exit(1);
    });

    // Look for legacy tasks.json files inside the workspace directory and in
    // the user's HOME/.pm/ if that's where the workspace lives.
    let candidates: Vec<PathBuf> = collect_legacy_files(pm_dir);
    if candidates.is_empty() {
        println!(
            "doctor --migrate: no legacy tasks.json files found near {}",
            pm_dir.display()
        );
        return;
    }

    let backup_dir = pm_dir.join(".legacy-backup");
    if let Err(e) = fs::create_dir_all(&backup_dir) {
        eprintln!(
            "doctor --migrate: could not create {}: {e}",
            backup_dir.display()
        );
        std::process::exit(1);
    }

    let mut imported = 0usize;
    for legacy in candidates {
        match MigrationPlan::plan(&layout, &legacy) {
            Ok(plan) => {
                let mut db = Database::load(pm_dir);
                for step in plan.steps {
                    // Convert each legacy task into a v2 Task and let
                    // Database::save place it at the right path.
                    let task = crate::task::Task {
                        id: step.leaf,
                        title: step.title,
                        summary: None,
                        description: None,
                        user_story: None,
                        requirements: None,
                        tags: Vec::new(),
                        deps: Vec::new(),
                        milestone: None,
                        memories: Vec::new(),
                        due: None,
                        parent: step.parent,
                        kind: step.kind,
                        status: Status::Open,
                        priority_level: None,
                        urgency: None,
                        process_stage: None,
                        issue_link: None,
                        pr_link: None,
                        artifacts: Vec::new(),
                        created_at_utc: Utc::now().timestamp(),
                        updated_at_utc: Utc::now().timestamp(),
                    };
                    db.tasks.push(task);
                    imported += 1;
                }
                if let Err(e) = db.save(pm_dir) {
                    eprintln!(
                        "doctor --migrate: save after import of {}: {e}",
                        legacy.display()
                    );
                    continue;
                }
                // Archive the legacy file.
                let dest = backup_dir.join(legacy.file_name().expect("legacy has filename"));
                if let Err(e) = fs::rename(&legacy, &dest) {
                    eprintln!(
                        "doctor --migrate: warning - imported {} but could not archive to {}: {e}",
                        legacy.display(),
                        dest.display(),
                    );
                }
            }
            Err(e) => eprintln!(
                "doctor --migrate: plan failed for {}: {e}",
                legacy.display()
            ),
        }
    }
    println!(
        "doctor --migrate: imported {imported} tickets; legacy files archived to {}",
        backup_dir.display()
    );
}

/// Collect candidate legacy `*_tasks.json` files near the workspace. Looks
/// only at the workspace directory itself; nested directories are not
/// traversed because v2 stores them as `CLAUDE.md` files under `state.items`.
fn collect_legacy_files(pm_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(pm_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if name == "tasks.json" || name.ends_with("_tasks.json") {
                out.push(path);
            }
        }
    }
    out
}

/// Recursively walk every ticket directory under `root`. A "ticket directory"
/// is any directory that contains a `CLAUDE.md` file. The visitor receives the
/// absolute path of each such directory.
fn walk_tickets(root: &Path, visitor: &mut dyn FnMut(&Path)) {
    walk_tickets_inner(root, visitor);
}

fn walk_tickets_inner(dir: &Path, visitor: &mut dyn FnMut(&Path)) {
    if dir.join(crate::store::claude_md::CLAUDE_MD).exists() {
        visitor(dir);
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // Skip top-level metadata directories that never contain tickets.
        if matches!(name, "locks" | "artifacts" | ".legacy-backup" | "templates") {
            continue;
        }
        walk_tickets_inner(&path, visitor);
    }
}

/// `pm search <query>`: case-insensitive substring search across every
/// `CLAUDE.md` body and front-matter in the workspace. Prints `path:lineno:
/// line` for each hit.
pub fn cmd_search(pm_dir: &Path, query: &str) {
    use crate::store::claude_md::CLAUDE_MD;
    use crate::store::layout::Layout;

    let layout = Layout::at(pm_dir);
    if !layout.is_initialised() {
        eprintln!("search: no .pm/ workspace at {}", pm_dir.display());
        std::process::exit(1);
    }
    let needle = query.to_lowercase();
    let mut hits = 0usize;
    walk_tickets(&layout.root, &mut |abs_dir: &Path| {
        let claude_path = abs_dir.join(CLAUDE_MD);
        let Ok(content) = fs::read_to_string(&claude_path) else {
            return;
        };
        for (i, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                hits += 1;
                println!("{}:{}: {}", claude_path.display(), i + 1, line);
            }
        }
    });
    if hits == 0 {
        println!("(no matches)");
    }
}

// ----- Phase 6: lock protocol + activity feed -----

/// Record an activity-feed event, warning rather than aborting on failure -
/// a missed event line should not fail the command whose state is already
/// saved. Mirrors [`commit_or_warn`].
fn emit_or_warn(pm_dir: &Path, verb: &str, id: Option<crate::store::LeafId>, detail: Option<&str>) {
    if let Err(e) = crate::store::events::emit_event(pm_dir, verb, id, detail) {
        eprintln!("warning: could not write events.log: {e}");
    }
}

/// `pm checkout <id> [--intent ...]`: acquire an advisory lock on a ticket and
/// open a checkout span. The lock records the git HEAD as `base_commit` so
/// `pm checkin` can squash everything done during the span into one commit.
/// A live soft lock by another agent warns but does not block; a live hard
/// lock blocks.
pub fn cmd_checkout(pm_dir: &Path, id: &str, intent: Option<&str>) {
    use crate::store::locks::{self, AcquireOutcome, LockFile, LockMode, DEFAULT_TTL_SECONDS};

    let db = Database::load(pm_dir);
    let leaf = match resolve_v2_id(id, &db) {
        Some(l) => l,
        None => {
            eprintln!("checkout: ticket not found: {id}");
            std::process::exit(1);
        }
    };

    // The squash base is the HEAD before any checkout-span work.
    let base_commit = crate::store::git::head_commit(pm_dir).ok().flatten();

    let lock = LockFile::new(
        leaf,
        intent.map(|s| s.to_string()),
        DEFAULT_TTL_SECONDS,
        LockMode::Soft,
        base_commit,
    );

    match locks::acquire(pm_dir, &lock, Utc::now()) {
        Ok(AcquireOutcome::Acquired) => {
            println!("checkout: {leaf} locked by {}.", lock.agent);
        }
        Ok(AcquireOutcome::Overlapped { previous }) => {
            eprintln!(
                "checkout: warning - {leaf} was already checked out by {} (soft lock; proceeding).",
                previous.agent
            );
            println!("checkout: {leaf} locked by {}.", lock.agent);
        }
        Ok(AcquireOutcome::Blocked { holder }) => {
            eprintln!(
                "checkout: {leaf} is hard-locked by {}; cannot check out.",
                holder.agent
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("checkout: could not acquire lock: {e}");
            std::process::exit(1);
        }
    }

    emit_or_warn(pm_dir, "checkout", Some(leaf), intent);
    commit_or_warn(pm_dir, &commit_subject_for(leaf, "checkout", intent));
}

/// `pm checkin <id> [--summary ...] [--granular]`: release the lock and close
/// the checkout span. By default every commit made since the lock's
/// `base_commit` is squashed into one checkin commit; `--granular` keeps the
/// individual commits.
pub fn cmd_checkin(pm_dir: &Path, id: &str, summary: Option<&str>, granular: bool) {
    use crate::store::locks;

    let db = Database::load(pm_dir);
    let leaf = match resolve_v2_id(id, &db) {
        Some(l) => l,
        None => {
            eprintln!("checkin: ticket not found: {id}");
            std::process::exit(1);
        }
    };

    // Read the lock first so we have the squash base before releasing it.
    let base_commit = match locks::read(pm_dir, leaf) {
        Ok(Some(lock)) => lock.base_commit,
        Ok(None) => {
            eprintln!("checkin: no active lock on {leaf} (committing work anyway).");
            None
        }
        Err(e) => {
            eprintln!("checkin: could not read lock: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = locks::release(pm_dir, leaf) {
        eprintln!("checkin: could not release lock: {e}");
        std::process::exit(1);
    }
    emit_or_warn(pm_dir, "checkin", Some(leaf), summary);

    let message = commit_subject_for(leaf, "checkin", summary);
    commit_or_warn(pm_dir, &message);

    // Squash the checkout span unless --granular or there is no recorded base.
    match (granular, base_commit) {
        (false, Some(base)) => {
            if let Err(e) = crate::store::git::squash_since(pm_dir, &base, &message) {
                eprintln!("checkin: squash failed, leaving individual commits: {e}");
            } else {
                println!("checkin: {leaf} released; checkout span squashed.");
                return;
            }
        }
        (true, _) => {
            println!("checkin: {leaf} released; checkout-span commits kept (--granular).");
            return;
        }
        (false, None) => {}
    }
    println!("checkin: {leaf} released.");
}

/// `pm heartbeat <id>`: refresh the heartbeat on a held lock so it does not
/// go stale. Intended to be called periodically by a lock holder - a TUI or
/// an agent loop. It only touches the lock file; it neither commits nor emits
/// an event, since heartbeats are high-frequency and that would be noise.
pub fn cmd_heartbeat(pm_dir: &Path, id: &str) {
    use crate::store::locks;

    let db = Database::load(pm_dir);
    let leaf = match resolve_v2_id(id, &db) {
        Some(l) => l,
        None => {
            eprintln!("heartbeat: ticket not found: {id}");
            std::process::exit(1);
        }
    };
    match locks::refresh_heartbeat(pm_dir, leaf, Utc::now()) {
        Ok(true) => println!("heartbeat: {leaf} refreshed."),
        Ok(false) => {
            eprintln!("heartbeat: no active lock on {leaf}.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("heartbeat: {e}");
            std::process::exit(1);
        }
    }
}

/// `pm locks`: reap any stale locks, then list the live ones. A lock is stale
/// once its heartbeat is older than its TTL window.
pub fn cmd_locks(pm_dir: &Path) {
    use crate::store::locks;

    let now = Utc::now();
    match locks::reap_stale(pm_dir, now) {
        Ok(reaped) => {
            for id in &reaped {
                println!("locks: reaped stale lock on {id}");
                emit_or_warn(pm_dir, "lock-reaped", Some(*id), None);
            }
        }
        Err(e) => {
            eprintln!("locks: {e}");
            std::process::exit(1);
        }
    }

    let live = match locks::list(pm_dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("locks: {e}");
            std::process::exit(1);
        }
    };
    if live.is_empty() {
        println!("locks: no active locks.");
        return;
    }
    for lock in live {
        let started = lock.started_at.format("%Y-%m-%d %H:%M:%S");
        let intent = lock.intent.as_deref().unwrap_or("");
        println!("{}  {}  since {started}  {intent}", lock.id, lock.agent);
    }
}

/// `pm next [--agent ...]`: print the first ready work item - a Task or
/// Subtask that is open, has every dependency done, and carries no live lock.
/// Container kinds (Project/Product/Epic) and Milestones are not work an
/// agent picks up, so they are excluded. Stale locks are reaped first so a
/// dead hold does not hide a ready ticket. Ties break on the lowest id for a
/// deterministic answer.
pub fn cmd_next(pm_dir: &Path, agent: Option<&str>, _filter: Option<&str>) {
    use crate::store::locks;
    use std::collections::HashSet;

    let db = Database::load(pm_dir);
    let now = Utc::now();
    // Reap stale locks so a dead hold does not mask a ready ticket.
    let _ = locks::reap_stale(pm_dir, now);
    let locked: HashSet<crate::store::LeafId> = locks::list(pm_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.id)
        .collect();

    let done: HashSet<crate::store::LeafId> = db
        .tasks
        .iter()
        .filter(|t| t.status == Status::Done)
        .map(|t| t.id)
        .collect();

    let mut ready: Vec<&Task> = db
        .tasks
        .iter()
        .filter(|t| matches!(t.kind, Kind::Task | Kind::Subtask))
        .filter(|t| t.status == Status::Open)
        .filter(|t| !locked.contains(&t.id))
        .filter(|t| t.deps.iter().all(|d| done.contains(d)))
        .collect();
    ready.sort_by_key(|t| t.id);

    let who = agent
        .map(|s| s.to_string())
        .unwrap_or_else(crate::store::events::actor);
    match ready.first() {
        Some(t) => println!(
            "next ({who}): {}  {}  [{}]",
            t.id,
            t.title,
            format_kind(t.kind)
        ),
        None => println!("next ({who}): no ready tickets."),
    }
}

/// `pm tv [PATH]`: drive the full-screen activity view (Phase 9). Shares its
/// renderer with Mode 3 in the main TUI via [`run_activity_view`].
pub fn cmd_tv(pm_dir: &Path) {
    if let Err(e) = run_activity_view(pm_dir) {
        eprintln!("pm tv: {e}");
        std::process::exit(1);
    }
}
/// `pm log <id>`: print the git history filtered to commits that touched the
/// ticket's directory. Shells out to `git log -- <ticket-path>`, which does
/// the subtree filtering natively.
pub fn cmd_log(pm_dir: &Path, id: &str) {
    let db = Database::load(pm_dir);
    let leaf = match resolve_v2_id(id, db_ref(&db)).or_else(|| {
        // Allow logging tickets that exist in state.json even when Database::load
        // could not reconstruct them (e.g. a missing CLAUDE.md). Fall back to the
        // raw IdInput so the user still sees history for a half-broken ticket.
        id.parse::<crate::store::IdInput>().ok().map(|p| p.leaf())
    }) {
        Some(l) => l,
        None => {
            eprintln!("log: ticket not found: {id}");
            std::process::exit(1);
        }
    };

    let layout = crate::store::layout::Layout::at(pm_dir);
    let state = crate::store::state::State::load(&layout.state_path()).unwrap_or_default();
    let Some(entry) = state.items.get(&leaf) else {
        eprintln!("log: state.json has no path for {leaf}; run `pm doctor`.");
        std::process::exit(1);
    };

    let root = match crate::store::git::ensure_repo(pm_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("log: could not open git repository: {e}");
            std::process::exit(1);
        }
    };
    let root_canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    let pm_canonical = std::fs::canonicalize(pm_dir).unwrap_or_else(|_| pm_dir.to_path_buf());
    let ticket_path_in_repo = match pm_canonical.join(&entry.path).strip_prefix(&root_canonical) {
        Ok(p) => p.to_path_buf(),
        Err(_) => {
            eprintln!("log: workspace lies outside repository workdir.");
            std::process::exit(1);
        }
    };
    let pathspec = ticket_path_in_repo.to_string_lossy().into_owned();

    // `%H` hash, `%ct` committer unix timestamp, `%s` subject, tab-separated.
    // `git log -- <pathspec>` keeps only commits whose diff touches that path.
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "--pretty=format:%H%x09%ct%x09%s", "--"])
        .arg(&pathspec)
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            eprintln!(
                "log: git log failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("log: could not run git: {e}");
            std::process::exit(1);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut printed = 0usize;
    for line in stdout.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(hash), Some(ts), Some(summary)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let dt = ts
            .parse::<i64>()
            .ok()
            .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "?".into());
        let short = if hash.len() >= 8 { &hash[..8] } else { hash };
        println!("{short}  {dt}  {summary}");
        printed += 1;
    }
    if printed == 0 {
        println!("(no commits touch {leaf} - check `pm doctor` if you expect history.)");
    }
}

/// Borrow helper so the deferred id resolution can fall back when Database::load
/// returns an empty set.
fn db_ref(db: &Database) -> &Database {
    db
}

/// `pm memory <action>` (Phase 10): three-tier memory store. Dispatches each
/// `MemoryAction` to the matching helper. Errors print a friendly message
/// and exit with code 1.
/// `pm mcp`: drive the stdio MCP server until stdin closes. Errors during
/// the loop exit with code 1.
pub fn cmd_mcp(pm_dir: &Path) {
    if let Err(e) = run_mcp_server(pm_dir.to_path_buf()) {
        eprintln!("pm mcp: {e}");
        std::process::exit(1);
    }
}

pub fn cmd_memory(db: &mut Database, pm_dir: &Path, action: MemoryAction) {
    match action {
        MemoryAction::Link { id, name } => memory_link(db, pm_dir, &id, &name),
        MemoryAction::Unlink { id, name } => memory_unlink(db, pm_dir, &id, &name),
        MemoryAction::List { id } => memory_list(db, pm_dir, &id),
        MemoryAction::Write {
            scope,
            ty,
            name,
            desc,
            ticket,
            project,
            content,
        } => memory_write(
            db,
            pm_dir,
            &scope,
            &ty,
            &name,
            desc.as_deref(),
            ticket.as_deref(),
            project.as_deref(),
            &content,
        ),
        MemoryAction::Promote { name, to } => memory_promote(db, pm_dir, &name, &to),
        MemoryAction::Show { name } => memory_show(db, pm_dir, &name),
    }
}

/// Resolve a ticket id to (leaf, absolute path to the ticket directory).
fn resolve_ticket_dir(db: &Database, pm_dir: &Path, id: &str) -> Option<(LeafId, PathBuf)> {
    let leaf = resolve_v2_id(id, db)?;
    let entry = db.state.items.get(&leaf)?;
    Some((leaf, pm_dir.join(&entry.path)))
}

/// Walk a ticket's parent chain to find the enclosing `PRJ` ancestor. Returns
/// `None` when the ticket is orphaned (no Project on the chain).
fn project_ancestor(db: &Database, leaf: LeafId) -> Option<LeafId> {
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(id) = cursor {
        if guard > 16 {
            break;
        }
        guard += 1;
        let task = db.get(id)?;
        if matches!(task.kind, Kind::Project) {
            return Some(task.id);
        }
        cursor = task.parent;
    }
    None
}

/// Look up the only `Kind::Project` ticket in the workspace, if there is
/// exactly one. Used to default `--project` for write/promote when omitted.
fn solo_project(db: &Database) -> Option<LeafId> {
    let mut found: Option<LeafId> = None;
    for task in db.tasks.iter() {
        if matches!(task.kind, Kind::Project) {
            if found.is_some() {
                return None; // more than one project, caller must specify
            }
            found = Some(task.id);
        }
    }
    found
}

/// Build a [`MemoryContext`] for a CLI verb. `ticket_arg` resolves to a
/// ticket-tier path; `project_override` lets the user pin a project leaf
/// when the workspace has more than one.
fn build_memory_context(
    db: &Database,
    pm_dir: &Path,
    ticket_arg: Option<&str>,
    project_override: Option<&str>,
) -> Result<MemoryContext, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME environment variable is not set".to_string())?;
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;

    let active_ticket_dir = if let Some(id) = ticket_arg {
        match resolve_ticket_dir(db, pm_dir, id) {
            Some((_, path)) => Some(path),
            None => return Err(format!("ticket not found: {id}")),
        }
    } else {
        None
    };

    let active_project = if let Some(arg) = project_override {
        let leaf = resolve_v2_id(arg, db).ok_or_else(|| format!("project not found: {arg}"))?;
        if !matches!(db.get(leaf).map(|t| t.kind), Some(Kind::Project)) {
            return Err(format!("--project must be a PRJ leaf; got {arg}"));
        }
        Some(leaf)
    } else if let Some(ticket_id) = ticket_arg {
        let leaf =
            resolve_v2_id(ticket_id, db).ok_or_else(|| format!("ticket not found: {ticket_id}"))?;
        project_ancestor(db, leaf)
    } else {
        solo_project(db)
    };

    Ok(MemoryContext {
        home,
        cwd,
        pm_root: pm_dir.to_path_buf(),
        active_project,
        active_ticket_dir,
    })
}

/// `pm memory link <id> <name>`: link an existing memory to a ticket. Resolves
/// the tier from disk (project before ticket before user), records the
/// matching [`MemoryRef`] variant in the ticket's `memories:` front-matter,
/// and saves. Idempotent on a re-link.
fn memory_link(db: &mut Database, pm_dir: &Path, id: &str, name: &str) {
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("memory link: ticket not found: {id}");
            std::process::exit(1);
        }
    };

    let ticket_dir = match db.state.items.get(&leaf).map(|e| pm_dir.join(&e.path)) {
        Some(p) => p,
        None => {
            eprintln!("memory link: state.json has no entry for {id}");
            std::process::exit(1);
        }
    };
    let ctx = MemoryContext {
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        pm_root: pm_dir.to_path_buf(),
        active_project: project_ancestor(db, leaf),
        active_ticket_dir: Some(ticket_dir.clone()),
    };

    let hit = match lookup_by_name(&ctx, name) {
        Ok(Some(h)) => h,
        Ok(None) => {
            eprintln!("memory link: no memory named {name:?} at any tier");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("memory link: {e}");
            std::process::exit(1);
        }
    };
    let memref = memref_for(&hit, name);
    let claude_path = ticket_dir.join("CLAUDE.md");
    let mut ticket = match crate::store::Ticket::read(&claude_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("memory link: cannot read {}: {e}", claude_path.display());
            std::process::exit(1);
        }
    };
    if !ticket
        .front_matter
        .memories
        .iter()
        .any(|m| memref_eq(m, &memref))
    {
        ticket.front_matter.memories.push(memref.clone());
        if let Err(e) = ticket.write_to(&ticket_dir) {
            eprintln!("memory link: cannot write {}: {e}", claude_path.display());
            std::process::exit(1);
        }
    }
    println!("linked {name} [{}] to {leaf}", hit.location.scope.as_str());
}

/// `pm memory unlink <id> <name>`: remove every front-matter reference to
/// `name` regardless of tier. Idempotent.
fn memory_unlink(db: &mut Database, pm_dir: &Path, id: &str, name: &str) {
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("memory unlink: ticket not found: {id}");
            std::process::exit(1);
        }
    };
    let ticket_dir = match db.state.items.get(&leaf).map(|e| pm_dir.join(&e.path)) {
        Some(p) => p,
        None => {
            eprintln!("memory unlink: state.json has no entry for {id}");
            std::process::exit(1);
        }
    };
    let claude_path = ticket_dir.join("CLAUDE.md");
    let mut ticket = match crate::store::Ticket::read(&claude_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("memory unlink: cannot read {}: {e}", claude_path.display());
            std::process::exit(1);
        }
    };
    let before = ticket.front_matter.memories.len();
    ticket
        .front_matter
        .memories
        .retain(|m| memref_name(m) != name);
    let after = ticket.front_matter.memories.len();
    if after == before {
        println!("unlink {name} from {leaf}: nothing to remove");
        return;
    }
    if let Err(e) = ticket.write_to(&ticket_dir) {
        eprintln!("memory unlink: cannot write {}: {e}", claude_path.display());
        std::process::exit(1);
    }
    println!(
        "unlinked {name} from {leaf} ({} entries removed)",
        before - after
    );
}

/// `pm memory list <id>`: print every memory linked to a ticket, tier-tagged.
fn memory_list(db: &Database, pm_dir: &Path, id: &str) {
    let leaf = match resolve_v2_id(id, db) {
        Some(l) => l,
        None => {
            eprintln!("memory list: ticket not found: {id}");
            std::process::exit(1);
        }
    };
    let ticket_dir = match db.state.items.get(&leaf).map(|e| pm_dir.join(&e.path)) {
        Some(p) => p,
        None => {
            eprintln!("memory list: state.json has no entry for {id}");
            std::process::exit(1);
        }
    };
    let claude_path = ticket_dir.join("CLAUDE.md");
    let ticket = match crate::store::Ticket::read(&claude_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("memory list: cannot read {}: {e}", claude_path.display());
            std::process::exit(1);
        }
    };

    if ticket.front_matter.memories.is_empty() {
        println!("{leaf}: no linked memories.");
        return;
    }
    println!("{leaf} linked memories:");
    for memref in &ticket.front_matter.memories {
        let tier = match memref {
            MemoryRef::User(_) => "user",
            MemoryRef::Project(_) => "project",
            MemoryRef::Ticket(_) => "ticket",
        };
        println!("  - {:<32}  [{tier}]", memref_name(memref));
    }
}

/// `pm memory write`: see the verb's clap doc for argument shape.
#[allow(clippy::too_many_arguments)]
fn memory_write(
    db: &Database,
    pm_dir: &Path,
    scope_arg: &str,
    type_arg: &str,
    name: &str,
    desc: Option<&str>,
    ticket_arg: Option<&str>,
    project_arg: Option<&str>,
    content: &str,
) {
    let scope = match Scope::parse(scope_arg) {
        Some(s) => s,
        None => {
            eprintln!("memory write: unknown --scope: {scope_arg} (use user|project|ticket)");
            std::process::exit(1);
        }
    };
    let kind = match MemoryType::parse(type_arg) {
        Some(t) => t,
        None => {
            eprintln!(
                "memory write: unknown --type: {type_arg} (use user|feedback|project|reference)"
            );
            std::process::exit(1);
        }
    };
    let ctx = match build_memory_context(db, pm_dir, ticket_arg, project_arg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("memory write: {e}");
            std::process::exit(1);
        }
    };
    match write_memory(
        &ctx,
        scope,
        name,
        kind,
        desc.map(|s| s.to_string()),
        content,
    ) {
        Ok(loc) => println!("wrote {} ({})", loc.file.display(), scope.as_str()),
        Err(e) => {
            eprintln!("memory write: {e}");
            std::process::exit(1);
        }
    }
}

/// `pm memory promote <name> --to <scope>`: move a memory between user and
/// project tiers. user->project leaves a back-reference; project->user
/// removes the project copy.
fn memory_promote(db: &Database, pm_dir: &Path, name: &str, to_arg: &str) {
    let target = match Scope::parse(to_arg) {
        Some(s) => s,
        None => {
            eprintln!("memory promote: unknown --to: {to_arg} (use user|project)");
            std::process::exit(1);
        }
    };
    let ctx = match build_memory_context(db, pm_dir, None, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("memory promote: {e}");
            std::process::exit(1);
        }
    };
    match promote_memory(&ctx, name, target) {
        Ok(outcome) => {
            println!(
                "promoted {name}: {} -> {}",
                outcome.source.file.display(),
                outcome.target.file.display()
            );
            if let Some(backref) = outcome.backref {
                println!("back-reference written at {}", backref.file.display());
            }
        }
        Err(e) => {
            eprintln!("memory promote: {e}");
            std::process::exit(1);
        }
    }
}

/// `pm memory show <name>`: print the contents of the resolved memory file
/// using the project-first collision rule.
fn memory_show(db: &Database, pm_dir: &Path, name: &str) {
    let ctx = match build_memory_context(db, pm_dir, None, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("memory show: {e}");
            std::process::exit(1);
        }
    };
    match lookup_by_name(&ctx, name) {
        Ok(Some(hit)) => {
            println!(
                "# {} [{}]",
                hit.file.front_matter.name,
                hit.location.scope.as_str()
            );
            if let Some(desc) = &hit.file.front_matter.description {
                println!("# {desc}");
            }
            println!();
            print!("{}", hit.file.body);
            if !hit.file.body.ends_with('\n') {
                println!();
            }
        }
        Ok(None) => {
            eprintln!("memory show: not found: {name}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("memory show: {e}");
            std::process::exit(1);
        }
    }
}

/// Build a [`MemoryRef`] variant matching the tier the hit was resolved from.
fn memref_for(hit: &MemoryHit, name: &str) -> MemoryRef {
    match hit.location.scope {
        Scope::User => MemoryRef::User(name.to_string()),
        Scope::Project => MemoryRef::Project(name.to_string()),
        Scope::Ticket => MemoryRef::Ticket(name.to_string()),
    }
}

/// Compare two memory references by tier and name.
fn memref_eq(a: &MemoryRef, b: &MemoryRef) -> bool {
    match (a, b) {
        (MemoryRef::User(x), MemoryRef::User(y))
        | (MemoryRef::Project(x), MemoryRef::Project(y))
        | (MemoryRef::Ticket(x), MemoryRef::Ticket(y)) => x == y,
        _ => false,
    }
}

/// Extract the memory's name from a [`MemoryRef`] regardless of tier.
fn memref_name(m: &MemoryRef) -> &str {
    match m {
        MemoryRef::User(s) | MemoryRef::Project(s) | MemoryRef::Ticket(s) => s.as_str(),
    }
}
