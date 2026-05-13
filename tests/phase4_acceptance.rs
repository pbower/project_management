//! Phase 4 acceptance test: drive the v2 CLI end-to-end and verify the disk
//! shape matches PM_DESIGN.md Section 4.

use std::fs;
use std::path::PathBuf;

use project_management::store::{
    Aliases, ArtifactsIndex, State, Ticket, TypePrefix, ARTIFACTS_MD, CLAUDE_MD,
};
use project_management::v2::cli::{ArtifactAction, KindArg, V2Commands};
use project_management::v2::cmd::run;

fn tmp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase4-acc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn full_hierarchy_then_inspect_disk() {
    let base = tmp_dir();
    let pm_root = base.clone();

    // init + the full PRJ -> PRD -> EPC -> TSK -> SBT chain.
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(
        V2Commands::Add {
            title: "PM tool".into(),
            kind: KindArg::Project,
            parent: None,
            slug: None,
        },
        Some(pm_root.clone()),
    )
    .unwrap();
    run(
        V2Commands::Add {
            title: "Core".into(),
            kind: KindArg::Product,
            parent: Some("PRJ1".into()),
            slug: None,
        },
        Some(pm_root.clone()),
    )
    .unwrap();
    run(
        V2Commands::Add {
            title: "Checkouts".into(),
            kind: KindArg::Epic,
            parent: Some("PRD1".into()),
            slug: None,
        },
        Some(pm_root.clone()),
    )
    .unwrap();
    run(
        V2Commands::Add {
            title: "Lock protocol".into(),
            kind: KindArg::Task,
            parent: Some("EPC1".into()),
            slug: None,
        },
        Some(pm_root.clone()),
    )
    .unwrap();
    run(
        V2Commands::Add {
            title: "Stale cleanup".into(),
            kind: KindArg::Subtask,
            parent: Some("TSK1".into()),
            slug: None,
        },
        Some(pm_root.clone()),
    )
    .unwrap();

    // The disk should match PM_DESIGN.md Section 4.
    let dot_pm = base.join(".pm");
    let prj = dot_pm.join("projects/pm-tool");
    let prd = prj.join("products/core");
    let epc = prd.join("epics/checkouts");
    let tsk = epc.join("tasks/lock-protocol");
    let sbt = tsk.join("subtasks/stale-cleanup");
    for p in [&prj, &prd, &epc, &tsk, &sbt] {
        assert!(p.is_dir(), "expected directory at {}", p.display());
        assert!(p.join(CLAUDE_MD).is_file(), "missing CLAUDE.md at {}", p.display());
        assert!(p.join("artifacts").is_dir(), "missing artifacts/ at {}", p.display());
    }

    // state.json should index all five with their relative paths.
    let state = State::load(&dot_pm.join("state.json")).unwrap();
    assert_eq!(state.items.len(), 5);
    assert_eq!(
        state.items.get(&project_management::store::LeafId::new(TypePrefix::Subtask, 1)).unwrap().path,
        PathBuf::from("projects/pm-tool/products/core/epics/checkouts/tasks/lock-protocol/subtasks/stale-cleanup"),
    );

    // Each ticket's CLAUDE.md should parse, carry the right id, parent, and
    // project, and have a non-empty section template.
    let task = Ticket::read(&tsk.join(CLAUDE_MD)).unwrap();
    assert_eq!(task.front_matter.title, "Lock protocol");
    assert_eq!(task.front_matter.parent.unwrap().to_string(), "EPC1");
    assert_eq!(task.front_matter.project.unwrap().to_string(), "PRJ1");
    let task_section_names = task.body.names();
    assert!(task_section_names.contains(&"Description"));
    assert!(task_section_names.contains(&"User Story"));
    assert!(task_section_names.contains(&"Acceptance Criteria"));

    fs::remove_dir_all(&base).ok();
}

