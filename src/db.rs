//! Database operations and utility functions for task management.
//!
//! Provides the [`Database`] struct for storing and managing tasks plus the
//! free functions that drive display, parsing, and hierarchy validation. The
//! in-memory data model now identifies tickets by [`LeafId`] (carrying both
//! the ticket type via prefix and a monotonic per-type number). ID
//! allocation goes through the embedded [`State`] counter map.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::fields::*;
use crate::store::artifacts::{self, ArtifactsIndex};
use crate::store::claude_md::{Ticket, ARTIFACTS_IMPORT, CLAUDE_MD};
use crate::store::id::{AddressId, IdInput, LeafId, TypePrefix};
use crate::store::layout::Layout;
use crate::store::sections::ParsedBody;
use crate::store::state::{ItemEntry, State};
use crate::store::task_bridge::{task_from_document, task_to_document};
use crate::task::Task;

/// In-memory database for storing and managing tasks.
///
/// The `state` field carries the per-type monotonic counters used by
/// [`Database::allocate_id`], the tombstone set so reused numbers stay out of
/// circulation, the on-disk path index for each ticket, and the named
/// [`crate::task::TaskTemplate`] presets used by the template commands.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Database {
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub state: State,
}

impl Database {
    /// Load the database from a `.pm/` workspace directory.
    ///
    /// Reads `<pm_dir>/state.json`, then for each indexed leaf id reads the
    /// per-ticket `CLAUDE.md` and reconstructs a [`Task`] through the
    /// [`task_from_document`] bridge. Artifact filenames are sourced from the
    /// ticket's `artifacts/` directory (preferring `ARTIFACTS.md` when
    /// present, falling back to a directory listing).
    ///
    /// Returns an empty database when the workspace has not been initialised.
    pub fn load(pm_dir: &Path) -> Self {
        if !pm_dir.exists() {
            return Database::default();
        }
        let layout = Layout::at(pm_dir);
        let state = match State::load(&layout.state_path()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error parsing state.json, starting fresh: {e}");
                State::fresh()
            }
        };

