//! Phase 10 acceptance tests: three-tier memory plumbing through the binary.
//!
//! Each test sets a controlled `HOME` and `--db` so the user-tier path is
//! deterministic and isolated. The project- and ticket-tier files land in
//! the workspace's `.pm/` and are committed via the regular ticket write
//! path; the user-tier files are seeded directly because PM refuses to
//! write them (matching the Phase 10 exit criterion).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase10-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir_all(&dir).unwrap();
    // Canonicalize so the path matches what std::env::current_dir() returns
    // inside the spawned `pm` subprocess. On macOS, `std::env::temp_dir()`
    // hands back `/var/folders/...` but the kernel resolves that to
    // `/private/var/folders/...` for the running process; if the test side
    // encoded the un-canonical form and the binary side encoded the
    // canonical form, the user-tier `~/.claude/projects/<encoded-cwd>/`
    // namespaces drift apart and lookups miss.
    fs::canonicalize(&dir).unwrap_or(dir)
}

/// Run `pm --db <pm_dir> <args>` with a controlled `HOME` and `cwd`. The
/// user-tier path then resolves to a predictable subtree under `home_dir`.
fn pm(pm_dir: &Path, home: &Path, args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_pm");
    let mut cmd = Command::new(bin);
    cmd.arg("--db")
        .arg(pm_dir)
        .args(args)
        .env("HOME", home)
        .current_dir(pm_dir);
    cmd.output().expect("invoke pm binary")
}

/// Encode an absolute path the way Claude Code (and PM's user-tier resolver)
/// does for its `~/.claude/projects/` namespace: every slash becomes a
/// hyphen.
fn encode_cwd(p: &Path) -> String {
    p.to_string_lossy().replace('/', "-")
}

fn assert_ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed: status={:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Seed a workspace with one PRJ and one TSK that has the PRJ as ancestor.
/// Returns `(pm_dir, home, prj, tsk)` ids as strings the binary accepts.
fn seed_minimal(label: &str) -> (PathBuf, PathBuf) {
    let pm_dir = tmp_dir(label);
    let home = tmp_dir(&format!("{label}-home"));

    assert_ok(&pm(&pm_dir, &home, &["init"]), "pm init");
    assert_ok(
        &pm(
            &pm_dir,
            &home,
            &["add", "Demo project", "--kind", "project"],
        ),
        "add PRJ",
    );
    assert_ok(
        &pm(
            &pm_dir,
            &home,
            &["add", "Core", "--kind", "product", "--parent", "PRJ1"],
        ),
        "add PRD",
    );
    assert_ok(
        &pm(
            &pm_dir,
            &home,
            &["add", "Checkouts", "--kind", "epic", "--parent", "PRD1"],
        ),
        "add EPC",
    );
    assert_ok(
        &pm(
            &pm_dir,
            &home,
            &["add", "Lock protocol", "--kind", "task", "--parent", "EPC1"],
        ),
        "add TSK",
    );

    (pm_dir, home)
}

#[test]
fn write_project_then_link_and_list() {
    let (pm_dir, home) = seed_minimal("write-link-list");

    let write = pm(
        &pm_dir,
        &home,
        &[
            "memory",
            "write",
            "--scope",
            "project",
            "--ty",
            "project",
            "--name",
            "auth-stack",
            "--desc",
            "Auth conventions",
            "--project",
            "PRJ1",
            "Use bearer JWTs.\n",
        ],
    );
    assert_ok(&write, "memory write project");

    let project_file = pm_dir.join("projects/PRJ1/memories/auth-stack.md");
    assert!(
        project_file.is_file(),
        "project-tier file written: {}",
        project_file.display()
    );

    let link = pm(&pm_dir, &home, &["memory", "link", "TSK1", "auth-stack"]);
    assert_ok(&link, "memory link");
    assert!(
        String::from_utf8_lossy(&link.stdout).contains("[project]"),
        "link output reports project tier: {}",
        String::from_utf8_lossy(&link.stdout),
    );

    let list = pm(&pm_dir, &home, &["memory", "list", "TSK1"]);
    assert_ok(&list, "memory list");
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("auth-stack"),
        "list shows the name: {stdout}"
    );
    assert!(
        stdout.contains("[project]"),
        "list annotates tier: {stdout}"
    );
}

