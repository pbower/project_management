//! v0.3.7 acceptance tests: embedded agent PTYs.
//!
//! Drives `crate::agents::Agent` against a real PTY with `echo` /
//! `cat` so we exercise the spawn -> reader thread -> parser pipeline
//! without depending on `claude` or a TTY.

use std::thread::sleep;
use std::time::Duration;

use project_management::agents::{Agent, AgentManager, AgentStatus};
use project_management::store::id::{LeafId, TypePrefix};

fn drain_until_text(agent: &mut Agent, needle: &str, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        agent.poll();
        if screen_contains(agent, needle) {
            return true;
        }
        sleep(Duration::from_millis(20));
    }
    agent.poll();
    screen_contains(agent, needle)
}

fn screen_contains(agent: &Agent, needle: &str) -> bool {
    let parser = agent.parser();
    let guard = parser.lock().unwrap();
    guard.screen().contents().contains(needle)
}

#[test]
fn spawning_echo_renders_output_into_the_pty_screen() {
    let leaf = LeafId::new(TypePrefix::Task, 1);
    let mut agent = Agent::spawn(leaf, "echo hello-from-agent", None, &[]).expect("spawn echo");

    assert!(
        drain_until_text(&mut agent, "hello-from-agent", Duration::from_secs(3)),
        "expected echo output to appear in the PTY within 3s"
    );

    // The shell-c command exits after echo; status flips to Exited
    // once the reader thread sees EOF.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        agent.poll();
        if agent.status == AgentStatus::Exited {
            return;
        }
        sleep(Duration::from_millis(20));
    }
    assert_eq!(agent.status, AgentStatus::Exited, "child should exit");
}

#[test]
fn writing_to_pty_round_trips_through_cat() {
    let leaf = LeafId::new(TypePrefix::Task, 2);
    let mut agent = Agent::spawn(leaf, "cat", None, &[]).expect("spawn cat");

    agent.write(b"thunder-roundtrip\r");

    assert!(
        drain_until_text(&mut agent, "thunder-roundtrip", Duration::from_secs(3)),
        "cat should echo our input back into its own stdout"
    );

    agent.kill();
}

/// Regression for the v0.3.7 writer-cache bug. `portable-pty`'s
/// `take_writer` consumes the writer; calling it on every keystroke
/// silently dropped every write after the first. The fix caches the
/// writer at spawn time. This test sends two distinct payloads in
/// sequence and asserts both come back from cat - if the second one
/// is missing the cache regressed.
#[test]
fn writing_to_pty_works_across_multiple_keystrokes() {
    let leaf = LeafId::new(TypePrefix::Task, 22);
    let mut agent = Agent::spawn(leaf, "cat", None, &[]).expect("spawn cat");

    agent.write(b"first-write\r");
    assert!(
        drain_until_text(&mut agent, "first-write", Duration::from_secs(2)),
        "first write must round-trip"
    );

    agent.write(b"second-write\r");
    assert!(
        drain_until_text(&mut agent, "second-write", Duration::from_secs(2)),
        "second write must also round-trip (regression: take_writer was being called per-keystroke)"
    );

    agent.kill();
}

#[test]
fn manager_enforces_one_agent_per_ticket() {
    let leaf = LeafId::new(TypePrefix::Task, 3);
    let mut manager = AgentManager::new();
    manager
        .spawn(leaf, "cat", None, &[])
        .expect("first spawn ok");
    let second = manager.spawn(leaf, "cat", None, &[]);
    assert!(matches!(
        second,
        Err(project_management::agents::AgentError::AlreadyExists)
    ));
    manager.close(leaf);
    // After close the slot is free again.
    manager
        .spawn(leaf, "cat", None, &[])
        .expect("post-close spawn ok");
    manager.close(leaf);
}

#[test]
fn scope_env_injects_thunder_scope_for_child_processes() {
    let leaf = LeafId::new(TypePrefix::Epic, 9);
    let env = project_management::agents::scope_env(leaf, "EPC9 checkouts");
    let scope = env
        .iter()
        .find(|(k, _)| k == "THUNDER_SCOPE")
        .expect("scope env present");
    assert_eq!(scope.1, "EPC9");
    let label = env.iter().find(|(k, _)| k == "THUNDER_LABEL").unwrap();
    assert_eq!(label.1, "EPC9 checkouts");
    let pm_ticket = env.iter().find(|(k, _)| k == "PM_TICKET").unwrap();
    assert_eq!(pm_ticket.1, "EPC9");
}
