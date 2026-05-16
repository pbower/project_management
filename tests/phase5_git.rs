//! Phase 5 acceptance tests. Exercises the git integration end-to-end
//! against the compiled `pm` binary so the test surface matches user flows.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase5-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn pm(pm_dir: &Path, args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_pm");
    let output = Command::new(bin)
        .arg("--db")
        .arg(pm_dir)
        .args(args)
        .output()
        .expect("invoke pm binary");
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

fn git_log_subjects(repo_root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["log", "--pretty=%s"])
        .output()
        .expect("git log");
    assert!(
        output.status.success(),
        "git log failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn pm_init_in_fresh_dir_creates_a_git_repo() {
    let dir = tmp_dir("init-fresh");
    pm(&dir, &["init"]);

    let git_dir = dir.join(".git");
    assert!(
        git_dir.is_dir(),
        ".git missing after pm init at {}",
        dir.display()
    );

    let subjects = git_log_subjects(&dir);
    assert_eq!(subjects, vec!["pm: init".to_string()]);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn pm_init_reuses_existing_parent_repo() {
    let parent = tmp_dir("init-existing");
    // Initialise a git repo at the parent, with one prior commit so HEAD is set.
    let status = Command::new("git")
        .arg("-C")
        .arg(&parent)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());
    fs::write(parent.join("preexisting.txt"), b"hello").unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&parent)
        .args(["add", "."])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(&parent)
        .args([
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            "preexisting",
        ])
        .status()
        .unwrap();

    let workspace = parent.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    pm(&workspace, &["init"]);

    // Workspace must NOT have its own .git - the parent repo is reused.
    assert!(
        !workspace.join(".git").exists(),
        "workspace should not have a nested .git"
    );

    // git log at the parent now shows the pm init commit on top of the
    // preexisting commit.
    let subjects = git_log_subjects(&parent);
    assert_eq!(subjects[0], "pm: init");
    assert!(subjects.iter().any(|s| s == "preexisting"));

    fs::remove_dir_all(&parent).ok();
}

#[test]
fn every_state_mutation_produces_a_structured_commit() {
    let dir = tmp_dir("mutations");
    pm(&dir, &["init"]);
    pm(&dir, &["add", "--kind", "project", "PM tool"]);
    pm(
        &dir,
        &["add", "--kind", "product", "Core", "--parent", "PRJ1"],
    );
    pm(&dir, &["set-status", "PRD1", "in-progress"]);
    pm(&dir, &["priority", "PRD1", "must-have"]);
    pm(&dir, &["tag", "PRD1", "+infra", "-draft"]);

    let subjects = git_log_subjects(&dir);
    // Subjects come back newest-first.
    let expected_order = vec![
        "pm: PRD1 tag",
        "pm: PRD1 priority",
        "pm: PRD1 status",
        "pm: PRD1 add (Core)",
        "pm: PRJ1 add (PM tool)",
        "pm: init",
    ];
    assert_eq!(subjects, expected_order, "got {subjects:?}");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn pm_log_filters_to_ticket_subtree() {
    let dir = tmp_dir("log-filter");
    pm(&dir, &["init"]);
    pm(&dir, &["add", "--kind", "project", "PM tool"]);
    pm(&dir, &["add", "--kind", "project", "Other"]);
    pm(
        &dir,
        &["add", "--kind", "product", "Core", "--parent", "PRJ1"],
    );
    pm(&dir, &["set-status", "PRJ2", "in-progress"]);

    let output = pm(&dir, &["log", "PRJ1"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();

    // PRJ1's slice should include its own creation and the PRD1 creation
    // (which lives under projects/PRJ1/products/PRD1/), but not the PRJ2
    // mutations.
    assert!(
        lines.iter().any(|l| l.contains("pm: PRJ1 add")),
        "missing PRJ1 add in {stdout}"
    );
    assert!(
        lines.iter().any(|l| l.contains("pm: PRD1 add")),
        "missing PRD1 add in PRJ1 log: {stdout}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("PRJ2")),
        "PRJ2 should not appear in PRJ1 log: {stdout}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("pm: init")),
        "init should not appear in PRJ1 log: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}
