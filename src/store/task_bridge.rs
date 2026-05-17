//! Bridge between the data-layer [`Task`] and the v2 on-disk shape
//! `(FrontMatter, ParsedBody)`. Pure conversion functions with no I/O.
//!
//! On write, a `Task` becomes:
//! - A [`FrontMatter`] populated from the task's metadata fields. The `id`,
//!   `parent`, `status`, `priority`, `urgency`, `process_stage`, `due`,
//!   `tags`, `created`, and `updated` fields map directly. `issue_link` and
//!   `pr_link` go into the `links` map under the keys `"issue"` and `"pr"`.
//! - A [`ParsedBody`] whose sections carry the task's prose fields: `# Summary`
//!   for `summary`, `# Description` for `description`, `# User Story` for
//!   `user_story`, `# Requirements` for `requirements`. Empty/`None` fields
//!   simply do not produce a section.
//!
//! On read, the reverse: front-matter fields populate the matching `Task`
//! scalars; named sections in the body populate the prose fields.
//!
//! `Task.artifacts` is not carried through `FrontMatter` or the body. It
//! reflects the contents of the ticket's `artifacts/` directory, which is
//! managed by [`crate::store::artifacts`]. Callers pass the artifact filename
//! list explicitly into [`task_from_document`].

use chrono::{DateTime, TimeZone, Utc};

use crate::db::{kind_to_prefix, prefix_to_kind, Database};
use crate::fields::Kind;
use crate::task::Task;

use super::front_matter::FrontMatter;
use super::id::LeafId;
use super::sections::ParsedBody;

/// Section names used by the bridge when reading and writing task prose
/// fields. Defined as constants so the bridge and any caller that wants to
/// inspect the body stay in sync on spelling.
pub const SECTION_SUMMARY: &str = "Summary";
pub const SECTION_DESCRIPTION: &str = "Description";
pub const SECTION_USER_STORY: &str = "User Story";
pub const SECTION_REQUIREMENTS: &str = "Requirements";

/// Convert a `Task` into a `(FrontMatter, ParsedBody)` pair ready to be
/// serialised into a `CLAUDE.md` file.
pub fn task_to_document(task: &Task) -> (FrontMatter, ParsedBody) {
    // Sanity: the LeafId's prefix must match the task's kind. Construction
    // paths feed allocate_id(kind_to_prefix(kind)) for new tasks, and the
    // round-trip preserves the relationship for read-back; the bridge does
    // not enforce it here so a hand-edited file with a mismatched id/kind
    // pair surfaces the mismatch loudly via prefix_to_kind on the read side.
    let _expected_prefix = kind_to_prefix(task.kind);

    let mut fm = FrontMatter::new(task.id, task.title.clone());
    fm.parent = task.parent;
    fm.status = task.status;
    fm.priority = task.priority_level;
    fm.urgency = task.urgency;
    fm.process_stage = task.process_stage;
    fm.due = task.due;
    fm.tags = task.tags.clone();
    fm.deps = task.deps.clone();
    fm.milestone = task.milestone;
    fm.memories = task.memories.clone();
    if let Some(link) = task.issue_link.as_ref() {
        fm.links.insert("issue".to_string(), link.clone());
    }
    if let Some(link) = task.pr_link.as_ref() {
        fm.links.insert("pr".to_string(), link.clone());
    }
    fm.created = unix_to_utc(task.created_at_utc);
    fm.updated = unix_to_utc(task.updated_at_utc);

    let mut body = ParsedBody::default();
    if let Some(text) = trim_to_optional(task.summary.as_deref()) {
        body.upsert(SECTION_SUMMARY, ensure_trailing_newline(text));
    }
    if let Some(text) = trim_to_optional(task.description.as_deref()) {
        body.upsert(SECTION_DESCRIPTION, ensure_trailing_newline(text));
    }
    if let Some(text) = trim_to_optional(task.user_story.as_deref()) {
        body.upsert(SECTION_USER_STORY, ensure_trailing_newline(text));
    }
    if let Some(text) = trim_to_optional(task.requirements.as_deref()) {
        body.upsert(SECTION_REQUIREMENTS, ensure_trailing_newline(text));
    }

    (fm, body)
}

/// Convert a `(FrontMatter, ParsedBody)` pair back into a `Task`.
///
/// `artifacts` is the list of file names the caller observed in the ticket's
/// `artifacts/` directory. The bridge does not touch disk; the caller is
/// expected to scan that directory (or read `ARTIFACTS.md`) and supply the
/// list here. Pass an empty `Vec` for an artifact-free ticket.
pub fn task_from_document(fm: &FrontMatter, body: &ParsedBody, artifacts: Vec<String>) -> Task {
    Task {
        id: fm.id,
        title: fm.title.clone(),
        summary: body.find(SECTION_SUMMARY).and_then(section_to_optional),
        description: body.find(SECTION_DESCRIPTION).and_then(section_to_optional),
        user_story: body.find(SECTION_USER_STORY).and_then(section_to_optional),
        requirements: body
            .find(SECTION_REQUIREMENTS)
            .and_then(section_to_optional),
        tags: fm.tags.clone(),
        deps: fm.deps.clone(),
        milestone: fm.milestone,
        memories: fm.memories.clone(),
        due: fm.due,
        parent: fm.parent,
        kind: prefix_to_kind(fm.id.prefix()),
        status: fm.status,
        priority_level: fm.priority,
        urgency: fm.urgency,
        process_stage: fm.process_stage,
        issue_link: fm.links.get("issue").cloned(),
        pr_link: fm.links.get("pr").cloned(),
        artifacts,
        created_at_utc: fm.created.timestamp(),
        updated_at_utc: fm.updated.timestamp(),
    }
}

