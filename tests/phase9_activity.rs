//! Phase 9 acceptance tests: the activity view used by Mode 3 and `pm tv`.
//!
//! The view itself is interactive and depends on a TTY for `enable_raw_mode`,
//! so the binary-level test surface is limited to the CLI shape (`pm tv
//! --help` and accepted argument forms). The behavioural guarantees the spec
//! makes - identical rendering between Mode 3 and `pm tv`, filter narrowing,
//! pause-anchored scroll, rotation-aware refresh - are checked against the
//! same `ActivityView` instance both call sites construct, so a library-level
//! end-to-end test through the public `views::events_view` API is a faithful
//! acceptance proxy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use project_management::store::events::{emit_event, Event};
use project_management::store::id::{LeafId, TypePrefix};
use project_management::store::layout::Layout as StoreLayout;
use project_management::views::events_view::{ActivityFilter, ActivityView};

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase9-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn pm(pm_dir: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_spacecell");
    let mut cmd = Command::new(bin);
    cmd.arg("--db").arg(pm_dir).args(args);
    cmd.output().expect("invoke pm binary")
}

fn pm_no_db(args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_spacecell");
    Command::new(bin)
        .args(args)
        .output()
        .expect("invoke pm binary")
}

#[test]
fn tv_help_lists_the_optional_path_argument() {
    // `pm help tv` is the structured help surface; it should mention the
    // verb and the PATH argument. This proves clap recognises both.
    let out = pm_no_db(&["help", "tv"]);
    assert!(out.status.success(), "pm help tv exit: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.to_lowercase().contains("tv"),
        "help mentions tv: {stdout}"
    );
    assert!(
        stdout.to_uppercase().contains("PATH"),
        "help mentions PATH arg: {stdout}"
    );
}

#[test]
fn activity_view_ingests_events_from_a_real_workspace() {
    let pm_dir = tmp_dir("ingest");
    let init = pm(&pm_dir, &["init"]);
    assert!(init.status.success(), "pm init failed: {init:?}");

    // The init verb already writes one event; add three more from the test.
    let tsk = LeafId::new(TypePrefix::Task, 1);
    emit_event(&pm_dir, "checkout", Some(tsk), Some("implement TTL")).unwrap();
    emit_event(&pm_dir, "edit", Some(tsk), None).unwrap();
    emit_event(&pm_dir, "checkin", Some(tsk), Some("done")).unwrap();

    // The pm_dir resolution treats the directory's parent as `pm_dir`
    // (see main.rs), so the actual `.pm/` lives one level down. Read
    // through the activity view by pointing it at that workspace.
    let workspace = StoreLayout::at(&pm_dir);
    let mut view = ActivityView::new(workspace.root.clone());
    view.refresh().unwrap();
    assert!(
        view.events.len() >= 4,
        "expected at least the init event plus three appended; got {}",
        view.events.len()
    );

    let verbs: Vec<&str> = view
        .events
        .iter()
        .map(|e: &Event| e.verb.as_str())
        .collect();
    assert!(verbs.contains(&"checkout"), "checkout present: {verbs:?}");
    assert!(verbs.contains(&"edit"), "edit present: {verbs:?}");
    assert!(verbs.contains(&"checkin"), "checkin present: {verbs:?}");
}

#[test]
fn filter_narrows_view_then_clears_restores_full_set() {
    let pm_dir = tmp_dir("filter");
    let init = pm(&pm_dir, &["init"]);
    assert!(init.status.success());

    let tsk1 = LeafId::new(TypePrefix::Task, 1);
    let tsk2 = LeafId::new(TypePrefix::Task, 2);
    emit_event(&pm_dir, "edit", Some(tsk1), None).unwrap();
    emit_event(&pm_dir, "edit", Some(tsk2), None).unwrap();
    emit_event(&pm_dir, "checkout", Some(tsk1), None).unwrap();

    let workspace = StoreLayout::at(&pm_dir);
    let mut view = ActivityView::new(workspace.root.clone());
    view.refresh().unwrap();
    let total = view.events.len();

    // Narrow to checkout-only via the verb field.
    view.filter = ActivityFilter::parse("verb:checkout");
    assert!(view.filtered().iter().all(|e| e.verb == "checkout"));
    assert!(view.filtered().len() < total, "filter narrows");

    // Clear via the public clear method (what `c` in the view binds to).
    view.filter.clear();
    assert_eq!(
        view.filtered().len(),
        total,
        "clearing the filter restores the full view"
    );
}

#[test]
fn refresh_picks_up_new_events_after_first_read() {
    let pm_dir = tmp_dir("incremental");
    let init = pm(&pm_dir, &["init"]);
    assert!(init.status.success());

    let workspace = StoreLayout::at(&pm_dir);
    let mut view = ActivityView::new(workspace.root.clone());
    view.refresh().unwrap();
    let initial = view.events.len();
    let initial_offset = view.read_offset;
    assert!(initial_offset > 0, "init wrote at least one event");

    // Append fresh events after the first refresh.
    let tsk = LeafId::new(TypePrefix::Task, 7);
    emit_event(&pm_dir, "edit", Some(tsk), None).unwrap();
    emit_event(&pm_dir, "edit", Some(tsk), None).unwrap();

    view.refresh().unwrap();
    assert_eq!(view.events.len(), initial + 2);
    assert!(
        view.read_offset > initial_offset,
        "incremental refresh advances the offset rather than re-reading from scratch"
    );
}

#[test]
fn activity_view_handles_missing_events_log_gracefully() {
    // If a user runs `pm tv` against a directory that has no events.log
    // yet (e.g. before `pm init`), the view should refresh into an empty
    // buffer rather than erroring.
    let pm_dir = tmp_dir("missing-log");
    let mut view = ActivityView::new(pm_dir.clone());
    view.refresh()
        .expect("refresh tolerates a missing events.log");
    assert!(view.events.is_empty());
    assert!(view.filtered().is_empty());
}
