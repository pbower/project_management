//! Form field model for the ticket editor.
//!
//! Short fields are text inputs; the user types straight in. Enum
//! fields cycle through their variants via Left/Right. The form holds
//! a single cursor that walks all rows top-to-bottom and a flag for
//! whether the cursor is in "typing" mode for the focused text field.

use std::fmt;

use chrono::NaiveDate;
use crossterm::event::KeyCode;

use crate::db::Database;
use crate::fields::{Kind, Priority, ProcessStage, Status, Urgency};
use crate::store::LeafId;
use crate::task::Task;

/// Which form field is currently focused. The order here is also the
/// rendering order in the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Title,
    Summary,
    Kind,
    Status,
    Priority,
    Urgency,
    ProcessStage,
    Tags,
    Due,
    Parent,
    Milestone,
    IssueLink,
    PrLink,
}

impl FieldId {
    pub const ALL: [FieldId; 13] = [
        FieldId::Title,
        FieldId::Summary,
        FieldId::Kind,
        FieldId::Status,
        FieldId::Priority,
        FieldId::Urgency,
        FieldId::ProcessStage,
        FieldId::Tags,
        FieldId::Due,
        FieldId::Parent,
        FieldId::Milestone,
        FieldId::IssueLink,
        FieldId::PrLink,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FieldId::Title => "Title",
            FieldId::Summary => "Summary",
            FieldId::Kind => "Kind",
            FieldId::Status => "Status",
            FieldId::Priority => "Priority",
            FieldId::Urgency => "Urgency",
            FieldId::ProcessStage => "Process",
            FieldId::Tags => "Tags",
            FieldId::Due => "Due",
            FieldId::Parent => "Parent",
            FieldId::Milestone => "Milestone",
            FieldId::IssueLink => "Issue link",
            FieldId::PrLink => "PR link",
        }
    }

    pub fn is_enum(self) -> bool {
        matches!(
            self,
            FieldId::Kind
                | FieldId::Status
                | FieldId::Priority
                | FieldId::Urgency
                | FieldId::ProcessStage
        )
    }
}

/// The pending form state. Each enum field is `Option<T>` because some
/// of them are nullable in the task model (Priority, Urgency, etc.);
/// others (Kind, Status) always have a value but we keep `Option` for
/// uniform display.
#[derive(Debug, Clone)]
pub struct FormState {
    pub title: String,
    pub summary: String,
    pub kind: Kind,
    pub status: Status,
    pub priority: Option<Priority>,
    pub urgency: Option<Urgency>,
    pub process_stage: Option<ProcessStage>,
    pub tags: String, // comma-separated; parsed on save
    pub due: String,  // YYYY-MM-DD or human input; parsed on save
    pub parent: String,
    pub milestone: String,
    pub issue_link: String,
    pub pr_link: String,
}

impl FormState {
    /// Build the form state from the current task in the database.
    pub fn from_task(task: &Task) -> Self {
        FormState {
            title: task.title.clone(),
            summary: task.summary.clone().unwrap_or_default(),
            kind: task.kind,
            status: task.status,
            priority: task.priority_level,
            urgency: task.urgency,
            process_stage: task.process_stage,
            tags: task.tags.join(", "),
            due: task
                .due
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            parent: task.parent.map(|p| p.to_string()).unwrap_or_default(),
            milestone: task.milestone.map(|m| m.to_string()).unwrap_or_default(),
            issue_link: task.issue_link.clone().unwrap_or_default(),
            pr_link: task.pr_link.clone().unwrap_or_default(),
        }
    }

    /// Compute the diff between this form state and the task as it
    /// currently stands in the database. Used by the editor's save
    /// path to decide which CLI verbs to call.
    pub fn diff(&self, task: &Task) -> Vec<FieldChange> {
        let mut changes: Vec<FieldChange> = Vec::new();
        if self.title != task.title {
            changes.push(FieldChange::Title(self.title.clone()));
        }
        if normalise_opt_string(&self.summary) != task.summary {
            changes.push(FieldChange::Summary(opt_string(&self.summary)));
        }
        if self.kind != task.kind {
            changes.push(FieldChange::Kind(self.kind));
        }
        if self.status != task.status {
            changes.push(FieldChange::Status(self.status));
        }
        if self.priority != task.priority_level {
            changes.push(FieldChange::Priority(self.priority));
        }
        if self.urgency != task.urgency {
            changes.push(FieldChange::Urgency(self.urgency));
        }
        if self.process_stage != task.process_stage {
            changes.push(FieldChange::ProcessStage(self.process_stage));
        }
        let new_tags: Vec<String> = self
            .tags
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if new_tags != task.tags {
            changes.push(FieldChange::Tags(new_tags));
        }
        let new_due = parse_due(&self.due);
        if new_due != task.due {
            changes.push(FieldChange::Due(new_due));
        }
        if normalise_opt_string(&self.issue_link) != task.issue_link {
            changes.push(FieldChange::IssueLink(opt_string(&self.issue_link)));
        }
        if normalise_opt_string(&self.pr_link) != task.pr_link {
            changes.push(FieldChange::PrLink(opt_string(&self.pr_link)));
        }
        // Parent and Milestone changes need an id lookup; parse here
        // so save can apply directly. Empty string means "clear".
        let new_parent = if self.parent.trim().is_empty() {
            None
        } else {
            self.parent.trim().parse::<LeafId>().ok()
        };
        if new_parent != task.parent {
            changes.push(FieldChange::Parent(new_parent));
        }
        let new_milestone = if self.milestone.trim().is_empty() {
            None
        } else {
            self.milestone.trim().parse::<LeafId>().ok()
        };
        if new_milestone != task.milestone {
            changes.push(FieldChange::Milestone(new_milestone));
        }
        changes
    }
}