        let mut tasks: Vec<Task> = Vec::with_capacity(state.items.len());
        for (_leaf, entry) in &state.items {
            let abs_dir = pm_dir.join(&entry.path);
            let claude_md_path = abs_dir.join(CLAUDE_MD);
            if !claude_md_path.exists() {
                eprintln!("state.json references missing {}; skipping.", claude_md_path.display());
                continue;
            }
            let ticket = match Ticket::read(&claude_md_path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Error reading {}: {e}; skipping.", claude_md_path.display());
                    continue;
                }
            };
            let artifact_files = read_artifact_filenames(&abs_dir.join("artifacts"));
            let task = task_from_document(&ticket.front_matter, &ticket.body, artifact_files);
            tasks.push(task);
        }
        Database { tasks, state }
    }

    /// Save the database to a `.pm/` workspace directory.
    ///
    /// For each [`Task`] in `self.tasks`, writes a `CLAUDE.md` to its
    /// addressed path under `pm_dir`. The ticket's `artifacts/` directory is
    /// created if missing; existing artifact files are left untouched.
    /// `state.json` is rewritten so `state.items` reflects the current set of
    /// tasks; `state.next` and `state.tombstones` are preserved across the
    /// save so id allocation history survives.
    pub fn save(&self, pm_dir: &Path) -> std::io::Result<()> {
        let layout = Layout::at(pm_dir);
        layout
            .init()
            .map_err(|e| std::io::Error::other(format!("layout init: {e}")))?;

        // Cloning state lets us rebuild items without losing the next /
        // tombstones counters held on the in-memory db.
        let mut state = self.state.clone();
        state.items.clear();

        let id_index: HashMap<LeafId, usize> = self
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, i))
            .collect();

        for task in &self.tasks {
            let address = match build_address(task, &self.tasks, &id_index) {
                Some(a) => a,
                None => {
                    // A task referencing a missing parent in this Database
                    // gets demoted to an orphan record so the save doesn't
                    // lose it.
                    AddressId::new(vec![task.id]).expect("single-leaf address always valid")
                }
            };
            let rel = layout.directory_for(&address);
            let abs_dir = pm_dir.join(&rel);
            layout
                .ensure_node_path(&rel)
                .map_err(|e| std::io::Error::other(format!("ensure node path: {e}")))?;

            // Build the front-matter + body via the bridge, render via Ticket
            // (which wraps the YAML delimiters and trailing @artifacts import
            // around the body).
            let (fm, body) = task_to_document(task);
            let ticket = Ticket { front_matter: fm, body };
            ticket
                .write_to(&abs_dir)
                .map_err(|e| std::io::Error::other(format!("write CLAUDE.md: {e}")))?;

            // Make sure ARTIFACTS.md reflects whatever is in the artifacts/
            // directory. Hand-curated `desc:` survives via the existing sweep.
            let artifacts_dir = abs_dir.join("artifacts");
            std::fs::create_dir_all(&artifacts_dir)?;
            if let Err(e) = artifacts::sweep_dir(&artifacts_dir, task.id) {
                eprintln!("artifact sweep for {}: {e}", task.id);
            }

            state.items.insert(task.id, ItemEntry { path: rel });
        }

        state
            .save(&layout.state_path())
            .map_err(|e| std::io::Error::other(format!("state.save: {e}")))?;
        Ok(())
    }

    /// Allocate the next monotonic [`LeafId`] for the given type prefix and
    /// return it to the caller. The internal `state` counter is bumped and
    /// any tombstoned numbers are skipped automatically.
    pub fn allocate_id(&mut self, prefix: TypePrefix) -> LeafId {
        self.state.allocate(prefix)
    }

    /// Create an index mapping task ids to their positions in the tasks vector.
    pub fn index(&self) -> HashMap<LeafId, usize> {
        let mut m = HashMap::new();
        for (i, t) in self.tasks.iter().enumerate() {
            m.insert(t.id, i);
        }
        m
    }

    /// Get a task by id.
    pub fn get(&self, id: LeafId) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Get a mutable reference to a task by id.
    pub fn get_mut(&mut self, id: LeafId) -> Option<&mut Task> {
        let idx = self.tasks.iter().position(|t| t.id == id)?;
        self.tasks.get_mut(idx)
    }

    /// Remove tasks by ids and clean up any parent references pointing to removed tasks.
    pub fn remove_ids(&mut self, ids: &HashSet<LeafId>) {
        self.tasks.retain(|t| !ids.contains(&t.id));
        // Also clear parent pointers of any tasks that pointed to removed nodes.
        let removed: BTreeSet<LeafId> = ids.iter().cloned().collect();
        for t in self.tasks.iter_mut() {
            if let Some(p) = t.parent {
                if removed.contains(&p) {
                    t.parent = None;
                }
            }
        }
    }
}

/// Walk a task's parent chain up to the root and return the resulting
/// [`AddressId`]. Returns `None` if any parent reference in the chain is
/// missing from the supplied id index. A cycle is broken at 16 hops; if the
/// chain does not terminate within that bound `None` is returned.
fn build_address(
    task: &Task,
    tasks: &[Task],
    id_index: &HashMap<LeafId, usize>,
) -> Option<AddressId> {
    let mut chain: Vec<LeafId> = Vec::with_capacity(5);
    chain.push(task.id);
    let mut cursor = task.parent;
    for _ in 0..16 {
        match cursor {
            None => break,
            Some(pid) => {
                let idx = *id_index.get(&pid)?;
                let parent = tasks.get(idx)?;
                chain.push(parent.id);
                cursor = parent.parent;
            }
        }
    }
    if cursor.is_some() {
        // Chain did not terminate within the hop guard.
        return None;
    }
    chain.reverse();
    AddressId::new(chain).ok()
}

