//! Clap derive surface for the v2 command tree.
//!
//! Mirrors PM_DESIGN.md Section 8.1. Every verb on the design surface has a
//! variant here, including the workflow / memory / view verbs that are stubs
//! until later phases wire them up (Phase 5 for git, Phase 6 for locks,
//! Phase 9 for `tv`, Phase 10 for memory).

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use crate::store::TypePrefix;

/// Top-level v2 subcommand tree. Embedded in [`crate::cmd::Commands`] as the
/// `V2` variant; reachable as `pm v2 <verb> ...`.
#[derive(Subcommand, Debug)]
pub enum V2Commands {
    /// Initialise a fresh `.pm/` in the current directory.
    Init,

    /// Create a new ticket.
    Add {
        /// Ticket title (free text; quote if it contains spaces).
        title: String,
        /// Ticket kind.
        #[arg(long, value_enum)]
        kind: KindArg,
        /// Parent address or leaf id (omit for an orphan ticket).
        #[arg(long)]
        parent: Option<String>,
        /// Override the auto-generated kebab-case slug.
        #[arg(long)]
        slug: Option<String>,
    },

    /// List tickets recorded in `state.json`.
    List {
        /// Filter by kind.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Render in tree form (defaults to flat).
        #[arg(long)]
        tree: bool,
    },

    /// Show a single ticket's metadata and rendered body.
    Show {
        /// Leaf id, address, or slugged form.
        id: String,
    },

    /// Move a ticket under a new parent.
    Move {
        /// Ticket to move (any id form).
        id: String,
        /// New parent address, or `:orphan` to detach.
        dest: String,
    },

    /// Mark a ticket complete (status: done).
    Complete { id: String },

    /// Tombstone a ticket and remove its on-disk directory.
    Delete {
        id: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        force: bool,
    },

    /// Open the ticket's CLAUDE.md in `$EDITOR`.
    Edit {
        id: String,
        /// Position the editor cursor at a specific section heading.
        #[arg(long)]
        section: Option<String>,
    },

    /// Print the composed context view (ancestors plus this ticket).
    Context {
        id: String,
        /// Skip the linked-memories section.
        #[arg(long)]
        no_memories: bool,
    },

    /// Write the composed view to a sidecar file on disk.
    Materialise {
        id: String,
        /// Output path (defaults to `<ticket-dir>/<leaf>.composed.md`).
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Manage a ticket's `artifacts/` directory.
    Artifact {
        #[command(subcommand)]
        action: ArtifactAction,
    },

    /// Manage per-kind section templates.
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },

    /// Update a ticket's status (e.g. `open`, `in-progress`, `done`).
    Status { id: String, value: String },

    /// Update priority (`must-have`, `nice-to-have`, `cut-first`).
    Priority { id: String, value: String },

    /// Set or clear the due date (`YYYY-MM-DD`, or `none` to clear).
    Due { id: String, value: String },

    /// Manage dependency edges.
    Dep {
        id: String,
        /// `needs` (add a dep) or `drop` (remove one).
        op: String,
        /// Other ticket id.
        other: String,
    },

    /// Add or remove tags. Each `op` is `+name` or `-name`.
    Tag { id: String, ops: Vec<String> },

    /// Set a free-form link (e.g. `github_issue owner/repo#42`).
    Link { id: String, key: String, value: String },

    /// Set the milestone leaf id (or `none` to clear).
    Milestone { id: String, value: String },

    /// Memory verbs (Phase 10 stub).
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Acquire a lock on a ticket (Phase 6 stub).
    Checkout {
        id: String,
        #[arg(long)]
        intent: Option<String>,
    },

    /// Release a lock on a ticket (Phase 6 stub).
    Checkin {
        id: String,
        #[arg(long)]
        summary: Option<String>,
    },

    /// Return the next dependency-ready ticket (Phase 6 stub).
    Next {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        filter: Option<String>,
    },

    /// Show active checkouts (Phase 6 stub).
    Locks,

    /// Full-screen activity feed (Phase 9 stub).
    Tv,

    /// Git log filtered to a ticket's subtree (Phase 5 stub).
    Log { id: String },

    /// Substring search across CLAUDE.md bodies.
    Search { query: String },

    /// Reconcile `state.json` against the on-disk truth.
    Doctor,
}

/// Subcommands for `pm v2 artifact ...`.
#[derive(Subcommand, Debug)]
pub enum ArtifactAction {
    /// Copy a file into the ticket's artifacts/ folder.
    Add {
        id: String,
        path: PathBuf,
        /// Optional description (defaults to empty).
        #[arg(long)]
        desc: Option<String>,
    },
    /// Rename an artifact, preserving its description.
    Rename {
        id: String,
        old: String,
        new: String,
    },
    /// List artifacts for a ticket.
    List { id: String },
}

/// Subcommands for `pm v2 template ...`.
#[derive(Subcommand, Debug)]
pub enum TemplateAction {
    /// Open the per-kind template in `$EDITOR`, copying from the built-in
    /// if no override exists yet.
    Edit { kind: KindArg },
    /// Re-apply the kind template to an existing ticket. Preserves matching
    /// section content; user-added sections are kept at the tail.
    Apply { id: String },
}

/// Subcommands for `pm v2 memory ...`. All stubs until Phase 10.
#[derive(Subcommand, Debug)]
pub enum MemoryAction {
    /// Link a memory to a ticket.
    Link {
        id: String,
        name: String,
        #[arg(long, value_enum, default_value_t = MemoryScopeArg::User)]
        scope: MemoryScopeArg,
    },
    /// Unlink a memory from a ticket.
    Unlink { id: String, name: String },
    /// List linked memories for a ticket.
    List { id: String },
    /// Promote a memory between tiers.
    Promote {
        name: String,
        #[arg(long, value_enum)]
        to: MemoryScopeArg,
    },
    /// Write a new memory at the given scope.
    Write {
        name: String,
        body: String,
        #[arg(long, value_enum, default_value_t = MemoryScopeArg::User)]
        scope: MemoryScopeArg,
    },
}

/// Ticket kind as exposed on the CLI.
#[derive(ValueEnum, Debug, Clone, Copy)]
#[value(rename_all = "kebab-case")]
pub enum KindArg {
    Project,
    Product,
    Epic,
    Task,
    Subtask,
    Milestone,
}

impl From<KindArg> for TypePrefix {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Project => TypePrefix::Project,
            KindArg::Product => TypePrefix::Product,
            KindArg::Epic => TypePrefix::Epic,
            KindArg::Task => TypePrefix::Task,
            KindArg::Subtask => TypePrefix::Subtask,
            KindArg::Milestone => TypePrefix::Milestone,
        }
    }
}

impl KindArg {
    pub fn from_prefix(p: TypePrefix) -> Self {
        match p {
            TypePrefix::Project => KindArg::Project,
            TypePrefix::Product => KindArg::Product,
            TypePrefix::Epic => KindArg::Epic,
            TypePrefix::Task => KindArg::Task,
            TypePrefix::Subtask => KindArg::Subtask,
            TypePrefix::Milestone => KindArg::Milestone,
        }
    }
}

/// Memory tier as exposed on the CLI.
#[derive(ValueEnum, Debug, Clone, Copy)]
#[value(rename_all = "kebab-case")]
pub enum MemoryScopeArg {
    User,
    Project,
    Ticket,
}
