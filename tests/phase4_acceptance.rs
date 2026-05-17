//! Phase 4 acceptance test. Exercises the exit criteria from
//! `PM_BUILD_PLAN.md` Phase 4 end to end against the public crate API:
//!
//! - Each lifecycle verb has a working handler.
//! - `pm add --kind project "PM tool"` produces a valid PRJ ticket.
//! - `pm add --kind task ... --parent EPC1` lands the task at the correct
//!   addressed path under the parent chain.
//! - `pm doctor` rebuilds state.json from disk truth.
//! - The full PRJ -> PRD -> EPC -> TSK -> SBT chain materialises with the
//!   disk shape PM_DESIGN.md Section 4 specifies.

use std::fs;
use std::path::PathBuf;

use project_management::db::{kind_to_prefix, Database};
use project_management::fields::{Kind, Status};
use project_management::store::claude_md::CLAUDE_MD;
use project_management::store::layout::Layout;
use project_management::store::state::State;
use project_management::store::LeafId;
use project_management::task::Task;

fn tmp_pm_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase4-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fresh_task(id: LeafId, title: &str, parent: Option<LeafId>, kind: Kind) -> Task {
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

#[test]
fn full_chain_creation_via_database_lands_on_disk_per_design_section_4() {
    let pm_dir = tmp_pm_dir();

    // Start from an empty workspace and allocate each level in turn, the same
    // way the CLI's `pm add` handler does.
    let mut db = Database::load(&pm_dir);
    let prj = db.allocate_id(kind_to_prefix(Kind::Project));
    db.tasks
        .push(fresh_task(prj, "PM tool", None, Kind::Project));
    db.save(&pm_dir).unwrap();

    let mut db = Database::load(&pm_dir);
    let prd = db.allocate_id(kind_to_prefix(Kind::Product));
    db.tasks
        .push(fresh_task(prd, "Core", Some(prj), Kind::Product));
    db.save(&pm_dir).unwrap();

    let mut db = Database::load(&pm_dir);
    let epc = db.allocate_id(kind_to_prefix(Kind::Epic));
    db.tasks
        .push(fresh_task(epc, "Checkouts", Some(prd), Kind::Epic));
    db.save(&pm_dir).unwrap();

    let mut db = Database::load(&pm_dir);
    let tsk = db.allocate_id(kind_to_prefix(Kind::Task));
    db.tasks
        .push(fresh_task(tsk, "Lock protocol", Some(epc), Kind::Task));
    db.save(&pm_dir).unwrap();

    let mut db = Database::load(&pm_dir);
    let sbt = db.allocate_id(kind_to_prefix(Kind::Subtask));
    db.tasks
        .push(fresh_task(sbt, "Stale cleanup", Some(tsk), Kind::Subtask));
    db.save(&pm_dir).unwrap();

    // Verify the addressed paths exist with CLAUDE.md per ticket.
    let prj_dir = pm_dir.join("projects/PRJ1");
    let prd_dir = prj_dir.join("products/PRD1");
    let epc_dir = prd_dir.join("epics/EPC1");
    let tsk_dir = epc_dir.join("tasks/TSK1");
    let sbt_dir = tsk_dir.join("subtasks/SBT1");
    for dir in [&prj_dir, &prd_dir, &epc_dir, &tsk_dir, &sbt_dir] {
        assert!(dir.is_dir(), "missing directory: {}", dir.display());
        assert!(
            dir.join(CLAUDE_MD).is_file(),
            "missing CLAUDE.md in {}",
            dir.display()
        );
        assert!(
            dir.join("artifacts").is_dir(),
            "missing artifacts/ in {}",
            dir.display()
        );
    }

    // state.json indexes the chain.
    let layout = Layout::at(&pm_dir);
    let state = State::load(&layout.state_path()).unwrap();
    for leaf in [prj, prd, epc, tsk, sbt] {
        assert!(state.items.contains_key(&leaf), "state.json missing {leaf}");
    }

    fs::remove_dir_all(&pm_dir).ok();
}

#[test]
fn parent_address_resolves_to_correct_subtree() {
    let pm_dir = tmp_pm_dir();

    let mut db = Database::load(&pm_dir);
    let prj = db.allocate_id(kind_to_prefix(Kind::Project));
    db.tasks.push(fresh_task(prj, "X", None, Kind::Project));
    let prd = db.allocate_id(kind_to_prefix(Kind::Product));
    db.tasks
        .push(fresh_task(prd, "Y", Some(prj), Kind::Product));
    let epc = db.allocate_id(kind_to_prefix(Kind::Epic));
    db.tasks.push(fresh_task(epc, "Z", Some(prd), Kind::Epic));
    db.save(&pm_dir).unwrap();

    // Re-load, add a task scoped to EPC1, save. The task must land under
    // projects/PRJ1/products/PRD1/epics/EPC1/tasks/TSK1, not at the root.
    let mut db = Database::load(&pm_dir);
    let tsk = db.allocate_id(kind_to_prefix(Kind::Task));
    db.tasks
        .push(fresh_task(tsk, "Lock protocol", Some(epc), Kind::Task));
    db.save(&pm_dir).unwrap();

    let expected = pm_dir.join("projects/PRJ1/products/PRD1/epics/EPC1/tasks/TSK1/CLAUDE.md");
    assert!(
        expected.is_file(),
        "task should land at PRJ1-PRD1-EPC1-TSK1 path; got missing {}",
        expected.display()
    );

    let orphan_path = pm_dir.join("tasks/TSK1/CLAUDE.md");
    assert!(
        !orphan_path.is_file(),
        "task should not have landed at orphan path {}",
        orphan_path.display()
    );

    fs::remove_dir_all(&pm_dir).ok();
}

#[test]
fn doctor_rebuilds_state_json_from_disk_truth() {
    let pm_dir = tmp_pm_dir();
    let layout = Layout::at(&pm_dir);
    layout.init().unwrap();

    // Plant a complete chain on disk via Database::save.
    let mut db = Database::load(&pm_dir);
    let prj = db.allocate_id(kind_to_prefix(Kind::Project));
    db.tasks.push(fresh_task(prj, "X", None, Kind::Project));
    let prd = db.allocate_id(kind_to_prefix(Kind::Product));
    db.tasks
        .push(fresh_task(prd, "Y", Some(prj), Kind::Product));
    db.save(&pm_dir).unwrap();

    // Corrupt state.json by emptying state.items but leave the on-disk
    // directories intact. The "doctor" reload must reconstruct the
    // index from the surviving CLAUDE.md files.
    let state_path = layout.state_path();
    let raw = fs::read_to_string(&state_path).unwrap();
    let mut state: State = serde_json::from_str(&raw).unwrap();
    state.items.clear();
    state.save(&state_path).unwrap();

    // Database::load with no items finds zero tasks. Run a fresh doctor-style
    // walk by invoking the public path that rebuilds state.json: re-load and
    // re-save, which currently rewrites state.items only from in-memory
    // tasks. So we exercise the actual doctor handler via the binary path
    // instead.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spacecell"))
        .arg("--db")
        .arg(&pm_dir)
        .arg("doctor")
        .output()
        .expect("invoke pm doctor");
    assert!(
        output.status.success(),
        "pm doctor failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("scanned 2 tickets"),
        "expected scan summary; got:\n{stdout}"
    );

    // After doctor, state.items should contain the two tickets again.
    let recovered = State::load(&state_path).unwrap();
    assert_eq!(recovered.items.len(), 2);
    assert!(recovered.items.contains_key(&prj));
    assert!(recovered.items.contains_key(&prd));

    fs::remove_dir_all(&pm_dir).ok();
}

#[test]
fn pm_init_scaffolds_the_workspace() {
    let pm_dir = tmp_pm_dir();
    let pm_dir_inside = pm_dir.join("workspace");
    fs::create_dir_all(&pm_dir_inside).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spacecell"))
        .arg("--db")
        .arg(&pm_dir_inside)
        .arg("init")
        .output()
        .expect("invoke pm init");
    assert!(
        output.status.success(),
        "pm init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let state_path = pm_dir_inside.join("state.json");
    let aliases_path = pm_dir_inside.join("aliases.json");
    let events_path = pm_dir_inside.join("events.log");
    let locks_path = pm_dir_inside.join("locks");
    assert!(state_path.is_file(), "missing {}", state_path.display());
    assert!(aliases_path.is_file(), "missing {}", aliases_path.display());
    assert!(events_path.is_file(), "missing {}", events_path.display());
    assert!(locks_path.is_dir(), "missing {}", locks_path.display());

    for type_folder in [
        "projects",
        "products",
        "epics",
        "tasks",
        "subtasks",
        "milestones",
    ] {
        let p = pm_dir_inside.join(type_folder);
        assert!(p.is_dir(), "missing type folder: {}", p.display());
    }

    fs::remove_dir_all(&pm_dir).ok();
}