/// Read the artifact filenames recorded for a ticket. Prefers parsing the
/// ticket's `ARTIFACTS.md` (which carries hand-curated descriptions); falls
/// back to a directory listing for tickets that have never been swept.
fn read_artifact_filenames(artifacts_dir: &Path) -> Vec<String> {
    let index_path = artifacts_dir.join(crate::store::artifacts::ARTIFACTS_MD);
    if index_path.exists() {
        if let Ok(idx) = ArtifactsIndex::load(&index_path) {
            return idx.entries.into_iter().map(|e| e.file).collect();
        }
    }
    if !artifacts_dir.exists() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(artifacts_dir) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if !ft.is_file() {
                    continue;
                }
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == crate::store::artifacts::ARTIFACTS_MD || name.starts_with('.') {
                continue;
            }
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Normalize a tag string by trimming, lowercasing, and replacing spaces with hyphens.
pub fn normalise_tag(s: &str) -> String {
    s.trim().to_lowercase().replace(' ', "-")
}

/// Split comma-separated tag strings and normalize each tag.
pub fn split_and_normalise_tags(inputs: &[String]) -> Vec<String> {
    let mut tags = Vec::new();
    for raw in inputs {
        for part in raw.split(',') {
            let tag = normalise_tag(part);
            if !tag.is_empty() {
                tags.push(tag);
            }
        }
    }
    tags.sort();
    tags.dedup();
    tags
}

/// Parse human-readable due date input with smart natural language support.
///
/// Supports:
/// - "today", "tomorrow", "yesterday"
/// - "next monday", "next tuesday", etc.
/// - "this friday", "this weekend"
/// - "end of week", "end of month"
/// - "in 3d", "in 2w", "in 1m"
/// - "YYYY-MM-DD" format
pub fn parse_due_input(s: &str) -> Option<NaiveDate> {
    let s = s.trim().to_lowercase();
    let today = Local::now().date_naive();

    // Simple cases
    match s.as_str() {
        "today" => return Some(today),
        "tomorrow" => return Some(today + Duration::days(1)),
        "yesterday" => return Some(today - Duration::days(1)),
        "end of week" | "eow" => {
            let (_, end) = start_end_of_this_week(today);
            return Some(end);
        },
        "end of month" | "eom" => {
            // Last day of current month
            let year = today.year();
            let month = today.month();
            let next_month = if month == 12 { 1 } else { month + 1 };
            let next_year = if month == 12 { year + 1 } else { year };
            let first_of_next = NaiveDate::from_ymd_opt(next_year, next_month, 1)?;
            return Some(first_of_next - Duration::days(1));
        },
        "this weekend" | "weekend" => {
            // Coming Saturday
            let days_until_saturday = (6 - today.weekday().num_days_from_monday()) % 7;
            let saturday = today + Duration::days(days_until_saturday as i64);
            return Some(saturday);
        },
        _ => {}
    }

    // "in X" patterns
    if let Some(rest) = s.strip_prefix("in ") {
        if let Some(nd) = rest.strip_suffix("d") {
            if let Ok(days) = nd.trim().parse::<i64>() {
                return Some(today + Duration::days(days));
            }
        }
        if let Some(nw) = rest.strip_suffix("w") {
            if let Ok(weeks) = nw.trim().parse::<i64>() {
                return Some(today + Duration::weeks(weeks));
            }
        }
        if let Some(nm) = rest.strip_suffix("m") {
            if let Ok(months) = nm.trim().parse::<i64>() {
                // Approximate: 30 days per month
                return Some(today + Duration::days(months * 30));
            }
        }
    }

    // Weekday patterns
    let weekdays = [
        ("monday", 0), ("tuesday", 1), ("wednesday", 2), ("thursday", 3),
        ("friday", 4), ("saturday", 5), ("sunday", 6),
        ("mon", 0), ("tue", 1), ("wed", 2), ("thu", 3),
        ("fri", 4), ("sat", 5), ("sun", 6),
    ];

    for (day_name, target_day) in weekdays {
        if s == day_name {
            // This week's occurrence
            let current_day = today.weekday().num_days_from_monday();
            let days_ahead = (target_day + 7 - current_day as i32) % 7;
            let target_date = today + Duration::days(days_ahead as i64);
            return Some(if days_ahead == 0 { today } else { target_date });
        }

        if s == format!("next {}", day_name) {
            // Next week's occurrence
            let current_day = today.weekday().num_days_from_monday();
            let days_ahead = (target_day + 7 - current_day as i32) % 7;
            let days_to_add = if days_ahead == 0 { 7 } else { days_ahead + 7 };
            return Some(today + Duration::days(days_to_add as i64));
        }

        if s == format!("this {}", day_name) {
            // This week's occurrence (same as bare weekday)
            let current_day = today.weekday().num_days_from_monday();
            let days_ahead = (target_day + 7 - current_day as i32) % 7;
            let target_date = today + Duration::days(days_ahead as i64);
            return Some(if days_ahead == 0 { today } else { target_date });
        }
    }

    // Try ISO format
    NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()
}

/// Calculate the start and end dates of the current ISO week (Monday to Sunday).
pub fn start_end_of_this_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    // ISO week: Monday start.
    let weekday = today.weekday().num_days_from_monday() as i64;
    let start = today - Duration::days(weekday);
    let end = start + Duration::days(6);
    (start, end)
}

