//! Database operations and utility functions for task management.
//!
//! This module provides the `Database` struct for storing and managing tasks,
//! along with various utility functions for date parsing, formatting, validation,
//! and hierarchical operations.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::fields::*;
use crate::task::{Task, TaskTemplate};

/// In-memory database for storing and managing tasks.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Database {
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub templates: Vec<TaskTemplate>,
}

impl Database {
    /// Load database from JSON file, creating a new empty database if file doesn't exist.
    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Database::default();
        }
        let mut buf = String::new();
        match File::open(path).and_then(|mut f| f.read_to_string(&mut buf)) {
            Ok(_) => match serde_json::from_str(&buf) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("Error parsing DB, starting fresh: {e}");
                    Database::default()
                }
            },
            Err(e) => {
                eprintln!("Error reading DB, starting fresh: {e}");
                Database::default()
            }
        }
    }

    /// Save database to JSON file using atomic write (temp file + rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // Atomic-ish write via temp + rename.
        let tmp = path.with_extension("json.tmp");
        let mut f = File::create(&tmp)?;
        let data = serde_json::to_string_pretty(self).unwrap();
        f.write_all(data.as_bytes())?;
        f.flush()?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    /// Generate the next available task ID.
    pub fn next_id(&self) -> u64 {
        self.tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    /// Create an index mapping task IDs to their positions in the tasks vector.
    pub fn index(&self) -> HashMap<u64, usize> {
        let mut m = HashMap::new();
        for (i, t) in self.tasks.iter().enumerate() {
            m.insert(t.id, i);
        }
        m
    }

    /// Get a task by ID.
    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Get a mutable reference to a task by ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Task> {
        let idx = self.tasks.iter().position(|t| t.id == id)?;
        self.tasks.get_mut(idx)
    }

    /// Remove tasks by IDs and clean up any parent references pointing to removed tasks.
    pub fn remove_ids(&mut self, ids: &HashSet<u64>) {
        self.tasks.retain(|t| !ids.contains(&t.id));
        // Also clear parent pointers of any tasks that pointed to removed nodes.
        let removed: BTreeSet<u64> = ids.iter().cloned().collect();
        for t in self.tasks.iter_mut() {
            if let Some(p) = t.parent {
                if removed.contains(&p) {
                    t.parent = None;
                }
            }
        }
    }
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
        Kind::Product => "Product",
        Kind::Epic => "Epic",
        Kind::Task => "Task",
        Kind::Subtask => "Subtask",
        Kind::Milestone => "Milestone",
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

/// Print tasks in a formatted table with optional tree indentation.
pub fn print_table(tasks: &[&Task], id_to_depth: Option<&HashMap<u64, usize>>) {
    // Header.
    println!(
        "{:<5} {:<10} {:<11} {:<6} {:<12} {:<14} {}",
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
        let project = t.project.clone().unwrap_or_else(|| "-".into());
        println!(
            "{:<5} {:<10} {:<11} {:<12} {:<14} {}{}{}",
            t.id,
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

/// Build a map of parent task IDs to their children's IDs.
pub fn build_children_map(tasks: &[Task]) -> BTreeMap<u64, Vec<u64>> {
    let mut map: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
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

/// Recursively collect all descendant task IDs from a root task.
pub fn collect_descendants(root: u64, child_map: &BTreeMap<u64, Vec<u64>>, out: &mut HashSet<u64>) {
    if let Some(children) = child_map.get(&root) {
        for &c in children {
            if out.insert(c) {
                collect_descendants(c, child_map, out);
            }
        }
    }
}

/// Collect all ancestor task IDs by following parent references.
pub fn collect_ancestors(mut id: u64, db: &Database) -> Vec<u64> {
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

/// Resolve a task identifier (either ID or name) to a task ID.
/// Returns an error if the name has multiple matches and suggests using ID instead.
pub fn resolve_task_identifier(identifier: &str, db: &Database) -> Result<u64, String> {
    // Try parsing as ID first
    if let Ok(id) = identifier.parse::<u64>() {
        if db.get(id).is_some() {
            return Ok(id);
        } else {
            return Err(format!("Task with ID {} not found", id));
        }
    }
    
    // Search by name (case-insensitive)
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
                error_msg.push_str(&format!("  ID {}: {} ({})", 
                    task.id, 
                    task.title, 
                    format_kind(task.kind)
                ));
                if let Some(ref project) = task.project {
                    error_msg.push_str(&format!(" [project: {}]", project));
                }
                error_msg.push('\n');
            }
            error_msg.push_str("Please use the specific ID instead.");
            Err(error_msg)
        }
    }
}

/// Parse a kind string from CSV format.
pub fn parse_kind(s: &str) -> Kind {
    match s.to_lowercase().as_str() {
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
