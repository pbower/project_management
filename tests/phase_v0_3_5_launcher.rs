//! v0.3.5 acceptance tests: launcher + terminal registry + MCP scope
//! enforcement.
//!
//! These exercise the library surface end-to-end without needing a TTY
//! or actually spawning OS terminals. `spawn_terminal` is hooked at the
//! registry level (the OS exec it triggers is intentionally a no-op
//! launcher template the test sets up) so we can verify the
//! UUID/registry contract without flaking on the user's shell.
//!
//! The MCP scope tests drive the dispatcher directly via the in-process
//! `Server` so the JSON-RPC envelope stays out of the test's way.

use std::fs;
use std::path::{Path, PathBuf};

use project_management::launcher;
use project_management::store::events::read_events;
use project_management::store::id::{LeafId, TypePrefix};
use project_management::store::layout::Layout as StoreLayout;

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-v035-{label}-{}-{}",
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

fn write_launcher_config(pm_dir: &Path, spawn: &str) {
    let toml = format!("[launcher]\nspawn = \"{}\"\n", spawn.replace('"', "\\\""));
    fs::write(pm_dir.join(".thunder.toml"), toml).unwrap();
}

#[test]
fn registry_round_trips_an_entry_written_by_spawn() {
    let dir = init_workspace("spawn-registry");
    // No-op spawn that still references {cmd} so the substituted
    // spawn_command on the registry entry includes the uuid we just
    // generated; `true` discards the args at runtime so nothing
    // actually launches.
    write_launcher_config(&dir, "true {cmd}");

    let leaf = LeafId::new(TypePrefix::Task, 1);
    let uuid = launcher::spawn_terminal(&dir, leaf, Some("demo"), Some(&dir)).expect("spawn");

    assert!(uuid.starts_with("tk-"), "uuid prefix: {uuid}");

    let entry = launcher::load_terminal(&dir, &uuid).expect("registry has entry");
    assert_eq!(entry.scope, leaf);
    assert_eq!(entry.status, launcher::TerminalStatus::Active);
    assert!(entry.label.contains("demo"));
    assert!(
        entry.spawn_command.contains(&uuid),
        "spawn_command should include the substituted uuid: {}",
        entry.spawn_command
    );
}

#[test]
fn spawn_emits_a_terminal_spawn_event_with_scope_tag() {
    let dir = init_workspace("spawn-event");
    write_launcher_config(&dir, "true");

    let leaf = LeafId::new(TypePrefix::Task, 42);
    let uuid = launcher::spawn_terminal(&dir, leaf, None, None).unwrap();

    let events = read_events(&dir).unwrap();
    let spawn_events: Vec<_> = events
        .iter()
        .filter(|e| e.verb == "terminal-spawn")
        .collect();
    assert_eq!(spawn_events.len(), 1, "exactly one spawn event");
    let ev = spawn_events[0];
    assert_eq!(ev.id, Some(leaf));
    assert!(
        ev.detail.as_deref().unwrap_or("").contains(&uuid),
        "spawn event detail mentions the uuid: {:?}",
        ev.detail
    );
}

#[test]
fn purge_dead_terminals_flips_status_when_heartbeat_lapses() {
    let dir = init_workspace("purge-flip");
    let leaf = LeafId::new(TypePrefix::Task, 7);
    let mut entry =
        launcher::TerminalEntry::new("tk-stale".into(), leaf, "stale".into(), "true".into(), 1);
    entry.last_heartbeat = chrono::Utc::now() - chrono::Duration::seconds(300);
    launcher::write_terminal(&dir, &entry).unwrap();

    let acted = launcher::purge_dead_terminals(&dir, false).unwrap();
    assert_eq!(acted, vec!["tk-stale".to_string()]);
    let back = launcher::load_terminal(&dir, "tk-stale").unwrap();
    assert_eq!(back.status, launcher::TerminalStatus::Dead);
}

#[test]
fn purge_dead_terminals_deletes_when_asked() {
    let dir = init_workspace("purge-delete");
    let leaf = LeafId::new(TypePrefix::Task, 9);
    let mut entry =
        launcher::TerminalEntry::new("tk-gone".into(), leaf, "gone".into(), "true".into(), 1);
    entry.last_heartbeat = chrono::Utc::now() - chrono::Duration::seconds(300);
    launcher::write_terminal(&dir, &entry).unwrap();

    let acted = launcher::purge_dead_terminals(&dir, true).unwrap();
    assert_eq!(acted, vec!["tk-gone".to_string()]);
    assert!(launcher::load_terminal(&dir, "tk-gone").is_none());
}

