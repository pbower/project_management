//! Operations across the three memory tiers.
//!
//! [`list_at_scope`] enumerates the memory files in one tier. [`list_all`]
//! enumerates every tier and tags each hit with its [`Scope`]. [`lookup_by_name`]
//! resolves a name to one tier using the project-first collision rule.
//! [`write_memory`] writes a memory file at the requested scope.
//! [`promote_memory`] moves a memory between the user and project tiers and
//! leaves a small back-reference at the original tier.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::store::id::LeafId;

use super::file::{MemoryFile, MemoryFileError, MemoryFrontMatter, MemoryMetadata};
use super::scope::{
    project_dir, project_file, ticket_dir, ticket_file, user_dir, user_file,
    MemoryLocation, MemoryType, Scope,
};

/// One hit during a tiered lookup: the resolved file location and the
/// parsed memory file at that location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHit {
    pub location: MemoryLocation,
    pub file: MemoryFile,
}

/// Resources needed to resolve memory paths at any tier. Callers build one
/// of these once per command from `pm_dir` and the cwd / home / project id
/// they have on hand, then pass it through subsequent calls.
#[derive(Debug, Clone)]
pub struct MemoryContext {
    pub home: PathBuf,
    pub cwd: PathBuf,
    /// `.pm/` root directory.
    pub pm_root: PathBuf,
    /// PRJ leaf for the active workspace. When `None`, project-tier
    /// operations report [`StoreError::NoActiveProject`].
    pub active_project: Option<LeafId>,
    /// On-disk path of the active ticket (the directory containing its
    /// `CLAUDE.md`). When `None`, ticket-tier operations report
    /// [`StoreError::NoActiveTicket`].
    pub active_ticket_dir: Option<PathBuf>,
}

impl MemoryContext {
    pub fn user_directory(&self) -> PathBuf {
        user_dir(&self.home, &self.cwd)
    }
    pub fn user_path(&self, name: &str) -> MemoryLocation {
        user_file(&self.home, &self.cwd, name)
    }
    pub fn project_directory(&self) -> Result<PathBuf, StoreError> {
        let prj = self.active_project.ok_or(StoreError::NoActiveProject)?;
        Ok(project_dir(&self.pm_root, prj))
    }
    pub fn project_path(&self, name: &str) -> Result<MemoryLocation, StoreError> {
        let prj = self.active_project.ok_or(StoreError::NoActiveProject)?;
        Ok(project_file(&self.pm_root, prj, name))
    }
    pub fn ticket_directory(&self) -> Result<PathBuf, StoreError> {
        let dir = self
            .active_ticket_dir
            .as_deref()
            .ok_or(StoreError::NoActiveTicket)?;
        Ok(ticket_dir(dir))
    }
    pub fn ticket_path(&self, name: &str) -> Result<MemoryLocation, StoreError> {
        let dir = self
            .active_ticket_dir
            .as_deref()
            .ok_or(StoreError::NoActiveTicket)?;
        Ok(ticket_file(dir, name))
    }
}

/// List every `*.md` memory file inside the given tier's directory. Missing
/// directories return an empty list - they're a normal state for a tier the
/// user has never written to.
pub fn list_at_scope(ctx: &MemoryContext, scope: Scope) -> Result<Vec<MemoryHit>, StoreError> {
    let dir = match scope {
        Scope::User => ctx.user_directory(),
        Scope::Project => ctx.project_directory()?,
        Scope::Ticket => ctx.ticket_directory()?,
    };
    list_files_in(&dir, scope)
}

/// List memories at every tier the context can resolve. Tiers the context
/// cannot resolve (e.g. project-tier without an active PRJ) are skipped
/// without erroring.
pub fn list_all(ctx: &MemoryContext) -> Result<Vec<MemoryHit>, StoreError> {
    let mut all: Vec<MemoryHit> = Vec::new();
    all.extend(list_files_in(&ctx.user_directory(), Scope::User)?);
    if let Some(prj) = ctx.active_project {
        all.extend(list_files_in(&project_dir(&ctx.pm_root, prj), Scope::Project)?);
    }
    if let Some(dir) = &ctx.active_ticket_dir {
        all.extend(list_files_in(&ticket_dir(dir), Scope::Ticket)?);
    }
    Ok(all)
}

