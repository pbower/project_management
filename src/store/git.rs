//! Git integration for the v2 workspace.
//!
//! Every state-mutating verb produces an atomic commit through
//! [`commit_workspace`]. The commit subject follows the pattern documented in
//! PM_DESIGN.md Section 7.4:
//!
//! ```text
//! pm: TSK7 status in_progress
//! pm: PRD1 add ("Core")
//! pm: TSK7 edit (+12 -3 in CLAUDE.md)
//! ```
//!
//! Agents never call git directly. The binary owns every commit, which keeps
//! the audit trail readable and the per-ticket history filterable by `pm log`.
//!
//! This layer shells out to the system `git` binary rather than linking a git
//! library into the executable. That keeps the dependency surface (and thus
//! the supply-chain attack surface) limited to the user's own
//! OS-managed git install.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Result type used across the git layer.
pub type GitResult<T> = Result<T, GitError>;

/// Errors emitted by the git layer.
#[derive(Debug)]
pub enum GitError {
    /// The `git` binary could not be launched (not installed or not on PATH).
    GitNotFound(std::io::Error),
    /// A `git` invocation exited non-zero.
    CommandFailed {
        args: Vec<String>,
        status: Option<i32>,
        stderr: String,
    },
    /// I/O failure while preparing a commit (e.g. resolving a workspace path).
    Io(std::io::Error),
    /// The workspace lives outside the discovered repository workdir.
    WorkspaceOutsideRepo,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::GitNotFound(e) => write!(f, "git: could not run the `git` binary: {e}"),
            GitError::CommandFailed { args, status, stderr } => {
                let code = status.map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
                write!(f, "git {} failed (exit {code}): {}", args.join(" "), stderr.trim())
            }
            GitError::Io(e) => write!(f, "git io: {e}"),
            GitError::WorkspaceOutsideRepo => {
                write!(f, "git: workspace is not inside the discovered repository")
            }
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self { GitError::Io(e) }
}