/// A single field-level change the save path will apply.
#[derive(Debug, Clone)]
pub enum FieldChange {
    Title(String),
    Summary(Option<String>),
    Kind(Kind),
    Status(Status),
    Priority(Option<Priority>),
    Urgency(Option<Urgency>),
    ProcessStage(Option<ProcessStage>),
    Tags(Vec<String>),
    Due(Option<NaiveDate>),
    Parent(Option<LeafId>),
    Milestone(Option<LeafId>),
    IssueLink(Option<String>),
    PrLink(Option<String>),
}

impl FieldChange {
    /// One-word label used in commit messages and events.log entries.
    pub fn label(&self) -> &'static str {
        match self {
            FieldChange::Title(_) => "title",
            FieldChange::Summary(_) => "summary",
            FieldChange::Kind(_) => "kind",
            FieldChange::Status(_) => "status",
            FieldChange::Priority(_) => "priority",
            FieldChange::Urgency(_) => "urgency",
            FieldChange::ProcessStage(_) => "process",
            FieldChange::Tags(_) => "tags",
            FieldChange::Due(_) => "due",
            FieldChange::Parent(_) => "parent",
            FieldChange::Milestone(_) => "milestone",
            FieldChange::IssueLink(_) => "issue-link",
            FieldChange::PrLink(_) => "pr-link",
        }
    }
}

// ---------------------------------------------------------------------------
// Field-level key handling
// ---------------------------------------------------------------------------

/// Mutation outcome of a key press on a field. Most keys produce
/// `Stayed`; `MovedNext` and `MovedPrev` exit typing mode and ask the
/// caller to walk to the next/previous row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKeyOutcome {
    Stayed,
    EnterTyping,
    ExitTyping,
}

/// Apply a key press to the focused field while in typing mode. Used
/// by the editor when it has entered text-input mode on a text field.
pub fn apply_text_keystroke(buf: &mut String, key: KeyCode) -> FieldKeyOutcome {
    match key {
        KeyCode::Char(c) => {
            buf.push(c);
            FieldKeyOutcome::Stayed
        }
        KeyCode::Backspace => {
            buf.pop();
            FieldKeyOutcome::Stayed
        }
        KeyCode::Esc | KeyCode::Enter => FieldKeyOutcome::ExitTyping,
        _ => FieldKeyOutcome::Stayed,
    }
}

/// Cycle an enum value in the requested direction (`+1` or `-1`).
/// Wraps at both ends so the user can always reach every variant.
pub fn cycle_kind(current: Kind, delta: i32) -> Kind {
    cycle_variant(
        current,
        delta,
        &[
            Kind::Project,
            Kind::Product,
            Kind::Epic,
            Kind::Task,
            Kind::Subtask,
            Kind::Milestone,
        ],
    )
}

pub fn cycle_status(current: Status, delta: i32) -> Status {
    cycle_variant(
        current,
        delta,
        &[Status::Open, Status::InProgress, Status::Done],
    )
}

pub fn cycle_priority(current: Option<Priority>, delta: i32) -> Option<Priority> {
    let variants = [
        None,
        Some(Priority::MustHave),
        Some(Priority::NiceToHave),
        Some(Priority::CutFirst),
    ];
    cycle_variant(current, delta, &variants)
}

pub fn cycle_urgency(current: Option<Urgency>, delta: i32) -> Option<Urgency> {
    let variants = [
        None,
        Some(Urgency::UrgentImportant),
        Some(Urgency::UrgentNotImportant),
        Some(Urgency::NotUrgentImportant),
        Some(Urgency::NotUrgentNotImportant),
    ];
    cycle_variant(current, delta, &variants)
}