/// Format a due date relative to today ("today", "tomorrow", "in 3d", "2d late").
pub fn format_due_relative(due: Option<NaiveDate>, today: NaiveDate) -> String {
    match due {
        None => "-".into(),
        Some(d) => {
            let delta = d - today;
            if delta.num_days() == 0 {
                "today".into()
            } else if delta.num_days() == 1 {
                "tomorrow".into()
            } else if delta.num_days() > 1 {
                format!("in {}d", delta.num_days())
            } else {
                format!("{}d late", -delta.num_days())
            }
        }
    }
}

/// Format a task kind for display.
pub fn format_kind(k: Kind) -> &'static str {
    match k {
        Kind::Project => "Project",
        Kind::Product => "Product",
        Kind::Epic => "Epic",
        Kind::Task => "Task",
        Kind::Subtask => "Subtask",
        Kind::Milestone => "Milestone",
    }
}

/// Map the data-layer [`Kind`] to its v2 [`TypePrefix`]. Used wherever a Task
/// needs to be turned into an addressed v2 ticket (id allocation, on-disk
/// directory naming, the Task <-> Document bridge).
pub fn kind_to_prefix(k: Kind) -> TypePrefix {
    match k {
        Kind::Project => TypePrefix::Project,
        Kind::Product => TypePrefix::Product,
        Kind::Epic => TypePrefix::Epic,
        Kind::Task => TypePrefix::Task,
        Kind::Subtask => TypePrefix::Subtask,
        Kind::Milestone => TypePrefix::Milestone,
    }
}

/// Inverse of [`kind_to_prefix`]. A `TypePrefix` carries the canonical kind of
/// its `LeafId`, so the bridge can recover the data-layer `Kind` on read.
pub fn prefix_to_kind(p: TypePrefix) -> Kind {
    match p {
        TypePrefix::Project => Kind::Project,
        TypePrefix::Product => Kind::Product,
        TypePrefix::Epic => Kind::Epic,
        TypePrefix::Task => Kind::Task,
        TypePrefix::Subtask => Kind::Subtask,
        TypePrefix::Milestone => Kind::Milestone,
    }
}

/// Format a priority level for display.
pub fn format_priority(p: Option<Priority>) -> &'static str {
    match p {
        Some(Priority::MustHave) => "Must Have",
        Some(Priority::NiceToHave) => "Nice to Have",
        Some(Priority::CutFirst) => "Cut First",
        None => "-",
    }
}

/// Format an urgency level for display.
pub fn format_urgency(u: Option<Urgency>) -> &'static str {
    match u {
        Some(Urgency::UrgentImportant) => "Urgent Important",
        Some(Urgency::UrgentNotImportant) => "Urgent Not Important",
        Some(Urgency::NotUrgentImportant) => "Not Urgent Important",
        Some(Urgency::NotUrgentNotImportant) => "Not Urgent Not Important",
        None => "-",
    }
}

/// Format a process stage for display.
pub fn format_process_stage(s: Option<ProcessStage>) -> &'static str {
    match s {
        Some(ProcessStage::Ideation) => "Ideation",
        Some(ProcessStage::Design) => "Design",
        Some(ProcessStage::Prototyping) => "Prototyping",
        Some(ProcessStage::ReadyToImplement) => "Ready to Implement",
        Some(ProcessStage::Implementation) => "Implementation",
        Some(ProcessStage::Testing) => "Testing",
        Some(ProcessStage::Refinement) => "Refinement",
        Some(ProcessStage::Release) => "Release",
        None => "-",
    }
}

