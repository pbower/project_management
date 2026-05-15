//! Phase 8 acceptance tests: the Document Workspace surface.
//!
//! The TUI itself is interactive and hard to drive without a terminal harness,
//! so these tests focus on the on-disk effects the Mode 2 keys actually
//! produce. The `a` key copies a source file into a ticket's `artifacts/`
//! and triggers a sweep; the `r` key (in its `move <ADDRESS>` form) reparents
//! a ticket and writes an alias from the old address to the new one. Both
//! flows go through the same library primitives that the CLI `pm move` and
//! `pm artifact add` verbs use, so the binary is a reliable proxy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase8-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `pm --db <pm_dir> <args...>` and return the raw `Output` so callers
/// can assert on streams as well as exit status.
fn pm(pm_dir: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_pm");
    let mut cmd = Command::new(bin);
    cmd.arg("--db").arg(pm_dir).args(args);
    cmd.output().expect("invoke pm binary")
}

/// Initialise `.pm/` and build a project hierarchy reasonable for the
/// document workspace tests: a PRJ, a PRD under it, an EPC under the PRD,
/// a TSK under the EPC.
fn seed_workspace(pm_dir: &Path) {
    let init = pm(pm_dir, &["init"]);
    assert!(init.status.success(), "pm init: {init:?}");

    let p = pm(pm_dir, &["add", "Demo project", "--kind", "project"]);
    assert!(p.status.success(), "add project: {}", String::from_utf8_lossy(&p.stderr));

    let prd = pm(pm_dir, &["add", "Core product", "--kind", "product", "--parent", "PRJ1"]);
    assert!(prd.status.success(), "add product: {}", String::from_utf8_lossy(&prd.stderr));

    let epc = pm(pm_dir, &["add", "Checkouts", "--kind", "epic", "--parent", "PRD1"]);
    assert!(epc.status.success(), "add epic: {}", String::from_utf8_lossy(&epc.stderr));

    let tsk = pm(pm_dir, &["add", "Lock protocol", "--kind", "task", "--parent", "EPC1"]);
    assert!(tsk.status.success(), "add task: {}", String::from_utf8_lossy(&tsk.stderr));
}

#[test]
#[allow(non_snake_case)]
fn artifact_add_lands_in_ARTIFACTS_md() {
    let dir = tmp_dir("artifact");
    seed_workspace(&dir);

    // Build a source file outside .pm/ so the copy is a real cross-directory
    // move, matching the TUI's `a` prompt flow.
    let src = dir.join("schema.png");
    fs::write(&src, b"png-bytes").unwrap();

    let out = pm(&dir, &["artifact", "add", "TSK1", src.to_str().unwrap()]);
    assert!(out.status.success(), "artifact add: {}", String::from_utf8_lossy(&out.stderr));

    // The TSK should now have a copy under its artifacts/ dir and the
    // sweep should have produced an ARTIFACTS.md entry.
    let task_dir = dir.join("projects/PRJ1/products/PRD1/epics/EPC1/tasks/TSK1");
    assert!(task_dir.join("artifacts/schema.png").exists(),
        "schema.png missing from artifacts/");
    let manifest = fs::read_to_string(task_dir.join("artifacts/ARTIFACTS.md"))
        .expect("ARTIFACTS.md should exist after sweep");
    assert!(manifest.contains("file: schema.png"), "ARTIFACTS.md missing entry: {manifest}");
}

#[test]
fn move_writes_alias_and_old_address_still_resolves() {
    let dir = tmp_dir("move");
    seed_workspace(&dir);

    // Stand up a second EPC so the task has somewhere to move to.
    let second_epc = pm(&dir, &["add", "Locking", "--kind", "epic", "--parent", "PRD1"]);
    assert!(second_epc.status.success(),
        "add second epic: {}", String::from_utf8_lossy(&second_epc.stderr));

    // Move TSK1 from EPC1 to EPC2.
    let mv = pm(&dir, &["move", "TSK1", "EPC2"]);
    assert!(mv.status.success(), "pm move: {}", String::from_utf8_lossy(&mv.stderr));

    // The alias file should now record the redirect.
    let aliases = fs::read_to_string(dir.join("aliases.json"))
        .expect("aliases.json should exist after a move");
    assert!(aliases.contains("PRJ1-PRD1-EPC1-TSK1"),
        "alias missing old address: {aliases}");
    assert!(aliases.contains("PRJ1-PRD1-EPC2-TSK1"),
        "alias missing new address: {aliases}");

    // The TSK should now live under EPC2 on disk, and the old leftover
    // directory under EPC1 should be gone - cmd_move's post-save cleanup
    // sees the new state.items[leaf] path now that Database::save mutates
    // self.state in place rather than writing a clone.
    let new_path = dir.join("projects/PRJ1/products/PRD1/epics/EPC2/tasks/TSK1");
    assert!(new_path.exists(), "new TSK1 dir missing: {}", new_path.display());

    let old_path = dir.join("projects/PRJ1/products/PRD1/epics/EPC1/tasks/TSK1");
    assert!(!old_path.exists(), "old TSK1 dir not cleaned up: {}", old_path.display());

    // Looking up the old address-form should still resolve to the live ticket
    // via the alias.
    let show = pm(&dir, &["show", "PRJ1-PRD1-EPC1-TSK1"]);
    assert!(show.status.success(),
        "show via old address: {}", String::from_utf8_lossy(&show.stderr));
}

#[test]
fn move_emits_event_and_records_move_verb() {
    let dir = tmp_dir("event");
    seed_workspace(&dir);

    let second_epc = pm(&dir, &["add", "Locking", "--kind", "epic", "--parent", "PRD1"]);
    assert!(second_epc.status.success());

    let _ = pm(&dir, &["move", "TSK1", "EPC2"]);

    let events = fs::read_to_string(dir.join("events.log"))
        .expect("events.log should exist after a move");
    assert!(events.contains("\"verb\":\"move\""),
        "move event missing from feed: {events}");
}