/// Run a `git` subcommand with `cwd` as the working directory. Returns stdout
/// (trailing newline trimmed) on success, a structured [`GitError`] otherwise.
fn run_git(cwd: &Path, args: &[&str]) -> GitResult<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(GitError::GitNotFound)?;
    if output.status.success() {
        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        while stdout.ends_with('\n') || stdout.ends_with('\r') {
            stdout.pop();
        }
        Ok(stdout)
    } else {
        Err(GitError::CommandFailed {
            args: args.iter().map(|s| s.to_string()).collect(),
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Open the git repository that should hold `pm_dir`. Discovery walks up from
/// `pm_dir` looking for an enclosing repo; if none is found a fresh
/// repository is initialised at `pm_dir` itself so the workspace is
/// self-contained. Returns the repository's working-tree root.
pub fn ensure_repo(pm_dir: &Path) -> GitResult<PathBuf> {
    // `rev-parse --show-toplevel` succeeds when `pm_dir` is inside a repo and
    // prints the working-tree root.
    if let Ok(root) = run_git(pm_dir, &["rev-parse", "--show-toplevel"]) {
        if !root.is_empty() {
            return Ok(PathBuf::from(root));
        }
    }
    // Not inside a repo: initialise one at `pm_dir`. `symbolic-ref` sets the
    // initial branch to `main` before any commit exists, which works on every
    // git version (older releases lack `init -b`).
    run_git(pm_dir, &["init"])?;
    run_git(pm_dir, &["symbolic-ref", "HEAD", "refs/heads/main"])?;
    Ok(pm_dir.to_path_buf())
}

/// Stage every change under `pm_dir` and write a commit with `message`.
///
/// PM's own commits are tool bookkeeping inside `.pm/`, so they pass
/// `--no-verify` (skip the repository's hooks) and disable commit signing -
/// running a project's `pre-commit` or prompting for a GPG passphrase on
/// every `pm` mutation would be wrong. The author/committer identity is set
/// per-invocation to `pm <pm@workspace>` so PM commits are recognisable
/// without touching the user's global git config.
///
/// Returns the resulting commit hash. When nothing is staged (the workspace
/// already matches HEAD) the existing HEAD hash is returned without writing a
/// new commit.
pub fn commit_workspace(pm_dir: &Path, message: &str) -> GitResult<String> {
    let root = ensure_repo(pm_dir)?;

    let rel_pm = workspace_relative_path(&root, pm_dir)?;
    let pathspec = if rel_pm.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel_pm.to_string_lossy().into_owned()
    };

    // Stage adds, modifications, and deletions under the workspace.
    run_git(&root, &["add", "-A", "--", &pathspec])?;

    // `diff --cached --quiet` exits 0 when nothing is staged. In that case the
    // workspace already matches HEAD - return the current commit unchanged.
    let nothing_staged = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["diff", "--cached", "--quiet"])
        .status()
        .map_err(GitError::GitNotFound)?
        .success();
    if nothing_staged {
        // A repo with no commits yet still needs its first commit; only treat
        // "nothing staged" as a no-op when a HEAD already exists.
        if let Ok(head) = run_git(&root, &["rev-parse", "HEAD"]) {
            return Ok(head);
        }
    }

    run_git(
        &root,
        &[
            "-c", "user.name=pm",
            "-c", "user.email=pm@workspace",
            "-c", "commit.gpgsign=false",
            "commit",
            "--no-verify",
            "--allow-empty",
            "-m", message,
        ],
    )?;
    run_git(&root, &["rev-parse", "HEAD"])
}

/// Compute `pm_dir` relative to the repository `root`. Both paths are
/// canonicalised first so symlinked temp dirs resolve consistently.
fn workspace_relative_path(root: &Path, pm_dir: &Path) -> GitResult<PathBuf> {
    let root_canon = canonicalise_or_self(root);
    let pm_canon = canonicalise_or_self(pm_dir);
    pm_canon
        .strip_prefix(&root_canon)
        .map(|p| p.to_path_buf())
        .map_err(|_| GitError::WorkspaceOutsideRepo)
}

fn canonicalise_or_self(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Build a structured commit-subject for a ticket mutation. Callers pass the
/// verb (`status`, `add`, `edit`, ...) and an optional short summary that goes
/// in parentheses. The leaf id is rendered via its `Display`.
pub fn subject(leaf: &impl std::fmt::Display, verb: &str, summary: Option<&str>) -> String {
    match summary {
        Some(s) if !s.is_empty() => format!("pm: {leaf} {verb} ({s})"),
        _ => format!("pm: {leaf} {verb}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-git-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Shell out to `git` for test setup so the tests do not depend on the
    /// module under test to build their fixtures.
    fn git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn ensure_repo_initialises_when_missing() {
        let dir = tmp_dir();
        let root = ensure_repo(&dir).unwrap();
        assert_eq!(canonicalise_or_self(&root), canonicalise_or_self(&dir));
        assert!(dir.join(".git").exists(), ".git missing after ensure_repo");
        // Re-call is idempotent and discovers the same repo.
        let again = ensure_repo(&dir).unwrap();
        assert_eq!(canonicalise_or_self(&again), canonicalise_or_self(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_repo_discovers_existing_parent_repo() {
        let parent = tmp_dir();
        git(&parent, &["init"]);
        let pm_dir = parent.join("workspace");
        std::fs::create_dir_all(&pm_dir).unwrap();

        let root = ensure_repo(&pm_dir).unwrap();
        assert_eq!(canonicalise_or_self(&root), canonicalise_or_self(&parent));
        // The nested workspace must not get its own repo.
        assert!(!pm_dir.join(".git").exists());
        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn commit_workspace_writes_one_commit() {
        let dir = tmp_dir();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let hash1 = commit_workspace(&dir, "pm: initial").unwrap();
        assert_eq!(hash1.len(), 40, "expected a full commit hash, got {hash1:?}");

        // Make a change and commit again.
        std::fs::write(dir.join("a.txt"), b"world").unwrap();
        let hash2 = commit_workspace(&dir, "pm: update").unwrap();
        assert_ne!(hash1, hash2);

        // No change -> same hash (no-op).
        let hash3 = commit_workspace(&dir, "pm: noop").unwrap();
        assert_eq!(hash2, hash3, "no staged changes must not produce a commit");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn commit_workspace_skips_repo_hooks() {
        // A pre-commit hook that always fails must not block PM's own commits.
        let dir = tmp_dir();
        ensure_repo(&dir).unwrap();
        let hooks = dir.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let hash = commit_workspace(&dir, "pm: with failing hook present").unwrap();
        assert_eq!(hash.len(), 40);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subject_renders_known_shapes() {
        struct Leaf(&'static str);
        impl std::fmt::Display for Leaf {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
        }
        assert_eq!(subject(&Leaf("TSK7"), "status", None), "pm: TSK7 status");
        assert_eq!(
            subject(&Leaf("TSK7"), "add", Some("Lock protocol")),
            "pm: TSK7 add (Lock protocol)",
        );
    }
}