/// Validate that a parent-child relationship follows the hierarchical rules.
pub fn validate_hierarchy(parent_kind: Kind, child_kind: Kind) -> bool {
    match (parent_kind, child_kind) {
        (Kind::Project, Kind::Product) => true,
        (Kind::Product, Kind::Epic) => true,
        (Kind::Epic, Kind::Task) => true,
        (Kind::Task, Kind::Subtask) => true,
        (Kind::Subtask, Kind::Subtask) => true,
        _ => false,
    }
}

/// Format a task status for display.
pub fn format_status(s: Status) -> &'static str {
    match s {
        Status::Open => "Open",
        Status::InProgress => "InProgress",
        Status::Done => "Done",
    }
}

/// Walk the parent chain from `task` and return the first ancestor whose
/// kind is `Kind::Project`. Returns `None` if no Project ancestor exists
/// (orphan task, or a parent reference that does not resolve in this db).
pub fn project_ancestor<'a>(db: &'a Database, task: &Task) -> Option<&'a Task> {
    let mut cur = task.parent;
    let mut guard = 0usize;
    while let Some(pid) = cur {
        guard += 1;
        if guard > 64 { return None; } // cycle/depth guard
        let parent = db.get(pid)?;
        if parent.kind == Kind::Project {
            return Some(parent);
        }
        cur = parent.parent;
    }
    None
}

/// Human-readable project label for a task: the project ancestor's title, or
/// `"-"` when none is found.
pub fn project_label(db: &Database, task: &Task) -> String {
    project_ancestor(db, task)
        .map(|p| p.title.clone())
        .unwrap_or_else(|| "-".to_string())
}

/// Print tasks in a formatted table with optional tree indentation. The
/// `Project` column is derived from each task's parent chain via
/// [`project_label`]; the `Task` struct no longer carries a free-form label.
pub fn print_table(db: &Database, tasks: &[&Task], id_to_depth: Option<&HashMap<LeafId, usize>>) {
    // Header.
    println!(
        "{:<8} {:<10} {:<11} {:<6} {:<12} {:<14} {}",
        "ID", "Kind", "Status", "Pri", "Due", "Project", "Title [tags]"
    );
    let today = Local::now().date_naive();
    for t in tasks {
        let indent = id_to_depth
            .and_then(|m| m.get(&t.id).copied())
            .unwrap_or(0);
        let indent_str = "  ".repeat(indent);
        let tags = if t.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", t.tags.join(","))
        };
        let due = format_due_relative(t.due, today);
        let project = project_label(db, t);
        println!(
            "{:<8} {:<10} {:<11} {:<12} {:<14} {}{}{}",
            t.id.to_string(),
            format_kind(t.kind),
            format_status(t.status),
            due,
            truncate(&project, 14),
            indent_str,
            t.title,
            tags
        );
    }
}

/// Truncate a string to a maximum width, adding ellipsis if needed.
pub fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i + 1 >= width {
                out.push('…');
                break;
            }
            out.push(ch);
        }
        out
    }
}

/// Build a map of parent task ids to their children's ids.
pub fn build_children_map(tasks: &[Task]) -> BTreeMap<LeafId, Vec<LeafId>> {
    let mut map: BTreeMap<LeafId, Vec<LeafId>> = BTreeMap::new();
    for t in tasks {
        if let Some(p) = t.parent {
            map.entry(p).or_default().push(t.id);
        }
    }
    for v in map.values_mut() {
        v.sort_unstable();
    }
    map
}

/// Recursively collect all descendant task ids from a root task.
pub fn collect_descendants(root: LeafId, child_map: &BTreeMap<LeafId, Vec<LeafId>>, out: &mut HashSet<LeafId>) {
    if let Some(children) = child_map.get(&root) {
        for &c in children {
            if out.insert(c) {
                collect_descendants(c, child_map, out);
            }
        }
    }
}

/// Collect all ancestor task ids by following parent references.
pub fn collect_ancestors(mut id: LeafId, db: &Database) -> Vec<LeafId> {
    let index = db.index();
    let mut chain = Vec::new();
    while let Some(t) = index.get(&id).and_then(|&i| db.tasks.get(i)) {
        if let Some(p) = t.parent {
            chain.push(p);
            id = p;
        } else {
            break;
        }
    }
    chain
}