#[test]
fn metadata_verbs_update_front_matter() {
    let base = tmp_dir();
    let pm_root = base.clone();
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(
        V2Commands::Add {
            title: "Solo".into(),
            kind: KindArg::Task,
            parent: None,
            slug: None,
        },
        Some(pm_root.clone()),
    )
    .unwrap();

    run(V2Commands::Status { id: "TSK1".into(), value: "in-progress".into() }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Priority { id: "TSK1".into(), value: "high".into() }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Tag { id: "TSK1".into(), ops: vec!["+infra".into(), "+locking".into()] }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Link { id: "TSK1".into(), key: "github_issue".into(), value: "pbower/project_management#42".into() }, Some(pm_root.clone())).unwrap();

    let tsk = base.join(".pm/tasks/solo");
    let t = Ticket::read(&tsk.join(CLAUDE_MD)).unwrap();
    assert_eq!(t.front_matter.status, project_management::fields::Status::InProgress);
    assert_eq!(t.front_matter.priority, Some(project_management::fields::Priority::MustHave));
    assert_eq!(t.front_matter.tags, vec!["infra".to_string(), "locking".to_string()]);
    assert_eq!(t.front_matter.links.get("github_issue").map(|s| s.as_str()), Some("pbower/project_management#42"));

    fs::remove_dir_all(&base).ok();
}

#[test]
fn move_writes_alias_and_relocates_directory() {
    let base = tmp_dir();
    let pm_root = base.clone();
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "PM".into(), kind: KindArg::Project, parent: None, slug: None }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Core".into(), kind: KindArg::Product, parent: Some("PRJ1".into()), slug: None }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Checkouts".into(), kind: KindArg::Epic, parent: Some("PRD1".into()), slug: None }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Locking".into(), kind: KindArg::Epic, parent: Some("PRD1".into()), slug: None }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Lock protocol".into(), kind: KindArg::Task, parent: Some("EPC1".into()), slug: None }, Some(pm_root.clone())).unwrap();

    // Move TSK1 from EPC1 (checkouts) to EPC2 (locking).
    run(V2Commands::Move { id: "TSK1".into(), dest: "EPC2".into() }, Some(pm_root.clone())).unwrap();

    let old_dir = base.join(".pm/projects/pm/products/core/epics/checkouts/tasks/lock-protocol");
    let new_dir = base.join(".pm/projects/pm/products/core/epics/locking/tasks/lock-protocol");
    assert!(!old_dir.exists(), "old directory should be gone");
    assert!(new_dir.is_dir(), "new directory should exist");

    // state.json points at the new path.
    let state = State::load(&base.join(".pm/state.json")).unwrap();
    let tsk = project_management::store::LeafId::new(TypePrefix::Task, 1);
    assert_eq!(state.items.get(&tsk).unwrap().path,
        PathBuf::from("projects/pm/products/core/epics/locking/tasks/lock-protocol"));

    // The old address resolves via aliases.
    let aliases = Aliases::load(&base.join(".pm/aliases.json")).unwrap();
    let target = aliases.resolve("PRJ1-PRD1-EPC1-TSK1", 4).unwrap();
    assert_eq!(target, "PRJ1-PRD1-EPC2-TSK1");

    fs::remove_dir_all(&base).ok();
}

#[test]
fn delete_tombstones_and_removes_directory() {
    let base = tmp_dir();
    let pm_root = base.clone();
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Tossable".into(), kind: KindArg::Task, parent: None, slug: None }, Some(pm_root.clone())).unwrap();
    let tsk_dir = base.join(".pm/tasks/tossable");
    assert!(tsk_dir.exists());
    run(V2Commands::Delete { id: "TSK1".into(), force: true }, Some(pm_root.clone())).unwrap();
    assert!(!tsk_dir.exists(), "directory should be removed");

    let state = State::load(&base.join(".pm/state.json")).unwrap();
    let tsk = project_management::store::LeafId::new(TypePrefix::Task, 1);
    assert!(!state.items.contains_key(&tsk));
    assert!(state.is_tombstoned(tsk));

    fs::remove_dir_all(&base).ok();
}