/// Resolve a memory by name using the project-first collision rule.
/// Project tier wins over ticket tier over user tier when multiple files
/// share a name. Returns `None` if no tier has a matching memory.
pub fn lookup_by_name(ctx: &MemoryContext, name: &str) -> Result<Option<MemoryHit>, StoreError> {
    if let Some(prj) = ctx.active_project {
        let loc = project_file(&ctx.pm_root, prj, name);
        if loc.file.is_file() {
            return Ok(Some(read_hit(loc)?));
        }
    }
    if let Some(dir) = &ctx.active_ticket_dir {
        let loc = ticket_file(dir, name);
        if loc.file.is_file() {
            return Ok(Some(read_hit(loc)?));
        }
    }
    let loc = user_file(&ctx.home, &ctx.cwd, name);
    if loc.file.is_file() {
        return Ok(Some(read_hit(loc)?));
    }
    Ok(None)
}

/// Write a memory file at the chosen scope. Creates the tier's directory
/// if it does not exist. Returns the resolved location.
///
/// The user tier is off-limits to a direct write. Per PM_BUILD_PLAN.md
/// Phase 10 the user tier stays under Claude Code's auto-memory control;
/// PM only writes there through [`promote_memory`] (which leaves a
/// back-reference on user->project, or restores canonical content on
/// project->user). A request to write at `Scope::User` returns
/// [`StoreError::UserTierWriteForbidden`].
pub fn write_memory(
    ctx: &MemoryContext,
    scope: Scope,
    name: &str,
    kind: MemoryType,
    description: Option<String>,
    body: &str,
) -> Result<MemoryLocation, StoreError> {
    let location = match scope {
        Scope::User => return Err(StoreError::UserTierWriteForbidden),
        Scope::Project => ctx.project_path(name)?,
        Scope::Ticket => ctx.ticket_path(name)?,
    };
    let mut mf = MemoryFile::new(name, kind, body);
    mf.front_matter.description = description;
    mf.write(&location.file).map_err(StoreError::File)?;
    Ok(location)
}

/// Write a memory file at any tier, including user. Used internally by
/// [`promote_memory`] for the canonical/back-reference writes that the
/// promote contract permits at the user tier. External callers go through
/// [`write_memory`] instead.
fn write_memory_unchecked(
    location: &MemoryLocation,
    mf: &MemoryFile,
) -> Result<(), StoreError> {
    mf.write(&location.file).map_err(StoreError::File)
}

/// Promote a memory between scopes. Currently supports user <-> project.
///
/// User -> Project:
///   1. Copy the user memory's content into `<.pm>/projects/<PRJ>/memories/`.
///   2. Replace the user-tier file with a small back-reference (type
///      `reference`) pointing at the project-tier path so Claude Code's
///      auto-loader still surfaces the existence of the memory.
///
/// Project -> User: the inverse. The project-tier file is removed; the
/// user-tier file becomes the canonical copy.
pub fn promote_memory(
    ctx: &MemoryContext,
    name: &str,
    target: Scope,
) -> Result<PromotionOutcome, StoreError> {
    match target {
        Scope::Project => promote_user_to_project(ctx, name),
        Scope::User => promote_project_to_user(ctx, name),
        Scope::Ticket => Err(StoreError::UnsupportedPromotion("ticket-tier promotion is not supported".to_string())),
    }
}

/// Result of a successful promote operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionOutcome {
    pub source: MemoryLocation,
    pub target: MemoryLocation,
    /// The back-reference written at the source tier so the old location
    /// still resolves through Claude Code's auto-loader. `None` when no
    /// back-reference applies (e.g. project -> user direction where the
    /// canonical now lives where the user expects).
    pub backref: Option<MemoryLocation>,
}