pub fn cycle_process_stage(current: Option<ProcessStage>, delta: i32) -> Option<ProcessStage> {
    let variants = [
        None,
        Some(ProcessStage::Ideation),
        Some(ProcessStage::Design),
        Some(ProcessStage::Prototyping),
        Some(ProcessStage::ReadyToImplement),
        Some(ProcessStage::Implementation),
        Some(ProcessStage::Testing),
        Some(ProcessStage::Refinement),
        Some(ProcessStage::Release),
    ];
    cycle_variant(current, delta, &variants)
}

fn cycle_variant<T: PartialEq + Copy>(current: T, delta: i32, variants: &[T]) -> T {
    let len = variants.len() as i32;
    let idx = variants.iter().position(|v| *v == current).unwrap_or(0) as i32;
    let next = ((idx + delta).rem_euclid(len)) as usize;
    variants[next]
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

pub fn display_priority(p: Option<Priority>) -> String {
    p.map(|v| {
        match v {
            Priority::MustHave => "must-have",
            Priority::NiceToHave => "nice-to-have",
            Priority::CutFirst => "cut-first",
        }
        .to_string()
    })
    .unwrap_or_else(|| "-".to_string())
}

pub fn display_urgency(u: Option<Urgency>) -> String {
    u.map(|v| {
        match v {
            Urgency::UrgentImportant => "urgent / important",
            Urgency::UrgentNotImportant => "urgent / not-important",
            Urgency::NotUrgentImportant => "not-urgent / important",
            Urgency::NotUrgentNotImportant => "not-urgent / not-important",
        }
        .to_string()
    })
    .unwrap_or_else(|| "-".to_string())
}

pub fn display_process_stage(s: Option<ProcessStage>) -> String {
    s.map(|v| {
        match v {
            ProcessStage::Ideation => "ideation",
            ProcessStage::Design => "design",
            ProcessStage::Prototyping => "prototyping",
            ProcessStage::ReadyToImplement => "ready-to-implement",
            ProcessStage::Implementation => "implementation",
            ProcessStage::Testing => "testing",
            ProcessStage::Refinement => "refinement",
            ProcessStage::Release => "release",
        }
        .to_string()
    })
    .unwrap_or_else(|| "-".to_string())
}

pub fn display_status(s: Status) -> String {
    match s {
        Status::Open => "open",
        Status::InProgress => "in-progress",
        Status::Done => "done",
    }
    .to_string()
}

pub fn display_kind(k: Kind) -> String {
    match k {
        Kind::Project => "PROJECT",
        Kind::Product => "PRODUCT",
        Kind::Epic => "EPIC",
        Kind::Task => "TASK",
        Kind::Subtask => "SUBTASK",
        Kind::Milestone => "MILESTONE",
    }
    .to_string()
}

impl fmt::Display for FieldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn opt_string(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn normalise_opt_string(s: &str) -> Option<String> {
    opt_string(s)
}

fn parse_due(s: &str) -> Option<NaiveDate> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(t, "%Y-%m-%d").ok()
}

/// Lookup helper for the editor: confirm the parsed parent / milestone
/// id actually exists in the database. Returns `Ok` if valid or there
/// is no value; `Err(message)` if the id was supplied but unknown.
pub fn validate_ref(
    db: &Database,
    field: &str,
    raw: &str,
    expected_kind: Option<Kind>,
) -> Result<(), String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(());
    }
    let leaf: LeafId = t
        .parse()
        .map_err(|_| format!("{field}: not a valid id: {t}"))?;
    let task = db
        .get(leaf)
        .ok_or_else(|| format!("{field}: id {leaf} not found"))?;
    if let Some(k) = expected_kind {
        if task.kind != k {
            return Err(format!(
                "{field}: {leaf} is a {:?}, expected {k:?}",
                task.kind
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_cycles_in_both_directions() {
        let a = cycle_status(Status::Open, 1);
        assert_eq!(a, Status::InProgress);
        let b = cycle_status(Status::Done, 1);
        assert_eq!(b, Status::Open);
        let c = cycle_status(Status::Open, -1);
        assert_eq!(c, Status::Done);
    }

    #[test]
    fn priority_cycle_includes_none() {
        let states = [
            cycle_priority(None, 1),
            cycle_priority(Some(Priority::MustHave), 1),
            cycle_priority(Some(Priority::NiceToHave), 1),
            cycle_priority(Some(Priority::CutFirst), 1),
        ];
        assert_eq!(states[0], Some(Priority::MustHave));
        assert_eq!(states[3], None);
    }

    #[test]
    fn typing_appends_and_backspaces() {
        let mut buf = "hel".to_string();
        apply_text_keystroke(&mut buf, KeyCode::Char('l'));
        apply_text_keystroke(&mut buf, KeyCode::Char('o'));
        assert_eq!(buf, "hello");
        apply_text_keystroke(&mut buf, KeyCode::Backspace);
        assert_eq!(buf, "hell");
        assert_eq!(
            apply_text_keystroke(&mut buf, KeyCode::Enter),
            FieldKeyOutcome::ExitTyping
        );
    }
}
