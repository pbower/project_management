//! Task data structure and related functionality.
//!
//! Defines the core `Task` struct that represents a single work item with all
//! its associated metadata, including hierarchy, timing, and process
//! information. Identifiers are typed `LeafId`s carrying the ticket type via
//! their prefix (`PRJ1`, `PRD3`, `EPC7`, `TSK22`, `SBT2`, `MLS1`).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::fields::*;
use crate::store::id::LeafId;

/// A work item with metadata for project management.
///
/// Tasks support hierarchical organisation (Project > Product > Epic > Task >
/// Subtask) plus cross-cutting milestones. Project membership is derived from
/// the parent chain on disk; there is no separate label field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: LeafId,
    pub title: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub user_story: Option<String>,
    #[serde(default)]
    pub requirements: Option<String>,
    pub tags: Vec<String>,
    pub due: Option<NaiveDate>,
    pub parent: Option<LeafId>,
    pub kind: Kind,
    pub status: Status,
    pub priority_level: Option<Priority>,
    pub urgency: Option<Urgency>,
    pub process_stage: Option<ProcessStage>,
    pub issue_link: Option<String>,
    pub pr_link: Option<String>,
    #[serde(default, alias = "design_files")]
    pub artifacts: Vec<String>,
    pub created_at_utc: i64,
    pub updated_at_utc: i64,
}

/// A template for creating tasks with predefined values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplate {
    pub name: String,
    pub title_template: Option<String>,
    pub description_template: Option<String>,
    pub tags: Vec<String>,
    pub kind: Kind,
    pub priority_level: Option<Priority>,
    pub urgency: Option<Urgency>,
    pub process_stage: Option<ProcessStage>,
    pub status: Status,
}