/// Resolve a task identifier to a [`LeafId`].
///
/// Accepts:
/// - Address-form ids (`TSK7`, `PRJ1-PRD1-EPC3-TSK7`, `TSK7-some-label`) -
///   parsed via [`IdInput`] and reduced to the terminal leaf.
/// - Exact title match - case-insensitive comparison against `task.title`.
///
/// Reports a clear error on no-match, an unknown-leaf match, or a multi-title
/// collision (with the ambiguous ids listed so the caller can disambiguate).
pub fn resolve_task_identifier(identifier: &str, db: &Database) -> Result<LeafId, String> {
    // Try parsing as a typed id first.
    if let Ok(input) = identifier.parse::<IdInput>() {
        let leaf = input.leaf();
        if db.get(leaf).is_some() {
            return Ok(leaf);
        }
        return Err(format!("Task with id {} not found", leaf));
    }

    // Search by title (case-insensitive).
    let matches: Vec<&Task> = db.tasks
        .iter()
        .filter(|task| task.title.to_lowercase() == identifier.to_lowercase())
        .collect();

    match matches.len() {
        0 => Err(format!("No task found with name '{}'", identifier)),
        1 => Ok(matches[0].id),
        _ => {
            let mut error_msg = format!("Multiple tasks found with name '{}':\n", identifier);
            for task in matches {
                error_msg.push_str(&format!("  {}: {} ({})",
                    task.id,
                    task.title,
                    format_kind(task.kind)
                ));
                let project = project_label(db, task);
                if project != "-" {
                    error_msg.push_str(&format!(" [project: {}]", project));
                }
                error_msg.push('\n');
            }
            error_msg.push_str("Please use the typed id instead.");
            Err(error_msg)
        }
    }
}

/// Parse a kind string from CSV format.
pub fn parse_kind(s: &str) -> Kind {
    match s.to_lowercase().as_str() {
        "project" => Kind::Project,
        "product" => Kind::Product,
        "epic" => Kind::Epic,
        "task" => Kind::Task,
        "subtask" => Kind::Subtask,
        "milestone" => Kind::Milestone,
        _ => Kind::Task, // Default fallback
    }
}

/// Parse a status string from CSV format.
pub fn parse_status(s: &str) -> Status {
    match s.to_lowercase().as_str() {
        "open" => Status::Open,
        "in-progress" => Status::InProgress,
        "done" => Status::Done,
        _ => Status::Open, // Default fallback
    }
}

/// Parse a priority string from CSV format.
pub fn parse_priority(s: &str) -> Option<Priority> {
    if s == "-" {
        return None;
    }
    match s.to_lowercase().as_str() {
        "must-have" => Some(Priority::MustHave),
        "nice-to-have" => Some(Priority::NiceToHave),
        "cut-first" => Some(Priority::CutFirst),
        _ => None,
    }
}

/// Parse an urgency string from CSV format.
pub fn parse_urgency(s: &str) -> Option<Urgency> {
    if s == "-" {
        return None;
    }
    match s.to_lowercase().as_str() {
        "urgent-important" => Some(Urgency::UrgentImportant),
        "urgent-not-important" => Some(Urgency::UrgentNotImportant),
        "not-urgent-important" => Some(Urgency::NotUrgentImportant),
        "not-urgent-not-important" => Some(Urgency::NotUrgentNotImportant),
        _ => None,
    }
}

/// Parse a process stage string from CSV format.
pub fn parse_process_stage(s: &str) -> Option<ProcessStage> {
    if s == "-" {
        return None;
    }
    match s.to_lowercase().as_str() {
        "ideation" => Some(ProcessStage::Ideation),
        "design" => Some(ProcessStage::Design),
        "prototyping" => Some(ProcessStage::Prototyping),
        "implementation" => Some(ProcessStage::Implementation),
        "testing" => Some(ProcessStage::Testing),
        "refinement" => Some(ProcessStage::Refinement),
        "release" => Some(ProcessStage::Release),
        _ => None,
    }
}
