//! Enumerations and field types for task management.
//!
//! This module defines all the structured data types used to categorise and organise tasks,
//! including task kinds, priorities, urgency levels, process stages, and status values.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Hierarchical task types that define the organisational structure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    #[serde(alias = "Product")]
    Product,
    #[serde(alias = "Epic")]
    Epic,
    #[serde(alias = "Task")]
    Task,
    #[serde(alias = "Subtask")]
    Subtask,
    #[serde(alias = "Milestone")]
    Milestone,
}

/// Priority classification for task importance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    MustHave,
    NiceToHave,
    CutFirst,
}

/// Urgency matrix classification based on importance and time sensitivity.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Urgency {
    UrgentImportant,
    UrgentNotImportant,
    NotUrgentImportant,
    NotUrgentNotImportant,
}

/// Development process stages for tracking workflow progress.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessStage {
    Ideation,
    Design,
    Prototyping,
    Implementation,
    Testing,
    Refinement,
    Release,
}

/// Task completion status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[serde(alias = "Open")]
    Open,
    #[serde(alias = "InProgress")]
    InProgress,
    #[serde(alias = "Done")]
    Done,
}

/// Available sorting options for task lists.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SortKey {
    Due,
    Priority,
    Id,
}

/// Filtering options for tasks based on due dates.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DueFilter {
    Today,
    ThisWeek,
    Overdue,
    None,
}
