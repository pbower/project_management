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

pub use id::{TypePrefix, LeafId, AddressId, IdInput, IdParseError};
pub use state::{State, ItemEntry, StateError};
pub use aliases::Aliases;
pub use layout::{Layout, LayoutError, TYPE_FOLDER_ROOTS};