#[test]
fn artifact_add_list_rename() {
    let base = tmp_dir();
    let pm_root = base.clone();
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Lock".into(), kind: KindArg::Task, parent: None, slug: None }, Some(pm_root.clone())).unwrap();

    // Drop an artifact file outside .pm/ first.
    let src = base.join("schema.png");
    fs::write(&src, b"PNG").unwrap();

    run(
        V2Commands::Artifact {
            action: ArtifactAction::Add {
                id: "TSK1".into(),
                path: src.clone(),
                desc: Some("ER diagram".into()),
            },
        },
        Some(pm_root.clone()),
    ).unwrap();

    let artifacts_dir = base.join(".pm/tasks/lock/artifacts");
    let idx = ArtifactsIndex::load(&artifacts_dir.join(ARTIFACTS_MD)).unwrap();
    assert_eq!(idx.find("schema.png").unwrap().desc, "ER diagram");

    run(
        V2Commands::Artifact {
            action: ArtifactAction::Rename {
                id: "TSK1".into(),
                old: "schema.png".into(),
                new: "er-diagram.png".into(),
            },
        },
        Some(pm_root.clone()),
    ).unwrap();
    let after = ArtifactsIndex::load(&artifacts_dir.join(ARTIFACTS_MD)).unwrap();
    assert_eq!(after.find("er-diagram.png").unwrap().desc, "ER diagram");
    assert!(after.find("schema.png").is_none());

    fs::remove_dir_all(&base).ok();
}

#[test]
fn template_apply_preserves_content() {
    let base = tmp_dir();
    let pm_root = base.clone();
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Solo".into(), kind: KindArg::Task, parent: None, slug: None }, Some(pm_root.clone())).unwrap();

    // Manually edit the description by reading + writing the file (simulates
    // a $EDITOR session).
    let tsk = base.join(".pm/tasks/solo");
    let mut t = Ticket::read(&tsk.join(CLAUDE_MD)).unwrap();
    t.upsert_section("Description", "Mine.\n");
    t.write_to(&tsk).unwrap();

    run(
        V2Commands::Template {
            action: project_management::v2::cli::TemplateAction::Apply { id: "TSK1".into() },
        },
        Some(pm_root.clone()),
    ).unwrap();

    let back = Ticket::read(&tsk.join(CLAUDE_MD)).unwrap();
    assert_eq!(back.body.find("Description").unwrap().body, "Mine.\n");

    fs::remove_dir_all(&base).ok();
}

#[test]
fn doctor_rebuilds_state_json_from_disk() {
    let base = tmp_dir();
    let pm_root = base.clone();
    run(V2Commands::Init, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "PM".into(), kind: KindArg::Project, parent: None, slug: None }, Some(pm_root.clone())).unwrap();
    run(V2Commands::Add { title: "Core".into(), kind: KindArg::Product, parent: Some("PRJ1".into()), slug: None }, Some(pm_root.clone())).unwrap();

    // Wipe state.json; doctor should rebuild it.
    let state_path = base.join(".pm/state.json");
    fs::write(&state_path, r#"{ "next": {}, "tombstones": {}, "items": {} }"#).unwrap();
    run(V2Commands::Doctor, Some(pm_root.clone())).unwrap();

    let rebuilt = State::load(&state_path).unwrap();
    assert_eq!(rebuilt.items.len(), 2, "doctor should rediscover both tickets");
    let prj = project_management::store::LeafId::new(TypePrefix::Project, 1);
    let prd = project_management::store::LeafId::new(TypePrefix::Product, 1);
    assert!(rebuilt.items.contains_key(&prj));
    assert!(rebuilt.items.contains_key(&prd));

    fs::remove_dir_all(&base).ok();
}