fn promote_user_to_project(
    ctx: &MemoryContext,
    name: &str,
) -> Result<PromotionOutcome, StoreError> {
    let prj = ctx.active_project.ok_or(StoreError::NoActiveProject)?;
    let source_loc = user_file(&ctx.home, &ctx.cwd, name);
    let target_loc = project_file(&ctx.pm_root, prj, name);

    if !source_loc.file.is_file() {
        return Err(StoreError::NotFound(format!(
            "user-tier memory {name:?} not found at {}",
            source_loc.file.display()
        )));
    }
    if target_loc.file.exists() {
        return Err(StoreError::TargetExists(target_loc.file.clone()));
    }

    // Read the canonical content from the user tier, write it to the project
    // tier, then replace the source with a back-reference.
    let original = MemoryFile::read(&source_loc.file).map_err(StoreError::File)?;
    write_memory_unchecked(&target_loc, &original)?;

    let backref_body = format!(
        "This memory was promoted to project tier on {}.\n\nCanonical: {}\n",
        chrono::Utc::now().format("%Y-%m-%d"),
        target_loc.file.display(),
    );
    let backref = MemoryFile {
        front_matter: MemoryFrontMatter {
            name: original.front_matter.name.clone(),
            description: Some(format!("Promoted to project tier at {}.", target_loc.file.display())),
            metadata: MemoryMetadata { kind: MemoryType::Reference },
        },
        body: backref_body,
    };
    write_memory_unchecked(&source_loc, &backref)?;

    Ok(PromotionOutcome {
        source: source_loc.clone(),
        target: target_loc,
        backref: Some(source_loc),
    })
}

fn promote_project_to_user(
    ctx: &MemoryContext,
    name: &str,
) -> Result<PromotionOutcome, StoreError> {
    let prj = ctx.active_project.ok_or(StoreError::NoActiveProject)?;
    let source_loc = project_file(&ctx.pm_root, prj, name);
    let target_loc = user_file(&ctx.home, &ctx.cwd, name);

    if !source_loc.file.is_file() {
        return Err(StoreError::NotFound(format!(
            "project-tier memory {name:?} not found at {}",
            source_loc.file.display()
        )));
    }

    // Read the project content, write it to the user tier (overwriting any
    // back-reference left behind by a prior user->project promotion), then
    // remove the project-tier copy.
    let original = MemoryFile::read(&source_loc.file).map_err(StoreError::File)?;
    write_memory_unchecked(&target_loc, &original)?;
    fs::remove_file(&source_loc.file).map_err(StoreError::Io)?;

    Ok(PromotionOutcome {
        source: source_loc,
        target: target_loc,
        backref: None,
    })
}

fn list_files_in(dir: &Path, scope: Scope) -> Result<Vec<MemoryHit>, StoreError> {
    let mut out: Vec<MemoryHit> = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(dir).map_err(StoreError::Io)? {
        let entry = entry.map_err(StoreError::Io)?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            entries.push(path);
        }
    }
    entries.sort();
    for path in entries {
        let location = MemoryLocation { scope, directory: dir.to_path_buf(), file: path };
        out.push(read_hit(location)?);
    }
    Ok(out)
}

fn read_hit(location: MemoryLocation) -> Result<MemoryHit, StoreError> {
    let file = MemoryFile::read(&location.file).map_err(StoreError::File)?;
    Ok(MemoryHit { location, file })
}

