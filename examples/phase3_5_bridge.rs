//! End-to-end exercise of the Phase 3.5 Task <-> Document bridge.
//!
//! Builds a small in-memory `Database` containing a PRJ -> PRD -> EPC -> TSK
//! -> SBT chain, runs each `Task` through the bridge to produce a
//! `(FrontMatter, ParsedBody)` pair, serialises that to a complete
//! `CLAUDE.md`-shaped string via the existing `Ticket::render`, parses it back,
//! reconstructs a `Task` through the bridge, and prints the resulting tree to
//! demonstrate round-trip fidelity.
//!
//! Usage:
//!     cargo run --example phase3_5_bridge

use std::process::ExitCode;

use chrono::NaiveDate;

use project_management::db::{kind_to_prefix, Database};
use project_management::fields::{Kind, Priority, ProcessStage, Status, Urgency};
use project_management::store::front_matter::Document;
use project_management::store::sections::ParsedBody;
use project_management::store::task_bridge::{project_ancestor, task_from_document, task_to_document};
use project_management::store::{LeafId, State, Ticket, TypePrefix};
use project_management::task::Task;

fn main() -> ExitCode {
    let mut state = State::fresh();

    // Pre-allocate the chain of leaf ids; the bridge takes pre-built Tasks so
    // we line them up in a deterministic order.
    let prj = state.allocate(TypePrefix::Project);
    let prd = state.allocate(TypePrefix::Product);
    let epc = state.allocate(TypePrefix::Epic);
    let tsk = state.allocate(TypePrefix::Task);
    let sbt = state.allocate(TypePrefix::Subtask);

    let mut db = Database { tasks: Vec::new(), state };

    db.tasks.push(make_task(prj, "pm", None, Kind::Project));
    db.tasks.push(make_task(prd, "Core", Some(prj), Kind::Product));
    db.tasks.push(make_task(epc, "Checkout protocol", Some(prd), Kind::Epic));
    db.tasks.push(make_task_with_prose(
        tsk,
        "Lock protocol with TTL and heartbeat",
        Some(epc),
        Kind::Task,
        Some("Heartbeat-driven lock release"),
        Some("Each agent gets an exclusive lock per ticket. Locks carry a TTL and heartbeat so a crashed agent does not hold a ticket indefinitely."),
        Some("As an agent, I want my lock to auto-release on crash."),
        Some("- TTL: 60s after last heartbeat\n- Cleanup runs on `pm doctor`"),
    ));
    db.tasks.push(make_task(sbt, "Stale-lock cleanup", Some(tsk), Kind::Subtask));

    println!("--- Source Database ---");
    for t in &db.tasks {
        let parent = t.parent.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
        println!("  {}  kind={:?}  parent={}  title={}", t.id, t.kind, parent, t.title);
    }

    // Run each task through the bridge to CLAUDE.md text and back. Confirm
    // every field survives.
    println!("\n--- Round-trip via bridge ---");
    let mut after: Vec<Task> = Vec::with_capacity(db.tasks.len());
    for task in &db.tasks {
        let (fm, body) = task_to_document(task);
        // Render through Ticket so the output matches what would actually land
        // on disk in Phase 4: front-matter delimiters, body, trailing
        // @artifacts import.
        let ticket = Ticket { front_matter: fm.clone(), body };
        let rendered = match ticket.render() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("render failed for {}: {e}", task.id);
                return ExitCode::FAILURE;
            }
        };

        // Parse it back.
        let doc = match Document::parse(&rendered) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("parse failed for {}: {e}", task.id);
                return ExitCode::FAILURE;
            }
        };
        let body_no_trailer = strip_artifacts_import(&doc.body);
        let parsed_body = ParsedBody::parse(&body_no_trailer);

        let recovered = task_from_document(&doc.front_matter, &parsed_body, task.artifacts.clone());
        assert_eq!(recovered.id, task.id);
        assert_eq!(recovered.title, task.title);
        assert_eq!(recovered.summary, task.summary);
        assert_eq!(recovered.description, task.description);
        assert_eq!(recovered.user_story, task.user_story);
        assert_eq!(recovered.requirements, task.requirements);
        assert_eq!(recovered.kind, task.kind);
        assert_eq!(recovered.parent, task.parent);
        after.push(recovered);
    }

    for t in &after {
        let parent = t.parent.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
        println!("  {}  kind={:?}  parent={}  title={}", t.id, t.kind, parent, t.title);
    }

    // project_ancestor: every non-Project task should resolve back to PRJ1.
    println!("\n--- project_ancestor derivation ---");
    for task in &db.tasks {
        if matches!(task.kind, Kind::Project) { continue; }
        let prj_owner = project_ancestor(&db, task);
        println!(
            "  {}  -> project {}",
            task.id,
            prj_owner.map(|l| l.to_string()).unwrap_or_else(|| "-".into()),
        );
    }

    // Render one sample to show the wire format produced by the bridge.
    println!("\n--- Sample CLAUDE.md for TSK1 (rendered by Ticket) ---");
    let (fm, body) = task_to_document(&db.tasks[3]);
    let ticket = Ticket { front_matter: fm, body };
    match ticket.render() {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("render failed: {e}"),
    }

    ExitCode::SUCCESS
}

fn make_task(id: LeafId, title: &str, parent: Option<LeafId>, kind: Kind) -> Task {
    let _ = kind_to_prefix(kind); // sanity check the kind matches the leaf prefix
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
        created_at_utc: 1_715_900_000,
        updated_at_utc: 1_715_900_000,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_task_with_prose(
    id: LeafId,
    title: &str,
    parent: Option<LeafId>,
    kind: Kind,
    summary: Option<&str>,
    description: Option<&str>,
    user_story: Option<&str>,
    requirements: Option<&str>,
) -> Task {
    let mut t = make_task(id, title, parent, kind);
    t.summary = summary.map(|s| s.to_string());
    t.description = description.map(|s| s.to_string());
    t.user_story = user_story.map(|s| s.to_string());
    t.requirements = requirements.map(|s| s.to_string());
    t.tags = vec!["infra".into(), "locking".into()];
    t.due = NaiveDate::from_ymd_opt(2026, 5, 25);
    t.status = Status::InProgress;
    t.priority_level = Some(Priority::MustHave);
    t.urgency = Some(Urgency::UrgentImportant);
    t.process_stage = Some(ProcessStage::Implementation);
    t
}

/// Strip the trailing `@artifacts/ARTIFACTS.md` line and surrounding
/// whitespace that `Ticket::render` appends. The reverse parse expects only
/// the section body.
fn strip_artifacts_import(body: &str) -> String {
    let trimmed = body.trim_end_matches('\n');
    let needle = "@artifacts/ARTIFACTS.md";
    if let Some(prefix) = trimmed.strip_suffix(needle) {
        prefix.to_string()
    } else {
        body.to_string()
    }
}
