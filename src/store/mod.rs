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

