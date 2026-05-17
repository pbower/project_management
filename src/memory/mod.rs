//! Three-tier memory store for PM tickets.
//!
//! Memory is the durable, cross-ticket knowledge layer that sits next to the
//! CLAUDE.md context model. Three tiers, each with a different owner:
//!
//! - **User tier** (`~/.claude/projects/<encoded-cwd>/memory/`) is Claude
//!   Code's own auto-memory store. PM treats it as read-mostly; the only
//!   write path here is the back-reference left behind when a user memory is
//!   promoted to project scope.
//! - **Project tier** (`<.pm>/projects/<PRJ>/memories/`) is the team-shared
//!   layer. PM owns it. Files are committed via the regular git path.
//! - **Ticket tier** (`<.pm>/.../<ticket>/memories/`) lives alongside the
//!   ticket's `CLAUDE.md`.
//!
//! All three share one on-disk format (front-matter + body) compatible with
//! Claude Code's auto-memory schema.

pub mod file;
pub mod scope;
pub mod store;

pub use file::{MemoryFile, MemoryFileError, MemoryFrontMatter};
pub use scope::{MemoryLocation, MemoryType, Scope};
pub use store::{
    list_all, list_at_scope, lookup_by_name, promote_memory, write_memory, MemoryHit, StoreError,
};
