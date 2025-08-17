//! Command implementations for the CLI interface.
//!
//! This module contains all the command handlers that implement the various
//! subcommands available in the CLI, from basic CRUD operations to complex
//! hierarchical queries and the TUI interface.

use clap::Subcommand;
use clap_complete::{generate, Shell};

use std::path::Path;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use chrono::{Local, NaiveDate, TimeZone, Utc};
use crate::db::*;
use crate::fields::*;
use crate::task::{Task, TaskTemplate};
use crate::tui::run::{run_tui, run_tui_with_edit};
use crate::tui::menu::MenuApp;
use crate::tui::workflow_run::run_workflow_tui;
use crate::tui::workflow::WorkflowExit;

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
        /// Project name.
        #[arg(long)]
        project: Option<String>,
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
        project: Option<String>,
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
        id: String 
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
        /// Default project
        #[arg(long)]
        project: Option<String>,
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
    project: Option<String>,
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
    let (task_kind, final_project, final_tags, final_priority, final_urgency, final_process_stage, final_status, final_desc) = 
        if let Some(template_name) = template {
            let template = db.templates.iter()
                .find(|t| t.name == template_name)
                .cloned();
            
            match template {
                Some(tmpl) => {
                    let template_tags = if tags.is_empty() { tmpl.tags } else { split_and_normalise_tags(&tags) };
                    (
                        if kind == Kind::Task { tmpl.kind } else { kind },
                        project.or(tmpl.project),
                        template_tags,
                        priority_level.or(tmpl.priority_level),
                        urgency.or(tmpl.urgency),
                        process_stage.or(tmpl.process_stage),
                        if status == Status::Open { tmpl.status } else { status },
                        desc.or(tmpl.description_template.clone()),
                    )
                },
                None => {
                    eprintln!("Template '{}' not found", template_name);
                    std::process::exit(1);
                }
            }
        } else {
            (kind, project, split_and_normalise_tags(&tags), priority_level, urgency, process_stage, status, desc)
        };
    
    let now_utc = Utc::now().timestamp();
    let id = db.next_id();

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
                        eprintln!("Invalid hierarchy: {} cannot be child of {}. Valid hierarchy: Product > Epic > Task > Subtask",
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
    let artifacts_list = artifacts.iter()
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
        project: final_project.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
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
    db.tasks.push(task);
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
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
                if t.project.as_deref() != Some(p.as_str()) {
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
                a_priority.cmp(&b_priority)
                    .then(a_urgency.cmp(&b_urgency))
                    .then(a.id.cmp(&b.id))
            });
        },
        SortKey::Id => filtered.sort_by_key(|t| t.id),
    }

    if let Some(n) = limit {
        filtered.truncate(n);
    }

    if tree {
        // Compute depths for indentation using ancestry in the full DB.
        let mut depth_map: HashMap<u64, usize> = HashMap::new();
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
        print_table(&filtered, Some(&depth_map));
    } else {
        print_table(&filtered, None);
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
    println!("ID:           {}", task.id);
    println!("Title:        {}", task.title);
    println!("Kind:         {}", format_kind(task.kind));
    println!("Status:       {}", format_status(task.status));
    println!("Priority:     {}", format_priority(task.priority_level));
    println!("Project:      {}", task.project.clone().unwrap_or_else(|| "-".into()));
    println!("Due:          {}", match task.due { Some(d) => format!("{d} ({})", format_due_relative(Some(d), today)), None => "-".into() });
    println!("Parent:       {}", task.parent.map(|p| p.to_string()).unwrap_or_else(|| "-".into()));
    println!("Tags:         {}", if task.tags.is_empty() { "-".into() } else { task.tags.join(",") });
    println!("Created UTC:  {}", Utc.timestamp_opt(task.created_at_utc, 0).single().unwrap().to_rfc3339());
    println!("Updated UTC:  {}", Utc.timestamp_opt(task.updated_at_utc, 0).single().unwrap().to_rfc3339());
    println!("Description:\n{}\n", task.description.unwrap_or_else(|| "-".into()));

    let child_map = build_children_map(&db.tasks);

    if parents {
        let chain = collect_ancestors(task_id, db);
        if chain.is_empty() {
            println!("Ancestors: -");
        } else {
            println!("Ancestors (closest first): {}", chain.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" -> "));
        }
    }

    if children {
        println!("Children:");
        if let Some(_children) = child_map.get(&task_id) {
            // Depth-first print.
            let idx = db.index();
            pub fn dfs(
                id: u64,
                child_map: &BTreeMap<u64, Vec<u64>>,
                idx: &HashMap<u64, usize>,
                db: &Database,
                depth: usize,
            ) {
                if let Some(children) = child_map.get(&id) {
                    for &c in children {
                        if let Some(&i) = idx.get(&c) {
                            let t = &db.tasks[i];
                            println!("{}- {} [{}] (#{})", "  ".repeat(depth), t.title, format_status(t.status), t.id);
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
    project: Option<String>,
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
            if hops > 64 { break; }
        }
    }

    // Store values needed for hierarchy validation 
    let (final_parent, final_kind) = {
        let Some(t) = db.get_mut(task_id) else {
            eprintln!("Task {} not found.", task_id);
            std::process::exit(1);
        };
        if let Some(s) = title { t.title = s; }
        if let Some(d) = desc { t.description = if d.is_empty() { None } else { Some(d) }; }
        if let Some(p) = project { t.project = if p.trim().is_empty() { None } else { Some(p.trim().to_string()) }; }
        if clear_due { t.due = None; }
        if let Some(ds) = due {
            t.due = parse_due_input(&ds);
            if t.due.is_none() {
                eprintln!("Unrecognised due date. Use YYYY-MM-DD, 'today', 'tomorrow', or 'in Nd'.");
                std::process::exit(1);
            }
        }
        if clear_parent { t.parent = None; }
        if let Some(pid) = parent_id {
            t.parent = Some(pid);
        }
        if let Some(k) = kind { t.kind = k; }
        if let Some(s) = status { t.status = s; }
        
        (t.parent, t.kind)
    };
    
    // Validate hierarchy after kind/parent updates
    if let Some(parent_id) = final_parent {
        if let Some(parent_task) = db.get(parent_id) {
            if !validate_hierarchy(parent_task.kind, final_kind) {
                eprintln!("Invalid hierarchy: {} cannot be child of {}. Valid hierarchy: Product > Epic > Task > Subtask",
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
    let rm = split_and_normalise_tags(&rm_tags).into_iter().collect::<HashSet<_>>();
    if !add.is_empty() || !rm.is_empty() {
        // Merge tags.
        let mut set = t.tags.iter().cloned().collect::<BTreeSet<_>>();
        for a in add.drain(..) { set.insert(a); }
        for r in rm { set.remove(&r); }
        t.tags = set.into_iter().collect();
    }

    t.updated_at_utc = Utc::now().timestamp();
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
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
    let option_count = [id.is_some(), tag.is_some(), project.is_some(), status_filter.is_some()].iter().filter(|&&x| x).count();
    if option_count != 1 {
        eprintln!("Error: Must specify exactly one of --id, --tag, --project, or --status");
        std::process::exit(1);
    }
    
    let mut to_mark: HashSet<u64> = HashSet::new();
    
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
                task.project.as_ref() == Some(project_filter)
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
    let option_count = [id.is_some(), tag.is_some(), project.is_some(), status_filter.is_some()].iter().filter(|&&x| x).count();
    if option_count != 1 {
        eprintln!("Error: Must specify exactly one of --id, --tag, --project, or --status");
        std::process::exit(1);
    }
    
    let mut to_delete: HashSet<u64> = HashSet::new();
    
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
        let mut children: HashSet<u64> = HashSet::new();
        collect_descendants(task_id, &child_map, &mut children);
        if !children.is_empty() && !cascade {
            eprintln!("Task {} has {} descendant(s). Use --cascade to delete all.", task_id, children.len());
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
                task.project.as_ref() == Some(project_filter)
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
    db.remove_ids(&ids);
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save DB: {e}");
        std::process::exit(1);
    }
    println!("Deleted.");
}

/// List all distinct project names in the database.
pub fn cmd_projects(db: &Database) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in &db.tasks {
        let key = t.project.clone().unwrap_or_else(|| "-".into());
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
    use clap::CommandFactory;
    use crate::cli::Cli;
    
    let mut app = Cli::command();
    let app_name = app.get_name().to_string();
    generate(shell, &mut app, app_name, &mut std::io::stdout());
}

/// Handle template management commands.
pub fn cmd_template(db: &mut Database, db_path: &Path, action: TemplateAction) {
    match action {
        TemplateAction::Save { task_id, template_name } => {
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
            if db.templates.iter().any(|t| t.name == template_name) {
                eprintln!("Template '{}' already exists. Use a different name.", template_name);
                std::process::exit(1);
            }
            
            let template = TaskTemplate {
                name: template_name.clone(),
                title_template: Some(task.title.clone()),
                description_template: task.description.clone(),
                project: task.project.clone(),
                tags: task.tags.clone(),
                kind: task.kind,
                priority_level: task.priority_level,
                urgency: task.urgency,
                process_stage: task.process_stage,
                status: task.status,
            };
            
            db.templates.push(template);
            
            if let Err(e) = db.save(db_path) {
                eprintln!("Failed to save database: {}", e);
                std::process::exit(1);
            }
            
            println!("Saved template '{}' from task {}", template_name, task_id_resolved);
        },
        
        TemplateAction::List => {
            if db.templates.is_empty() {
                println!("No templates found.");
                return;
            }
            
            println!("{:<20} {:<10} {:<12} {:<15}", "Name", "Kind", "Status", "Project");
            for template in &db.templates {
                let project = template.project.as_deref().unwrap_or("-");
                println!(
                    "{:<20} {:<10} {:<12} {:<15}",
                    truncate(&template.name, 20),
                    format_kind(template.kind),
                    format_status(template.status),
                    truncate(project, 15)
                );
            }
        },
        
        TemplateAction::Delete { template_name } => {
            let initial_len = db.templates.len();
            db.templates.retain(|t| t.name != template_name);
            
            if db.templates.len() == initial_len {
                eprintln!("Template '{}' not found.", template_name);
                std::process::exit(1);
            }
            
            if let Err(e) = db.save(db_path) {
                eprintln!("Failed to save database: {}", e);
                std::process::exit(1);
            }
            
            println!("Deleted template '{}'", template_name);
        },
        
        TemplateAction::Create {
            name,
            title_template,
            description,
            project,
            tags,
            kind,
            priority,
            urgency,
            process_stage,
            status,
        } => {
            // Check if template already exists
            if db.templates.iter().any(|t| t.name == name) {
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
                project,
                tags: template_tags,
                kind,
                priority_level: priority,
                urgency,
                process_stage,
                status,
            };
            
            db.templates.push(template);
            
            if let Err(e) = db.save(db_path) {
                eprintln!("Failed to save database: {}", e);
                std::process::exit(1);
            }
            
            println!("Created template '{}'", name);
        },
    }
}

/// Export tasks to CSV format for external analysis and time tracking.
pub fn cmd_export(
    db: &Database, 
    output: Option<String>, 
    all: bool, 
    project: Option<String>, 
    tag: Option<String>
) {
    let output_path = output.unwrap_or_else(|| "tasks.csv".to_string());
    
    // Filter tasks
    let tasks: Vec<&Task> = db.tasks.iter()
        .filter(|task| {
            // Include completed tasks only if --all is specified
            if !all && task.status == Status::Done {
                return false;
            }
            
            // Project filter
            if let Some(ref proj_filter) = project {
                if task.project.as_ref() != Some(proj_filter) {
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
        let priority = task.priority_level.map(|p| format_priority(Some(p))).unwrap_or("-");
        let urgency = task.urgency.map(|u| format_urgency(Some(u))).unwrap_or("-");
        let process_stage = task.process_stage.map(|ps| format_process_stage(Some(ps))).unwrap_or("-");
        let project = task.project.as_deref().unwrap_or("-");
        let tags = if task.tags.is_empty() { "-".to_string() } else { task.tags.join(";") };
        let due = task.due.map(|d| d.to_string()).unwrap_or("-".to_string());
        let parent = task.parent.map(|p| p.to_string()).unwrap_or("-".to_string());
        let created = chrono::Utc.timestamp_opt(task.created_at_utc, 0).single().unwrap().to_rfc3339();
        let updated = chrono::Utc.timestamp_opt(task.updated_at_utc, 0).single().unwrap().to_rfc3339();
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
            escape_csv(project),
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
        },
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
            "Database file does not exist"
        ));
    }

    let parent_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let backup_dir = parent_dir.join("backup");
    
    // Create backup directory if it doesn't exist
    fs::create_dir_all(&backup_dir)?;
    
    // Generate timestamp for backup filename
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let db_filename = db_path.file_name()
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
            },
            Err(e) => {
                eprintln!("Warning: Failed to create backup: {}", e);
                print!("Continue without backup? (y/N): ");
                use std::io::{self, Write};
                io::stdout().flush().unwrap();
                
                let mut response = String::new();
                if io::stdin().read_line(&mut response).is_err() 
                    || !response.trim().to_lowercase().starts_with('y') {
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
        eprintln!("Invalid CSV header. Expected:\n{}\nGot:\n{}", expected_header, lines[0]);
        std::process::exit(1);
    }
    
    let mut imported_count = 0;
    let mut skipped_count = 0;
    let mut next_id = db.next_id();
    
    // Process each CSV row (skip header)
    for (line_num, line) in lines.iter().skip(1).enumerate() {
        let line_num = line_num + 2; // +2 because we skip header and line numbers are 1-based
        
        // Simple CSV parsing (handles quoted fields)
        let fields = parse_csv_line(line);
        if fields.len() != 14 {
            eprintln!("Warning: Line {} has {} fields, expected 14. Skipping.", line_num, fields.len());
            skipped_count += 1;
            continue;
        }
        
        // Parse fields
        let _original_id = fields[0].parse::<u64>().unwrap_or(0);
        let title = fields[1].clone();
        let kind = parse_kind(&fields[2]);
        let status = parse_status(&fields[3]);
        let priority = parse_priority(&fields[4]);
        let urgency = parse_urgency(&fields[5]);
        let process_stage = parse_process_stage(&fields[6]);
        let project = if fields[7] == "-" { None } else { Some(fields[7].clone()) };
        let tags = if fields[8] == "-" { Vec::new() } else { fields[8].split(';').map(|s| s.to_string()).collect() };
        let due = if fields[9] == "-" { None } else { NaiveDate::parse_from_str(&fields[9], "%Y-%m-%d").ok() };
        let parent = if fields[10] == "-" { None } else { fields[10].parse::<u64>().ok() };
        let description = if fields[13] == "-" { None } else { Some(fields[13].clone()) };
        
        if title.is_empty() {
            eprintln!("Warning: Line {} has empty title. Skipping.", line_num);
            skipped_count += 1;
            continue;
        }
        
        // Check if task with same title already exists
        if db.tasks.iter().any(|t| t.title == title) {
            eprintln!("Warning: Task with title '{}' already exists. Skipping.", title);
            skipped_count += 1;
            continue;
        }
        
        // Create new task with sequential ID
        let new_task = Task {
            id: next_id,
            title,
            summary: None, // CSV doesn't include summary field
            description,
            user_story: None, // CSV doesn't include user_story field  
            requirements: None, // CSV doesn't include requirements field
            tags,
            project,
            due,
            parent,
            kind,
            status,
            priority_level: priority,
            urgency,
            process_stage,
            issue_link: None, // CSV doesn't include issue_link field
            pr_link: None, // CSV doesn't include pr_link field
            artifacts: Vec::new(), // CSV doesn't include artifacts field
            created_at_utc: Utc::now().timestamp(),
            updated_at_utc: Utc::now().timestamp(),
        };
        
        db.tasks.push(new_task);
        imported_count += 1;
        next_id += 1;
    }
    
    // Save database
    if let Err(e) = db.save(db_path) {
        eprintln!("Failed to save database: {}", e);
        std::process::exit(1);
    }
    
    println!("Import completed. {} tasks imported, {} skipped.", imported_count, skipped_count);
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
            },
            ',' if !in_quotes => {
                // Field separator
                fields.push(current_field);
                current_field = String::new();
            },
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
        },
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
            },
            Err(e) => {
                eprintln!("Failed to backup {}: {}", project.display_name, e);
            }
        }
    }
    
    println!("Backup completed: {}/{} projects backed up successfully.", success_count, total_count);
}

/// Export all projects to CSV format.
pub fn cmd_export_all(pm_dir: &Path, output: Option<String>, include_completed: bool, project_filter: Option<String>, tag_filter: Option<String>) {
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
    let mut all_tasks = Vec::new();
    
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
            
            all_tasks.push((project.clone(), task.clone()));
        }
    }
    
    // Create CSV content
    let mut csv_content = String::new();
    
    // CSV Header (add project name column)
    csv_content.push_str("ProjectName,ID,Title,Kind,Status,Priority,Urgency,ProcessStage,Project,Tags,Due,Parent,CreatedUTC,UpdatedUTC,Description\n");
    
    // CSV Rows
    let task_count = all_tasks.len();
    for (project, task) in &all_tasks {
        let priority = task.priority_level.map(|p| format_priority(Some(p))).unwrap_or("-");
        let urgency = task.urgency.map(|u| format_urgency(Some(u))).unwrap_or("-");
        let process_stage = task.process_stage.map(|ps| format_process_stage(Some(ps))).unwrap_or("-");
        let project_field = task.project.as_deref().unwrap_or("-");
        let tags = if task.tags.is_empty() { "-".to_string() } else { task.tags.join(";") };
        let due = task.due.map(|d| d.to_string()).unwrap_or("-".to_string());
        let parent = task.parent.map(|p| p.to_string()).unwrap_or("-".to_string());
        let created = chrono::Utc.timestamp_opt(task.created_at_utc, 0).single().unwrap().to_rfc3339();
        let updated = chrono::Utc.timestamp_opt(task.updated_at_utc, 0).single().unwrap().to_rfc3339();
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
            escape_csv(project_field),
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
            println!("Exported {} task(s) from {} project(s) to {}", task_count, projects.len(), output_path);
        },
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
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    ).unwrap();
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
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen
    ).unwrap();
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