/// Walk a task's parent chain in `db` looking for the [`Kind::Project`]
/// ancestor. Returns the project's `LeafId` if one exists in the chain,
/// otherwise `None`. The walk caps at 64 hops as a guard against malformed
/// data forming a cycle.
pub fn project_ancestor(db: &Database, task: &Task) -> Option<LeafId> {
    let mut current = task.parent?;
    for _ in 0..64 {
        let parent = db.get(current)?;
        if parent.kind == Kind::Project {
            return Some(parent.id);
        }
        match parent.parent {
            Some(next) => current = next,
            None => return None,
        }
    }
    None
}

fn unix_to_utc(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now)
}

/// Return `Some(trimmed_string)` if the input has non-whitespace content,
/// `None` otherwise. Treats `Some("")` and `Some("   ")` the same as `None`
/// so empty form fields do not produce empty sections on disk.
fn trim_to_optional(input: Option<&str>) -> Option<String> {
    let text = input?;
    let trimmed = text.trim_end_matches('\n');
    if trimmed.trim().is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Convert a parsed section body into the equivalent `Option<String>` for the
/// `Task` struct's prose fields. Empty/whitespace-only bodies become `None`.
fn section_to_optional(section: &super::sections::Section) -> Option<String> {
    let trimmed = section.body.trim_end_matches('\n');
    if trimmed.trim().is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Ensure `s` ends with exactly one trailing newline. `ParsedBody`'s
/// `close_section` strips multiple trailing blanks; matching that contract
/// keeps round-trips idempotent.
fn ensure_trailing_newline(mut s: String) -> String {
    while s.ends_with('\n') {
        s.pop();
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::{Priority, ProcessStage, Status, Urgency};
    use crate::store::id::TypePrefix;
    use crate::store::MemoryRef;
    use chrono::NaiveDate;

    fn sample_full_task() -> Task {
        let id = LeafId::new(TypePrefix::Task, 7);
        Task {
            id,
            title: "Lock protocol with TTL and heartbeat".to_string(),
            summary: Some("Heartbeat-driven lock release".to_string()),
            description: Some("Each agent gets an exclusive lock per ticket.".to_string()),
            user_story: Some("As an agent, I want my lock to auto-release on crash.".to_string()),
            requirements: Some("- TTL: 60s\n- Cleanup runs on pm doctor".to_string()),
            tags: vec!["infra".to_string(), "locking".to_string()],
            deps: vec![
                LeafId::new(TypePrefix::Task, 6),
                LeafId::new(TypePrefix::Task, 11),
            ],
            milestone: Some(LeafId::new(TypePrefix::Milestone, 1)),
            memories: vec![
                MemoryRef::User("feedback-testing".to_string()),
                MemoryRef::Project("auth-stack-conventions".to_string()),
            ],
            due: NaiveDate::from_ymd_opt(2026, 5, 25),
            parent: Some(LeafId::new(TypePrefix::Epic, 3)),
            kind: Kind::Task,
            status: Status::InProgress,
            priority_level: Some(Priority::MustHave),
            urgency: Some(Urgency::UrgentImportant),
            process_stage: Some(ProcessStage::Implementation),
            issue_link: Some("pbower/project_management#42".to_string()),
            pr_link: Some("pbower/project_management#43".to_string()),
            artifacts: vec!["schema.png".to_string(), "bench.csv".to_string()],
            created_at_utc: 1_715_900_000,
            updated_at_utc: 1_715_910_000,
        }
    }

    #[test]
    fn round_trip_preserves_every_field_on_a_full_task() {
        let original = sample_full_task();
        let (fm, body) = task_to_document(&original);
        let back = task_from_document(&fm, &body, original.artifacts.clone());

        assert_eq!(back.id, original.id);
        assert_eq!(back.title, original.title);
        assert_eq!(back.summary, original.summary);
        assert_eq!(back.description, original.description);
        assert_eq!(back.user_story, original.user_story);
        assert_eq!(back.requirements, original.requirements);
        assert_eq!(back.tags, original.tags);
        assert_eq!(back.deps, original.deps);
        assert_eq!(back.milestone, original.milestone);
        assert_eq!(back.memories, original.memories);
        assert_eq!(back.due, original.due);
        assert_eq!(back.parent, original.parent);
        assert_eq!(back.kind, original.kind);
        assert_eq!(back.status, original.status);
        assert_eq!(back.priority_level, original.priority_level);
        assert_eq!(back.urgency, original.urgency);
        assert_eq!(back.process_stage, original.process_stage);
        assert_eq!(back.issue_link, original.issue_link);
        assert_eq!(back.pr_link, original.pr_link);
        assert_eq!(back.artifacts, original.artifacts);
        assert_eq!(back.created_at_utc, original.created_at_utc);
        assert_eq!(back.updated_at_utc, original.updated_at_utc);
    }

    #[test]
    fn round_trip_preserves_minimal_task() {
        let id = LeafId::new(TypePrefix::Task, 1);
        let original = Task {
            id,
            title: "Bare task".to_string(),
            summary: None,
            description: None,
            user_story: None,
            requirements: None,
            tags: Vec::new(),
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
            due: None,
            parent: None,
            kind: Kind::Task,
            status: Status::Open,
            priority_level: None,
            urgency: None,
            process_stage: None,
            issue_link: None,
            pr_link: None,
            artifacts: Vec::new(),
            created_at_utc: 0,
            updated_at_utc: 0,
        };
        let (fm, body) = task_to_document(&original);
        let back = task_from_document(&fm, &body, Vec::new());

        assert_eq!(back.title, "Bare task");
        assert!(back.summary.is_none());
        assert!(back.description.is_none());
        assert!(back.user_story.is_none());
        assert!(back.requirements.is_none());
        assert!(back.tags.is_empty());
        assert!(back.parent.is_none());
        assert_eq!(back.kind, Kind::Task);
        assert!(back.priority_level.is_none());
    }

    #[test]
    fn empty_or_whitespace_prose_fields_do_not_produce_sections() {
        let mut task = sample_full_task();
        task.summary = Some("   \n".to_string());
        task.description = Some(String::new());
        task.user_story = None;
        let (_, body) = task_to_document(&task);
        let names = body.names();
        assert!(!names.contains(&"Summary"));
        assert!(!names.contains(&"Description"));
        assert!(!names.contains(&"User Story"));
        // Requirements was Some("- TTL: 60s..."); should still produce a section.
        assert!(names.contains(&"Requirements"));
    }

    #[test]
    fn project_kind_task_round_trips_as_prj() {
        let id = LeafId::new(TypePrefix::Project, 1);
        let task = Task {
            id,
            title: "pm".to_string(),
            summary: None,
            description: None,
            user_story: None,
            requirements: None,
            tags: Vec::new(),
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
            due: None,
            parent: None,
            kind: Kind::Project,
            status: Status::Open,
            priority_level: None,
            urgency: None,
            process_stage: None,
            issue_link: None,
            pr_link: None,
            artifacts: Vec::new(),
            created_at_utc: 0,
            updated_at_utc: 0,
        };
        let (fm, body) = task_to_document(&task);
        assert_eq!(fm.id.prefix(), TypePrefix::Project);
        let back = task_from_document(&fm, &body, Vec::new());
        assert_eq!(back.kind, Kind::Project);
        assert_eq!(back.id.to_string(), "PRJ1");
    }

    #[test]
    fn project_ancestor_walks_full_chain() {
        let prj = LeafId::new(TypePrefix::Project, 1);
        let prd = LeafId::new(TypePrefix::Product, 1);
        let epc = LeafId::new(TypePrefix::Epic, 3);
        let tsk = LeafId::new(TypePrefix::Task, 7);

        let mut db = Database::default();
        db.tasks.push(simple_task(prj, "pm", None, Kind::Project));
        db.tasks
            .push(simple_task(prd, "core", Some(prj), Kind::Product));
        db.tasks
            .push(simple_task(epc, "checkouts", Some(prd), Kind::Epic));
        let task = simple_task(tsk, "lock", Some(epc), Kind::Task);
        db.tasks.push(task.clone());

        assert_eq!(project_ancestor(&db, &task), Some(prj));
    }

    #[test]
    fn project_ancestor_returns_none_for_orphan() {
        let tsk = LeafId::new(TypePrefix::Task, 7);
        let mut db = Database::default();
        let task = simple_task(tsk, "stray", None, Kind::Task);
        db.tasks.push(task.clone());
        assert!(project_ancestor(&db, &task).is_none());
    }

    fn simple_task(id: LeafId, title: &str, parent: Option<LeafId>, kind: Kind) -> Task {
        Task {
            id,
            title: title.to_string(),
            summary: None,
            description: None,
            user_story: None,
            requirements: None,
            tags: Vec::new(),
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
            due: None,
            parent,
            kind,
            status: Status::Open,
            priority_level: None,
            urgency: None,
            process_stage: None,
            issue_link: None,
            pr_link: None,
            artifacts: Vec::new(),
            created_at_utc: 0,
            updated_at_utc: 0,
        }
    }
}
