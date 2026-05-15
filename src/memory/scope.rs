//! Scope and path resolution for memory files.
//!
//! Three on-disk locations, one type system. [`Scope`] is the tag callers
//! pick by name; [`MemoryLocation`] is the resolved path triple (directory
//! for the tier plus the per-name file path). [`MemoryType`] captures the
//! Claude Code auto-memory `metadata.type` field, kept here so callers do
//! not have to round-trip through a free-form string.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::id::LeafId;

/// Which tier a memory file lives at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// `~/.claude/projects/<encoded-cwd>/memory/`. Claude Code owns these;
    /// PM reads freely but only writes on a promote-to-user back-reference.
    User,
    /// `<.pm>/projects/<PRJ>/memories/`. Team-shared, committed via git.
    Project,
    /// `<.pm>/.../<ticket-dir>/memories/`. Per-ticket, committed alongside
    /// the ticket's CLAUDE.md.
    Ticket,
}

impl Scope {
    /// Parse a CLI-friendly scope string. Recognises `user`, `project`,
    /// `ticket`. Returns `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "user" => Some(Scope::User),
            "project" => Some(Scope::Project),
            "ticket" => Some(Scope::Ticket),
            _ => None,
        }
    }

    /// Canonical CLI label (`user` / `project` / `ticket`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Ticket => "ticket",
        }
    }
}

/// The `metadata.type` field carried in a memory file's front-matter.
/// Mirrors the Claude Code auto-memory types so files round-trip cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    /// Parse a CLI-friendly type string. Recognises `user`, `feedback`,
    /// `project`, `reference`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "user" => Some(MemoryType::User),
            "feedback" => Some(MemoryType::Feedback),
            "project" => Some(MemoryType::Project),
            "reference" => Some(MemoryType::Reference),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }
}

/// A resolved memory location: the tier's directory plus the path to a
/// specific memory file within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLocation {
    pub scope: Scope,
    pub directory: PathBuf,
    pub file: PathBuf,
}

/// Encode an absolute path the way Claude Code does for its
/// `~/.claude/projects/` namespace: leading slash and every interior slash
/// become hyphens. `/home/pbow/pm` -> `-home-pbow-pm`.
pub fn encode_cwd(cwd: &Path) -> String {
    let mut s = String::new();
    for ch in cwd.to_string_lossy().chars() {
        if ch == '/' {
            s.push('-');
        } else {
            s.push(ch);
        }
    }
    s
}

/// Build the user-tier memory directory for `cwd`. The directory may not
/// exist yet; callers create it on demand when writing.
pub fn user_dir(home: &Path, cwd: &Path) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(encode_cwd(cwd))
        .join("memory")
}

/// Build the user-tier file path for `name` in the cwd's project namespace.
pub fn user_file(home: &Path, cwd: &Path, name: &str) -> MemoryLocation {
    let directory = user_dir(home, cwd);
    let file = directory.join(format!("{name}.md"));
    MemoryLocation { scope: Scope::User, directory, file }
}

/// Build the project-tier directory for a given `PRJ` leaf id under
/// `pm_dir/.pm/`.
pub fn project_dir(pm_root: &Path, prj: LeafId) -> PathBuf {
    pm_root
        .join("projects")
        .join(prj.to_string())
        .join("memories")
}

/// Build the project-tier file path for `name` under `pm_root/projects/<PRJ>`.
pub fn project_file(pm_root: &Path, prj: LeafId, name: &str) -> MemoryLocation {
    let directory = project_dir(pm_root, prj);
    let file = directory.join(format!("{name}.md"));
    MemoryLocation { scope: Scope::Project, directory, file }
}

/// Build the ticket-tier directory for a ticket whose CLAUDE.md lives at
/// `ticket_dir`.
pub fn ticket_dir(ticket_dir: &Path) -> PathBuf {
    ticket_dir.join("memories")
}

/// Build the ticket-tier file path for `name` under `ticket_dir`.
pub fn ticket_file(ticket_dir_path: &Path, name: &str) -> MemoryLocation {
    let directory = ticket_dir(ticket_dir_path);
    let file = directory.join(format!("{name}.md"));
    MemoryLocation { scope: Scope::Ticket, directory, file }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;
    use std::path::Path;

    #[test]
    fn scope_parses_canonical_strings() {
        assert_eq!(Scope::parse("user"), Some(Scope::User));
        assert_eq!(Scope::parse("project"), Some(Scope::Project));
        assert_eq!(Scope::parse("ticket"), Some(Scope::Ticket));
        assert_eq!(Scope::parse("USER"), Some(Scope::User));
        assert_eq!(Scope::parse("nonsense"), None);
    }

    #[test]
    fn scope_round_trips_to_string() {
        for s in [Scope::User, Scope::Project, Scope::Ticket] {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn memory_type_parses_canonical_strings() {
        assert_eq!(MemoryType::parse("user"), Some(MemoryType::User));
        assert_eq!(MemoryType::parse("feedback"), Some(MemoryType::Feedback));
        assert_eq!(MemoryType::parse("project"), Some(MemoryType::Project));
        assert_eq!(MemoryType::parse("reference"), Some(MemoryType::Reference));
        assert_eq!(MemoryType::parse("nope"), None);
    }

    #[test]
    fn encode_cwd_replaces_slashes_with_hyphens() {
        assert_eq!(encode_cwd(Path::new("/home/pbow/pm")), "-home-pbow-pm");
        assert_eq!(encode_cwd(Path::new("/tmp")), "-tmp");
        assert_eq!(encode_cwd(Path::new("/a/b/c/d")), "-a-b-c-d");
    }

    #[test]
    fn user_paths_resolve_under_claude_projects() {
        let loc = user_file(Path::new("/home/me"), Path::new("/home/me/pm"), "feedback-testing");
        assert_eq!(
            loc.directory,
            PathBuf::from("/home/me/.claude/projects/-home-me-pm/memory"),
        );
        assert_eq!(loc.file.file_name().unwrap(), "feedback-testing.md");
        assert_eq!(loc.scope, Scope::User);
    }

    #[test]
    fn project_paths_resolve_under_prj_leaf() {
        let prj = LeafId::new(TypePrefix::Project, 1);
        let loc = project_file(Path::new("/work/.pm"), prj, "auth-stack");
        assert_eq!(
            loc.directory,
            PathBuf::from("/work/.pm/projects/PRJ1/memories"),
        );
        assert_eq!(loc.scope, Scope::Project);
    }

    #[test]
    fn ticket_paths_resolve_under_ticket_dir() {
        let dir = Path::new("/work/.pm/projects/PRJ1/.../tasks/TSK7");
        let loc = ticket_file(dir, "lock-design");
        assert_eq!(
            loc.directory,
            dir.join("memories"),
        );
        assert_eq!(loc.scope, Scope::Ticket);
    }
}
