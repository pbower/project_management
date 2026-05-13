//! Helper enums for the top-level [`crate::cmd::Commands`] tree.
//!
//! Verb variants themselves live in [`crate::cmd`]; this module owns the
//! smaller enums that several variants share - artifact / template / memory
//! sub-actions plus the ticket-kind and memory-tier value enums.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use crate::store::TypePrefix;

/// Subcommands for `pm artifact ...`.
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

/// Subcommands for `pm template ...`.
#[derive(Subcommand, Debug)]
pub enum TemplateAction {
    /// Open the per-kind template in `$EDITOR`, copying from the built-in
    /// if no override exists yet.
    Edit { kind: KindArg },
    /// Re-apply the kind template to an existing ticket. Preserves matching
    /// section content; user-added sections are kept at the tail.
    Apply { id: String },
}

/// Subcommands for `pm memory ...`. All stubs until Phase 10.
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
