//! v2 store module: typed IDs, disk layout, state.json, and aliases.
//!
//! Builds the foundation for the v2 data model where each ticket is a directory
//! containing a `CLAUDE.md` plus an `artifacts/` folder, and `state.json` indexes
//! leaf IDs to on-disk paths.
//!
//! This module is additive to the existing `task.rs` / `db.rs` data model and
//! will replace it over subsequent phases (see PM_BUILD_PLAN.md).

pub mod aliases;
pub mod artifacts;
pub mod claude_md;
pub mod events;
pub mod front_matter;
pub mod git;
pub mod id;
pub mod layout;
pub mod locks;
pub mod migrate;
pub mod resolver;
pub mod sections;
pub mod state;
pub mod task_bridge;
pub mod templates;
pub mod watcher;

pub use aliases::Aliases;
pub use artifacts::{
    rename_artifact, sweep_dir, ArtifactEntry, ArtifactError, ArtifactsIndex, SweepReport,
    ARTIFACTS_MD,
};
pub use claude_md::{Ticket, TicketError, ARTIFACTS_IMPORT, CLAUDE_MD};
pub use events::{actor, emit_event, read_events, Event, EventError, EventResult};
pub use front_matter::{split_front_matter, Document, FrontMatter, FrontMatterError, MemoryRef};
pub use git::{
    commit_workspace, ensure_repo, head_commit, squash_since, subject as commit_subject, GitError,
    GitResult,
};
pub use id::{AddressId, IdInput, IdParseError, LeafId, TypePrefix};
pub use layout::{Layout, LayoutError, TYPE_FOLDER_ROOTS};
pub use locks::{
    acquire, list as list_locks, read as read_lock, reap_stale, refresh_heartbeat, release,
    AcquireOutcome, LockError, LockFile, LockMode, LockResult, DEFAULT_TTL_SECONDS,
};
pub use migrate::{MigrateError, MigrationPlan, MigrationStep};
pub use resolver::{ResolveError, Resolved, Resolver};
pub use sections::{ParsedBody, Section};
pub use state::{ItemEntry, State, StateError};
pub use task_bridge::{
    project_ancestor, task_from_document, task_to_document, SECTION_DESCRIPTION,
    SECTION_REQUIREMENTS, SECTION_SUMMARY, SECTION_USER_STORY,
};
pub use templates::{ResolvedTemplate, TemplateSource};
pub use watcher::{ArtifactsWatcher, DEFAULT_DEBOUNCE};
