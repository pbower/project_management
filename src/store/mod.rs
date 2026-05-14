//! v2 store module: typed IDs, disk layout, state.json, and aliases.
//!
//! Builds the foundation for the v2 data model where each ticket is a directory
//! containing a `CLAUDE.md` plus an `artifacts/` folder, and `state.json` indexes
//! leaf IDs to on-disk paths.
//!
//! This module is additive to the existing `task.rs` / `db.rs` data model and
//! will replace it over subsequent phases (see PM_BUILD_PLAN.md).

pub mod id;
pub mod state;
pub mod aliases;
pub mod layout;
pub mod resolver;
pub mod migrate;
pub mod front_matter;
pub mod sections;
pub mod templates;
pub mod claude_md;
pub mod artifacts;
pub mod watcher;
pub mod task_bridge;
pub mod git;
pub mod events;
pub mod locks;

pub use id::{TypePrefix, LeafId, AddressId, IdInput, IdParseError};
pub use state::{State, ItemEntry, StateError};
pub use aliases::Aliases;
pub use layout::{Layout, LayoutError, TYPE_FOLDER_ROOTS};
pub use resolver::{Resolver, Resolved, ResolveError};
pub use migrate::{MigrationPlan, MigrationStep, MigrateError};
pub use front_matter::{FrontMatter, Document, MemoryRef, FrontMatterError, split_front_matter};
pub use sections::{Section, ParsedBody};
pub use templates::{ResolvedTemplate, TemplateSource};
pub use claude_md::{Ticket, TicketError, CLAUDE_MD, ARTIFACTS_IMPORT};
pub use artifacts::{
    ArtifactsIndex, ArtifactEntry, SweepReport, ArtifactError, ARTIFACTS_MD,
    sweep_dir, rename_artifact,
};
pub use watcher::{ArtifactsWatcher, DEFAULT_DEBOUNCE};
pub use task_bridge::{
    task_to_document, task_from_document, project_ancestor,
    SECTION_DESCRIPTION, SECTION_REQUIREMENTS, SECTION_SUMMARY, SECTION_USER_STORY,
};
pub use git::{commit_workspace, ensure_repo, subject as commit_subject, GitError, GitResult};
pub use events::{Event, EventError, EventResult, actor, emit_event, read_events};
pub use locks::{
    LockFile, LockMode, AcquireOutcome, LockError, LockResult, DEFAULT_TTL_SECONDS,
    acquire, release, list as list_locks, reap_stale, refresh_heartbeat, read as read_lock,
};

