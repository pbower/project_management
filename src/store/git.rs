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

use std::path::{Path, PathBuf};

use git2::{IndexAddOption, Repository, RepositoryInitOptions, Signature, Time};

/// Result type used across the git layer.
pub type GitResult<T> = Result<T, GitError>;

/// Errors emitted by the git layer.
#[derive(Debug)]
pub enum GitError {
    /// Wraps git2 errors.
    Git(git2::Error),
    /// I/O failure while preparing a commit (e.g. could not resolve the
    /// workspace path against the repo workdir).
    Io(std::io::Error),
    /// The repo has no workdir (a bare repo). The PM workspace assumes a
    /// non-bare repository.
    BareRepository,
    /// The workspace lives outside the repository workdir.
    WorkspaceOutsideRepo,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Git(e) => write!(f, "git: {e}"),
            GitError::Io(e) => write!(f, "git io: {e}"),
            GitError::BareRepository => write!(f, "git: workspace must live in a non-bare repository"),
            GitError::WorkspaceOutsideRepo => write!(f, "git: workspace is not inside the discovered repository"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self { GitError::Git(e) }
}

impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self { GitError::Io(e) }
}

/// Open the git repository that should hold `pm_dir`. Discovery walks up from
/// `pm_dir` looking for an enclosing repo; if none is found a fresh
/// repository is initialised at `pm_dir` itself so the workspace is
/// self-contained.
pub fn ensure_repo(pm_dir: &Path) -> GitResult<Repository> {
    if let Ok(repo) = Repository::discover(pm_dir) {
        return Ok(repo);
    }
    let mut opts = RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = Repository::init_opts(pm_dir, &opts)?;
    Ok(repo)
}

/// Stage every change under `pm_dir` (relative to the repository workdir) and
/// write a commit with `message`. The author/committer signature defaults to
/// `pm <pm@workspace>` so commits are recognisable without overriding the
/// user's normal git identity in other repositories.
pub fn commit_workspace(pm_dir: &Path, message: &str) -> GitResult<git2::Oid> {
    let repo = ensure_repo(pm_dir)?;
    let oid = commit_with_repo(&repo, pm_dir, message)?;
    Ok(oid)
}

fn commit_with_repo(repo: &Repository, pm_dir: &Path, message: &str) -> GitResult<git2::Oid> {
    let workdir = repo.workdir().ok_or(GitError::BareRepository)?;
    let pm_dir_canonical = canonicalise_or_self(pm_dir);
    let workdir_canonical = canonicalise_or_self(workdir);

    let rel_pm = pm_dir_canonical
        .strip_prefix(&workdir_canonical)
        .map_err(|_| GitError::WorkspaceOutsideRepo)?;

    // Stage everything under the workspace. The `*` glob is intentional: it
    // covers add/modify/delete so a removed ticket directory is recorded too.
    let mut index = repo.index()?;
    let pathspec = if rel_pm.as_os_str().is_empty() {
        PathBuf::from("*")
    } else {
        rel_pm.join("*")
    };
    let pathspec_str = pathspec.to_string_lossy().into_owned();
    index.add_all(
        std::iter::once(pathspec_str.as_str()),
        IndexAddOption::DEFAULT,
        None,
    )?;
    // Mirror deletions too.
    index.update_all(
        std::iter::once(pathspec_str.as_str()),
        None,
    )?;
    index.write()?;
    let tree_id = index.write_tree()?;

    // Skip the commit when the staged tree matches HEAD - there is nothing
    // new to record.
    if let Ok(head) = repo.head() {
        if let Some(target) = head.target() {
            if let Ok(commit) = repo.find_commit(target) {
                if commit.tree_id() == tree_id {
                    return Ok(commit.id());
                }
            }
        }
    }

    let tree = repo.find_tree(tree_id)?;
    let signature = identity()?;
    let parents: Vec<git2::Commit> = match repo.head() {
        Ok(head) => match head.target() {
            Some(oid) => vec![repo.find_commit(oid)?],
            None => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

    let oid = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )?;
    Ok(oid)
}

/// Construct the signature used for PM-driven commits. Falls back to a
/// deterministic identity so headless test runs without a configured git
/// user.name produce reproducible commits.
fn identity() -> GitResult<Signature<'static>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let when = Time::new(now, 0);
    Ok(Signature::new("pm", "pm@workspace", &when)?)
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

    #[test]
    fn ensure_repo_initialises_when_missing() {
        let dir = tmp_dir();
        let repo = ensure_repo(&dir).unwrap();
        assert!(repo.workdir().is_some());
        // Re-call returns the same repo (idempotent).
        let _again = ensure_repo(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_repo_discovers_existing_parent_repo() {
        let parent = tmp_dir();
        Repository::init(&parent).unwrap();
        let pm_dir = parent.join("workspace");
        std::fs::create_dir_all(&pm_dir).unwrap();

        let repo = ensure_repo(&pm_dir).unwrap();
        assert_eq!(
            canonicalise_or_self(repo.workdir().unwrap()),
            canonicalise_or_self(&parent),
        );
        std::fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn commit_workspace_writes_one_commit() {
        let dir = tmp_dir();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let oid1 = commit_workspace(&dir, "pm: initial").unwrap();
        assert!(!oid1.is_zero());

        // Make a change and commit again.
        std::fs::write(dir.join("a.txt"), b"world").unwrap();
        let oid2 = commit_workspace(&dir, "pm: update").unwrap();
        assert_ne!(oid1, oid2);

        // No change -> same oid (no-op).
        let oid3 = commit_workspace(&dir, "pm: noop").unwrap();
        assert_eq!(oid2, oid3, "no staged changes must not produce a commit");

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
