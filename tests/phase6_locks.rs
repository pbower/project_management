//! Phase 6 acceptance tests: the lock protocol and the activity feed,
//! exercised end-to-end against the compiled `pm` binary so the test surface
//! matches real user and agent flows.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase6-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `pm --db <pm_dir> <args...>` with optional `PM_AGENT_ID`. Returns the
/// raw `Output` so callers can assert on exit status as well as streams.
fn pm_raw(pm_dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_spacecell");
    let mut cmd = Command::new(bin);
    cmd.arg("--db").arg(pm_dir).args(args);
    if let Some(a) = agent {
        cmd.env("PM_AGENT_ID", a);
    }
    cmd.output().expect("invoke pm binary")
}

/// Like [`pm_raw`] but panics if the command exits non-zero - used for the
/// setup steps that must succeed.
fn pm(pm_dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
    let output = pm_raw(pm_dir, agent, args);
    if !output.status.success() {
        panic!(
            "pm {:?} failed (status={}): stdout={} stderr={}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    output
}

/// Build a workspace with a full PRJ -> PRD -> EPC chain plus `task_count`
/// tasks under the epic. Returns the workspace path.
fn workspace_with_tasks(label: &str, task_count: usize) -> PathBuf {
    let dir = tmp_dir(label);
    pm(&dir, None, &["init"]);
    pm(&dir, None, &["add", "--kind", "project", "PM tool"]);
    pm(
        &dir,
        None,
        &["add", "--kind", "product", "Core", "--parent", "PRJ1"],
    );
    pm(
        &dir,
        None,
        &["add", "--kind", "epic", "Checkouts", "--parent", "PRD1"],
    );
    for n in 0..task_count {
        pm(
            &dir,
            None,
            &[
                "add",
                "--kind",
                "task",
                &format!("Task {n}"),
                "--parent",
                "EPC1",
            ],
        );
    }
    dir
}

fn events(pm_dir: &Path) -> Vec<serde_json::Value> {
    let raw = fs::read_to_string(pm_dir.join("events.log")).unwrap_or_default();
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each events.log line is valid JSON"))
        .collect()
}

fn lock_path(pm_dir: &Path, id: &str) -> PathBuf {
    pm_dir.join("locks").join(format!("{id}.lock"))
}

#[test]
fn two_agents_check_out_different_tickets_concurrently() {
    let dir = workspace_with_tasks("two-agents", 2);

    pm(
        &dir,
        Some("claude-a"),
        &["checkout", "TSK1", "--intent", "ttl"],
    );
    pm(
        &dir,
        Some("claude-b"),
        &["checkout", "TSK2", "--intent", "heartbeat"],
    );

    // Both lock files exist and credit the right agents.
    assert!(lock_path(&dir, "TSK1").is_file());
    assert!(lock_path(&dir, "TSK2").is_file());
    let l1 = fs::read_to_string(lock_path(&dir, "TSK1")).unwrap();
    let l2 = fs::read_to_string(lock_path(&dir, "TSK2")).unwrap();
    assert!(
        l1.contains("\"claude-a\""),
        "TSK1 should be held by claude-a: {l1}"
    );
    assert!(
        l2.contains("\"claude-b\""),
        "TSK2 should be held by claude-b: {l2}"
    );

    // `pm locks` lists both.
    let out = pm(&dir, None, &["locks"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("TSK1") && stdout.contains("claude-a"),
        "locks output: {stdout}"
    );
    assert!(
        stdout.contains("TSK2") && stdout.contains("claude-b"),
        "locks output: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn soft_lock_overlap_warns_but_does_not_block() {
    let dir = workspace_with_tasks("soft-overlap", 1);

    pm(
        &dir,
        Some("claude-a"),
        &["checkout", "TSK1", "--intent", "first"],
    );
    // A second agent checking out the same ticket must succeed (soft lock).
    let out = pm_raw(
        &dir,
        Some("claude-b"),
        &["checkout", "TSK1", "--intent", "second"],
    );
    assert!(
        out.status.success(),
        "soft overlap must not block the checkout"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already checked out by claude-a"),
        "expected an overlap warning naming claude-a, got: {stderr}",
    );
    // Last-writer-wins: claude-b now holds the lock.
    let lock = fs::read_to_string(lock_path(&dir, "TSK1")).unwrap();
    assert!(
        lock.contains("\"claude-b\""),
        "claude-b should now hold TSK1: {lock}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn stale_lock_is_reaped_by_pm_locks() {
    let dir = workspace_with_tasks("stale-locks", 1);
    pm(&dir, Some("claude-a"), &["checkout", "TSK1"]);

    // Force the heartbeat far into the past so the lock is well past its TTL.
    let path = lock_path(&dir, "TSK1");
    let mut lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    lock["last_heartbeat"] = serde_json::json!("2000-01-01T00:00:00Z");
    fs::write(&path, serde_json::to_string_pretty(&lock).unwrap()).unwrap();

    let out = pm(&dir, None, &["locks"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("reaped stale lock on TSK1"),
        "locks output: {stdout}"
    );
    assert!(
        !path.exists(),
        "stale lock file should be gone after `pm locks`"
    );
    // The reap is recorded in the activity feed.
    assert!(
        events(&dir)
            .iter()
            .any(|e| e["verb"] == "lock-reaped" && e["id"] == "TSK1"),
        "expected a lock-reaped event for TSK1",
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn stale_lock_is_reaped_by_pm_doctor() {
    let dir = workspace_with_tasks("stale-doctor", 1);
    pm(&dir, Some("claude-a"), &["checkout", "TSK1"]);

    let path = lock_path(&dir, "TSK1");
    let mut lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    lock["last_heartbeat"] = serde_json::json!("2000-01-01T00:00:00Z");
    fs::write(&path, serde_json::to_string_pretty(&lock).unwrap()).unwrap();

    let out = pm(&dir, None, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("reaped stale lock on TSK1"),
        "doctor output: {stdout}"
    );
    assert!(
        !path.exists(),
        "stale lock file should be gone after `pm doctor`"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn heartbeat_keeps_a_lock_alive() {
    let dir = workspace_with_tasks("heartbeat", 1);
    pm(&dir, Some("claude-a"), &["checkout", "TSK1"]);

    // Age the heartbeat but not past the TTL; refresh it; the lock survives.
    let path = lock_path(&dir, "TSK1");
    let mut lock: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let before = lock["last_heartbeat"].as_str().unwrap().to_string();
    lock["last_heartbeat"] = serde_json::json!("2020-06-01T00:00:00Z");
    fs::write(&path, serde_json::to_string_pretty(&lock).unwrap()).unwrap();

    pm(&dir, Some("claude-a"), &["heartbeat", "TSK1"]);
    let refreshed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let after = refreshed["last_heartbeat"].as_str().unwrap();
    assert_ne!(
        after, "2020-06-01T00:00:00Z",
        "heartbeat must bump last_heartbeat"
    );
    assert!(
        after > before.as_str(),
        "refreshed heartbeat must be newer than the original"
    );

    // Heartbeat on a ticket with no lock fails cleanly.
    let out = pm_raw(&dir, None, &["heartbeat", "TSK1"]);
    let _ = out; // TSK1 still locked here; instead probe a never-locked ticket.
    let dir2 = workspace_with_tasks("heartbeat-nolock", 1);
    let out = pm_raw(&dir2, None, &["heartbeat", "TSK1"]);
    assert!(
        !out.status.success(),
        "heartbeat on an unlocked ticket should fail"
    );
    fs::remove_dir_all(&dir2).ok();

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn events_log_records_every_verb_with_actor_and_id() {
    let dir = tmp_dir("events");
    pm(&dir, Some("claude-be"), &["init"]);
    pm(
        &dir,
        Some("claude-be"),
        &["add", "--kind", "project", "PM tool"],
    );
    pm(
        &dir,
        Some("claude-be"),
        &["add", "--kind", "product", "Core", "--parent", "PRJ1"],
    );
    pm(
        &dir,
        Some("claude-be"),
        &["set-status", "PRD1", "in-progress"],
    );
    pm(&dir, Some("claude-be"), &["priority", "PRD1", "must-have"]);
    pm(&dir, Some("claude-be"), &["tag", "PRD1", "+infra"]);

    let log = events(&dir);
    // Every verb in the session is present.
    let verbs: Vec<&str> = log.iter().map(|e| e["verb"].as_str().unwrap()).collect();
    for expected in ["init", "add", "status", "priority", "tag"] {
        assert!(
            verbs.contains(&expected),
            "events.log missing verb {expected:?}: {verbs:?}"
        );
    }
    // The actor is honoured from PM_AGENT_ID on every line.
    for e in &log {
        assert_eq!(
            e["actor"], "claude-be",
            "every event must credit PM_AGENT_ID"
        );
    }
    // Ticket-scoped verbs carry the right id; init does not.
    let add_prd = log.iter().find(|e| e["verb"] == "add" && e["id"] == "PRD1");
    assert!(add_prd.is_some(), "add event for PRD1 missing");
    assert!(
        add_prd.unwrap()["detail"] == "Core",
        "add detail should be the title"
    );
    let init = log.iter().find(|e| e["verb"] == "init").unwrap();
    assert!(
        init.get("id").is_none() || init["id"].is_null(),
        "init has no ticket id"
    );
    let status = log.iter().find(|e| e["verb"] == "status").unwrap();
    assert_eq!(status["id"], "PRD1");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn events_log_grows_synchronously_per_command() {
    // The feed is appended to before each command returns, so a `tail -f`
    // consumer sees an entry land for every state change as it happens.
    let dir = workspace_with_tasks("feed-growth", 1);
    let after_setup = events(&dir).len();
    assert!(
        after_setup >= 4,
        "init + 3 adds should already be on the feed"
    );

    pm(&dir, Some("claude-a"), &["checkout", "TSK1"]);
    let after_checkout = events(&dir).len();
    assert_eq!(
        after_checkout,
        after_setup + 1,
        "checkout appends exactly one line"
    );

    pm(
        &dir,
        Some("claude-a"),
        &["set-status", "TSK1", "in-progress"],
    );
    assert_eq!(
        events(&dir).len(),
        after_checkout + 1,
        "status appends exactly one line"
    );

    pm(
        &dir,
        Some("claude-a"),
        &["checkin", "TSK1", "--summary", "done"],
    );
    assert_eq!(
        events(&dir).len(),
        after_checkout + 2,
        "checkin appends exactly one line"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn checkin_squashes_the_checkout_span_by_default() {
    let dir = workspace_with_tasks("squash", 1);

    pm(
        &dir,
        Some("claude-a"),
        &["checkout", "TSK1", "--intent", "implement"],
    );
    pm(
        &dir,
        Some("claude-a"),
        &["set-status", "TSK1", "in-progress"],
    );
    pm(&dir, Some("claude-a"), &["priority", "TSK1", "must-have"]);

    let before = git_subjects(&dir);
    // The span produced separate commits.
    assert!(before.iter().any(|s| s == "pm: TSK1 checkout (implement)"));
    assert!(before.iter().any(|s| s == "pm: TSK1 status"));
    assert!(before.iter().any(|s| s == "pm: TSK1 priority"));

    pm(
        &dir,
        Some("claude-a"),
        &["checkin", "TSK1", "--summary", "implemented"],
    );
    let after = git_subjects(&dir);
    // The span collapsed to a single checkin commit.
    assert!(
        after.iter().any(|s| s == "pm: TSK1 checkin (implemented)"),
        "after: {after:?}"
    );
    assert!(
        !after.iter().any(|s| s == "pm: TSK1 checkout (implement)"),
        "checkout commit should be squashed away: {after:?}"
    );
    assert!(
        !after.iter().any(|s| s == "pm: TSK1 status"),
        "status commit should be squashed away: {after:?}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn checkin_granular_keeps_the_span_commits() {
    let dir = workspace_with_tasks("granular", 1);

    pm(
        &dir,
        Some("claude-a"),
        &["checkout", "TSK1", "--intent", "implement"],
    );
    pm(
        &dir,
        Some("claude-a"),
        &["set-status", "TSK1", "in-progress"],
    );
    pm(
        &dir,
        Some("claude-a"),
        &["checkin", "TSK1", "--summary", "implemented", "--granular"],
    );

    let after = git_subjects(&dir);
    // With --granular every span commit survives alongside the checkin.
    assert!(
        after.iter().any(|s| s == "pm: TSK1 checkout (implement)"),
        "after: {after:?}"
    );
    assert!(
        after.iter().any(|s| s == "pm: TSK1 status"),
        "after: {after:?}"
    );
    assert!(
        after.iter().any(|s| s == "pm: TSK1 checkin (implemented)"),
        "after: {after:?}"
    );

    fs::remove_dir_all(&dir).ok();
}

/// Git commit subjects, newest first.
fn git_subjects(repo_root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["log", "--pretty=%s"])
        .output()
        .expect("git log");
    assert!(
        out.status.success(),
        "git log failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|s| s.to_string())
        .collect()
}