/// Errors emitted by the memory store.
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    File(MemoryFileError),
    NoActiveProject,
    NoActiveTicket,
    NotFound(String),
    TargetExists(PathBuf),
    UnsupportedPromotion(String),
    /// A caller asked PM to write the user tier directly. The user tier
    /// stays under Claude Code's auto-memory control; PM only touches it
    /// through [`promote_memory`].
    UserTierWriteForbidden,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "memory store io: {e}"),
            StoreError::File(e) => write!(f, "memory store file: {e}"),
            StoreError::NoActiveProject => {
                write!(f, "no active project; pass --project <PRJ> or run inside one")
            }
            StoreError::NoActiveTicket => write!(f, "no active ticket; pass --ticket <ID>"),
            StoreError::NotFound(s) => write!(f, "{s}"),
            StoreError::TargetExists(p) => write!(f, "target already exists: {}", p.display()),
            StoreError::UnsupportedPromotion(s) => write!(f, "{s}"),
            StoreError::UserTierWriteForbidden => write!(
                f,
                "PM does not write user-tier memories directly; use `pm memory promote --to user` \
                 or author the file under `~/.claude/projects/.../memory/` via Claude Code"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;
    use std::path::PathBuf;

    fn tmp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-memory-store-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx(home: PathBuf, cwd: PathBuf, pm_root: PathBuf, prj: Option<LeafId>) -> MemoryContext {
        MemoryContext {
            home,
            cwd,
            pm_root,
            active_project: prj,
            active_ticket_dir: None,
        }
    }

    #[test]
    fn writing_user_tier_directly_is_forbidden() {
        let root = tmp_dir("user-write");
        let ctx = ctx(root.join("home"), root.join("work"), root.join(".pm"), None);
        let err = write_memory(
            &ctx, Scope::User, "feedback-testing",
            MemoryType::Feedback,
            Some("Real DB in tests".into()),
            "Integration tests must hit a real DB.\n",
        ).unwrap_err();
        assert!(matches!(err, StoreError::UserTierWriteForbidden));
        // Nothing was written.
        let dir = ctx.user_directory();
        assert!(!dir.exists() || std::fs::read_dir(&dir).map(|d| d.count() == 0).unwrap_or(true));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_then_list_at_project_tier() {
        let root = tmp_dir("proj-write");
        let home = root.join("home");
        let cwd = root.join("work");
        let pm_root = root.join(".pm");
        let prj = LeafId::new(TypePrefix::Project, 1);
        let ctx = ctx(home, cwd, pm_root, Some(prj));

        let loc = write_memory(
            &ctx, Scope::Project, "auth-stack",
            MemoryType::Project,
            Some("Auth conventions".into()),
            "Bearer JWTs.\n",
        ).unwrap();
        assert!(loc.file.is_file());

        let hits = list_at_scope(&ctx, Scope::Project).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file.front_matter.name, "auth-stack");
        assert_eq!(hits[0].location.scope, Scope::Project);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn project_tier_requires_active_project() {
        let root = tmp_dir("no-prj");
        let ctx = ctx(root.join("home"), root.join("work"), root.join(".pm"), None);
        let err = write_memory(&ctx, Scope::Project, "x", MemoryType::Project, None, "")
            .unwrap_err();
        assert!(matches!(err, StoreError::NoActiveProject));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lookup_resolves_project_before_user_for_same_name() {
        // To get content into the user tier without going through write_memory
        // (which is forbidden), drop a file straight into the user-tier
        // directory the way Claude Code's auto-memory would.
        let root = tmp_dir("collision");
        let home = root.join("home");
        let cwd = root.join("work");
        let pm_root = root.join(".pm");
        let prj = LeafId::new(TypePrefix::Project, 1);
        let ctx = ctx(home.clone(), cwd.clone(), pm_root.clone(), Some(prj));

        let user_loc = ctx.user_path("shared");
        fs::create_dir_all(&user_loc.directory).unwrap();
        MemoryFile::new("shared", MemoryType::User, "USER content\n")
            .write(&user_loc.file).unwrap();
        write_memory(&ctx, Scope::Project, "shared", MemoryType::Project, None,
            "PROJECT content\n").unwrap();

        let hit = lookup_by_name(&ctx, "shared").unwrap().expect("hit");
        assert_eq!(hit.location.scope, Scope::Project,
            "project tier must win on collision");
        assert!(hit.file.body.contains("PROJECT content"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lookup_falls_back_to_user_when_only_tier_present() {
        let root = tmp_dir("user-only");
        let ctx = ctx(
            root.join("home"),
            root.join("work"),
            root.join(".pm"),
            Some(LeafId::new(TypePrefix::Project, 1)),
        );
        let user_loc = ctx.user_path("solo");
        fs::create_dir_all(&user_loc.directory).unwrap();
        MemoryFile::new("solo", MemoryType::User, "user body\n")
            .write(&user_loc.file).unwrap();
        let hit = lookup_by_name(&ctx, "solo").unwrap().expect("hit");
        assert_eq!(hit.location.scope, Scope::User);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn lookup_returns_none_when_missing() {
        let root = tmp_dir("missing");
        let ctx = ctx(root.join("home"), root.join("work"), root.join(".pm"), None);
        assert!(lookup_by_name(&ctx, "nope").unwrap().is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn promote_user_to_project_moves_and_leaves_backref() {
        let root = tmp_dir("promote-up");
        let home = root.join("home");
        let cwd = root.join("work");
        let pm_root = root.join(".pm");
        let prj = LeafId::new(TypePrefix::Project, 1);
        let ctx = ctx(home.clone(), cwd.clone(), pm_root.clone(), Some(prj));

        let user_loc = ctx.user_path("auth-stack");
        fs::create_dir_all(&user_loc.directory).unwrap();
        let mut seed = MemoryFile::new("auth-stack", MemoryType::User, "Use bearer JWTs.\n");
        seed.front_matter.description = Some("Auth conventions".into());
        seed.write(&user_loc.file).unwrap();

        let outcome = promote_memory(&ctx, "auth-stack", Scope::Project).unwrap();
        // Canonical content lives at project tier.
        assert!(outcome.target.file.is_file());
        let project_mf = MemoryFile::read(&outcome.target.file).unwrap();
        assert!(project_mf.body.contains("Use bearer JWTs"));
        // Back-reference at the user tier.
        let backref = outcome.backref.expect("user-to-project leaves a back-reference");
        assert!(backref.file.is_file());
        let backref_mf = MemoryFile::read(&backref.file).unwrap();
        assert_eq!(backref_mf.front_matter.metadata.kind, MemoryType::Reference);
        assert!(backref_mf.body.contains("Canonical:"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn promote_project_to_user_clears_project_tier() {
        let root = tmp_dir("promote-down");
        let home = root.join("home");
        let cwd = root.join("work");
        let pm_root = root.join(".pm");
        let prj = LeafId::new(TypePrefix::Project, 1);
        let ctx = ctx(home.clone(), cwd.clone(), pm_root.clone(), Some(prj));

        write_memory(&ctx, Scope::Project, "auth-stack", MemoryType::Project,
            None, "Project body\n").unwrap();

        let outcome = promote_memory(&ctx, "auth-stack", Scope::User).unwrap();
        assert!(outcome.target.file.is_file(), "user-tier canonical exists after demotion");
        assert!(!outcome.source.file.exists(), "project-tier file removed");
        assert!(outcome.backref.is_none(), "demotion does not write a back-reference");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn promote_user_to_project_errors_when_source_missing() {
        let root = tmp_dir("missing-source");
        let ctx = ctx(
            root.join("home"),
            root.join("work"),
            root.join(".pm"),
            Some(LeafId::new(TypePrefix::Project, 1)),
        );
        let err = promote_memory(&ctx, "nope", Scope::Project).unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn promote_to_ticket_is_unsupported_for_phase_10() {
        let root = tmp_dir("ticket-tier");
        let ctx = ctx(root.join("home"), root.join("work"), root.join(".pm"), None);
        let err = promote_memory(&ctx, "x", Scope::Ticket).unwrap_err();
        assert!(matches!(err, StoreError::UnsupportedPromotion(_)));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_all_combines_resolvable_tiers() {
        let root = tmp_dir("list-all");
        let home = root.join("home");
        let cwd = root.join("work");
        let pm_root = root.join(".pm");
        let prj = LeafId::new(TypePrefix::Project, 1);
        let ticket_path = pm_root.join("projects/PRJ1/products/PRD1/epics/EPC1/tasks/TSK1");
        fs::create_dir_all(&ticket_path).unwrap();

        let mut ctx = ctx(home, cwd, pm_root, Some(prj));
        ctx.active_ticket_dir = Some(ticket_path);

        // The user tier is seeded directly because PM does not write it.
        let user_loc = ctx.user_path("u");
        fs::create_dir_all(&user_loc.directory).unwrap();
        MemoryFile::new("u", MemoryType::User, "")
            .write(&user_loc.file).unwrap();
        write_memory(&ctx, Scope::Project, "p", MemoryType::Project, None, "").unwrap();
        write_memory(&ctx, Scope::Ticket, "t", MemoryType::Reference, None, "").unwrap();

        let hits = list_all(&ctx).unwrap();
        assert_eq!(hits.len(), 3);
        let scopes: Vec<Scope> = hits.iter().map(|h| h.location.scope).collect();
        assert!(scopes.contains(&Scope::User));
        assert!(scopes.contains(&Scope::Project));
        assert!(scopes.contains(&Scope::Ticket));

        fs::remove_dir_all(&root).ok();
    }
}
