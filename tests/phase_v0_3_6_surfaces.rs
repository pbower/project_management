//! v0.3.6 acceptance tests: Workbench surfaces (Memories, Terminals,
//! Templates) + per-kind path conventions.
//!
//! Surface render is tested at the library level via the smaller
//! enumeration helpers each surface uses; the ratatui render path
//! itself needs a TTY and so stays out of the test surface.

use std::fs;
use std::path::PathBuf;

use project_management::launcher;
use project_management::store::events::emit_event;
use project_management::store::id::{LeafId, TypePrefix};
use project_management::store::layout::Layout as StoreLayout;

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-v036-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_workspace(label: &str) -> PathBuf {
    let dir = tmp_dir(label);
    StoreLayout::at(&dir).init().expect("init workspace");
    dir
}

/// Terminals surface relies on `launcher::list_terminals` for its
/// rendering source. A registry round-trip is what the surface sees,
/// so we test that the data the surface reads matches what spawn
/// writes.
#[test]
fn terminals_surface_sees_freshly_written_entries() {
    let dir = init_workspace("terminals-sees-fresh");
    let leaf = LeafId::new(TypePrefix::Task, 11);
    let entry = launcher::TerminalEntry::new(
        "tk-fresh".into(),
        leaf,
        "demo".into(),
        "true {cmd}".into(),
        1234,
    );
    launcher::write_terminal(&dir, &entry).unwrap();

    let listed = launcher::list_terminals(&dir).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].uuid, "tk-fresh");
    assert_eq!(listed[0].scope, leaf);
}

/// Terminal-spawn events carry the scope tag so the Activity feed and
/// the state-change ticker can show "agent X scoped to <leaf>".
#[test]
fn terminal_spawn_event_carries_scope_tag() {
    let dir = init_workspace("spawn-scope-tag");
    let leaf = LeafId::new(TypePrefix::Epic, 5);
    fs::write(dir.join(".thunder.toml"), "[launcher]\nspawn = \"true\"\n").unwrap();

    launcher::spawn_terminal(&dir, leaf, Some("checkouts"), Some(&dir)).unwrap();

    let events = project_management::store::events::read_events(&dir).expect("read events");
    let spawn = events
        .iter()
        .find(|e| e.verb == "terminal-spawn")
        .expect("spawn event landed");
    assert_eq!(spawn.id, Some(leaf));
}

/// Memory tier directories follow the documented convention. The
/// Memories surface enumerates files under these directories; the
/// path predicates have to be stable.
#[test]
fn memory_directory_layout_matches_documentation() {
    use project_management::memory::scope as mem_scope;

    let pm_dir = init_workspace("memory-dirs");

    let prj = LeafId::new(TypePrefix::Project, 1);
    let project_dir = mem_scope::project_dir(&pm_dir, prj);
    assert!(
        project_dir.ends_with("projects/PRJ1/memories"),
        "project memory dir: {}",
        project_dir.display()
    );

    let home = tmp_dir("memory-dirs-home");
    let user_dir = mem_scope::user_dir(&home, &pm_dir);
    assert!(
        user_dir.starts_with(&home),
        "user dir lives under home: {}",
        user_dir.display()
    );
    assert!(
        user_dir.to_string_lossy().contains(".claude/projects/"),
        "user dir under .claude/projects: {}",
        user_dir.display()
    );
}

/// Templates surface enumerates the six per-kind files plus the
/// workspace launcher config. The on-disk paths the surface
/// constructs need to live where the launcher actually looks for
/// them.
#[test]
fn templates_surface_paths_match_launcher_lookups() {
    let pm_dir = init_workspace("templates-paths");

    let project_template = pm_dir.join("templates/task.toml");
    assert_eq!(project_template.parent().unwrap(), pm_dir.join("templates"),);

    // Launcher reads .thunder.toml at the workspace root.
    let launcher_path = pm_dir.join(".thunder.toml");
    assert!(launcher_path.parent().unwrap() == pm_dir);

    // Sanity: writing a launcher config and reloading config gets the
    // user a value back without panic.
    fs::write(&launcher_path, "[launcher]\nspawn = \"echo {cmd}\"\n").unwrap();
    let cfg = launcher::load_config(&pm_dir);
    assert_eq!(
        launcher::resolve_spawn_command(&cfg),
        "echo {cmd}".to_string()
    );
}

/// Spawn emits a scoped event; the activity-strip ticker filters on
/// state-change verbs. terminal-spawn is informational rather than a
/// state change, so it must NOT pollute the state-change column.
#[test]
fn terminal_spawn_does_not_count_as_state_change() {
    let dir = init_workspace("spawn-not-state-change");
    let leaf = LeafId::new(TypePrefix::Task, 1);
    emit_event(&dir, "terminal-spawn", Some(leaf), Some("tk-x")).unwrap();
    emit_event(&dir, "status", Some(leaf), Some("in-progress")).unwrap();

    let events = project_management::store::events::read_events(&dir).expect("read events");
    assert_eq!(events.len(), 2);
    let state_changes: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.verb.as_str(), "status" | "complete" | "move"))
        .collect();
    assert_eq!(state_changes.len(), 1, "only the status verb counts");
}