#[test]
fn label_for_omits_title_when_none() {
    let leaf = LeafId::new(TypePrefix::Task, 3);
    assert_eq!(launcher::config::label_for(leaf, None), leaf.to_string());
    assert!(launcher::config::label_for(leaf, Some("lock proto")).ends_with("lock proto"));
}

// ---------------------------------------------------------------------------
// MCP scope enforcement
// ---------------------------------------------------------------------------

mod mcp_scope {
    use super::*;
    use project_management::mcp::server::Server;
    use std::io::Cursor;

    fn rpc(server: &mut Server, lines: &[&str]) -> Vec<serde_json::Value> {
        let mut input = String::new();
        for l in lines {
            input.push_str(l);
            input.push('\n');
        }
        let mut out: Vec<u8> = Vec::new();
        server.drive(Cursor::new(input), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn seed(label: &str) -> PathBuf {
        let dir = init_workspace(label);

        // Build PRJ1 / PRD1 / EPC1 / TSK1 + a sibling TSK2 under a
        // different epic so we have an out-of-scope target.
        use project_management::cmd::*;
        use project_management::db::Database;
        use project_management::fields::*;

        let mut db = Database::load(&dir);
        cmd_init(&dir);
        cmd_add(
            &mut db,
            &dir,
            "Demo".into(),
            None,
            None,
            vec![],
            None,
            None,
            Kind::Project,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            Status::Open,
        );
        cmd_add(
            &mut db,
            &dir,
            "Core".into(),
            None,
            None,
            vec![],
            None,
            Some("PRJ1".into()),
            Kind::Product,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            Status::Open,
        );
        cmd_add(
            &mut db,
            &dir,
            "Epic1".into(),
            None,
            None,
            vec![],
            None,
            Some("PRD1".into()),
            Kind::Epic,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            Status::Open,
        );
        cmd_add(
            &mut db,
            &dir,
            "Task1".into(),
            None,
            None,
            vec![],
            None,
            Some("EPC1".into()),
            Kind::Task,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            Status::Open,
        );
        // Sibling epic + sibling task that sits outside EPC1.
        cmd_add(
            &mut db,
            &dir,
            "Epic2".into(),
            None,
            None,
            vec![],
            None,
            Some("PRD1".into()),
            Kind::Epic,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            Status::Open,
        );
        cmd_add(
            &mut db,
            &dir,
            "Task2".into(),
            None,
            None,
            vec![],
            None,
            Some("EPC2".into()),
            Kind::Task,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            Status::Open,
        );
        dir
    }

    /// With `THUNDER_SCOPE=EPC1`, a checkout on TSK1 stays in scope so
    /// no warning event is recorded. A checkout on TSK2 (under EPC2)
    /// is out-of-scope and produces a warning event.
    #[test]
    fn out_of_scope_checkout_emits_warning_event_but_succeeds() {
        let dir = seed("scope-warning");

        // Set the scope for the duration of this test; remember to
        // clear it afterwards so other tests in the binary do not
        // inherit it. Cargo runs integration tests in their own
        // process so the cross-test leak risk is bounded but the
        // hygiene matters anyway.
        std::env::set_var("THUNDER_SCOPE", "EPC1");

        let mut server = Server::new(dir.clone());

        // In-scope checkout: TSK1 sits under EPC1.
        let resp = rpc(
            &mut server,
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"checkout","arguments":{"id":"TSK1","intent":"in-scope"}}}"#,
            ],
        );
        assert_eq!(resp[0]["result"]["isError"], serde_json::json!(false));

        // Out-of-scope checkout: TSK2 lives under EPC2.
        let resp = rpc(
            &mut server,
            &[
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"checkout","arguments":{"id":"TSK2","intent":"out-of-scope"}}}"#,
            ],
        );
        assert_eq!(resp[0]["result"]["isError"], serde_json::json!(false));

        // The warning event landed in events.log with scope = EPC1.
        let events = read_events(&dir).unwrap();
        let warnings: Vec<_> = events
            .iter()
            .filter(|e| e.verb == "warning")
            .filter(|e| e.detail.as_deref().unwrap_or("").contains("out-of-scope"))
            .collect();
        assert_eq!(warnings.len(), 1, "exactly one out-of-scope warning");
        let w = warnings[0];
        assert_eq!(w.id.map(|i| i.to_string()), Some("TSK2".to_string()));
        assert_eq!(w.scope.map(|s| s.to_string()), Some("EPC1".to_string()));

        std::env::remove_var("THUNDER_SCOPE");
    }
}