#[test]
fn write_user_scope_is_rejected() {
    let (pm_dir, home) = seed_minimal("reject-user");

    let out = pm(
        &pm_dir,
        &home,
        &[
            "memory",
            "write",
            "--scope",
            "user",
            "--ty",
            "user",
            "--name",
            "do-not-write",
            "body",
        ],
    );
    assert!(!out.status.success(), "user-tier writes must be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("user-tier")
            || stderr.contains("user tier")
            || stderr.contains("does not write"),
        "stderr explains the refusal: {stderr}"
    );

    let user_dir = home
        .join(".claude")
        .join("projects")
        .join(encode_cwd(&pm_dir))
        .join("memory");
    assert!(
        !user_dir.join("do-not-write.md").exists(),
        "no user-tier file created"
    );
}

#[test]
fn unlink_removes_from_front_matter() {
    let (pm_dir, home) = seed_minimal("unlink");
    assert_ok(
        &pm(
            &pm_dir,
            &home,
            &[
                "memory",
                "write",
                "--scope",
                "project",
                "--ty",
                "project",
                "--name",
                "foo",
                "--project",
                "PRJ1",
                "body",
            ],
        ),
        "write project memory",
    );
    assert_ok(
        &pm(&pm_dir, &home, &["memory", "link", "TSK1", "foo"]),
        "link",
    );
    let unlink = pm(&pm_dir, &home, &["memory", "unlink", "TSK1", "foo"]);
    assert_ok(&unlink, "unlink");
    let list = pm(&pm_dir, &home, &["memory", "list", "TSK1"]);
    assert_ok(&list, "memory list after unlink");
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        !stdout.contains("foo"),
        "memory removed from front-matter: {stdout}"
    );
}

#[test]
fn promote_user_to_project_moves_and_leaves_backref() {
    let (pm_dir, home) = seed_minimal("promote-up");

    // Seed a user-tier file directly because PM refuses to write it.
    let user_dir = home
        .join(".claude")
        .join("projects")
        .join(encode_cwd(&pm_dir))
        .join("memory");
    fs::create_dir_all(&user_dir).unwrap();
    let user_file = user_dir.join("config-style.md");
    fs::write(
        &user_file,
        "---\nname: config-style\nmetadata:\n  type: user\n---\n\nUse snake_case.\n",
    )
    .unwrap();

    let out = pm(
        &pm_dir,
        &home,
        &["memory", "promote", "config-style", "--to", "project"],
    );
    assert_ok(&out, "promote up");

    let project_file = pm_dir.join("projects/PRJ1/memories/config-style.md");
    assert!(project_file.is_file(), "project canonical written");
    let project_body = fs::read_to_string(&project_file).unwrap();
    assert!(
        project_body.contains("Use snake_case"),
        "project copy has the content"
    );

    // User-tier file is now a back-reference (type: reference, body
    // pointing at the canonical project path).
    let user_body = fs::read_to_string(&user_file).unwrap();
    assert!(
        user_body.contains("type: reference"),
        "user file demoted to a reference"
    );
    assert!(
        user_body.contains("Canonical:"),
        "user file mentions canonical path"
    );
}

#[test]
fn context_no_memories_flag_suppresses_section() {
    let (pm_dir, home) = seed_minimal("context-flag");
    assert_ok(
        &pm(
            &pm_dir,
            &home,
            &[
                "memory",
                "write",
                "--scope",
                "project",
                "--ty",
                "project",
                "--name",
                "n",
                "--project",
                "PRJ1",
                "memory body\n",
            ],
        ),
        "write memory",
    );
    assert_ok(
        &pm(&pm_dir, &home, &["memory", "link", "TSK1", "n"]),
        "link",
    );

    let with_memories = pm(&pm_dir, &home, &["context", "TSK1"]);
    assert_ok(&with_memories, "pm context (default includes memories)");
    let with_text = String::from_utf8_lossy(&with_memories.stdout);
    assert!(
        with_text.contains("Linked memories"),
        "default context output includes the section: {with_text}"
    );

    let without = pm(&pm_dir, &home, &["context", "TSK1", "--no-memories"]);
    assert_ok(&without, "pm context --no-memories");
    let without_text = String::from_utf8_lossy(&without.stdout);
    assert!(
        !without_text.contains("Linked memories"),
        "--no-memories suppresses the section: {without_text}"
    );
}
