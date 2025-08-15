//! Task data structure and related functionality.
//!
//! This module defines the core `Task` struct that represents a single work item
//! with all its associated metadata, including hierarchy, timing, and process information.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::fields::*;

/// A work item with comprehensive metadata for project management.
///
/// Tasks support hierarchical organisation (Product > Epic > Task > Subtask),
/// time tracking, process stages, and various categorisation fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub user_story: Option<String>,
    #[serde(default)]
    pub requirements: Option<String>,
    pub tags: Vec<String>,
    pub project: Option<String>,
    pub due: Option<NaiveDate>,
    pub parent: Option<u64>,
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
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub kind: Kind,
    pub priority_level: Option<Priority>,
    pub urgency: Option<Urgency>,
    pub process_stage: Option<ProcessStage>,
    pub status: Status,
}
