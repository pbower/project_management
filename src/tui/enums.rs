//! Lightweight enums used by the workflow board.
//!
//! The v0.9 TUI had a much larger state machine here; v0.3.0 demolition
//! kept only what the workflow board actually needs. New TUI surfaces
//! (LHP, Workbench, Activity) use [`crate::tui::nav`] instead.

use crate::store::LeafId;

/// Hierarchy levels for the workflow board's drill-down filter.
/// Includes `Milestone` because the board may filter to it; the strict
/// linear hierarchy in [`crate::tui::nav::Level`] excludes it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HierarchyLevel {
    Project,
    Product,
    Epic,
    Task,
    Subtask,
    Milestone,
}

/// Context for the workflow board's drill-down filter.
#[derive(Clone, PartialEq, Debug)]
pub struct NavigationContext {
    pub level: HierarchyLevel,
    pub parent_id: Option<LeafId>,
    pub parent_title: Option<String>,
}

impl NavigationContext {
    /// Create a context for viewing all items at a specific hierarchy level.
    pub fn new_all_level(level: HierarchyLevel) -> Self {
        NavigationContext {
            level,
            parent_id: None,
            parent_title: None,
        }
    }

    /// Create a context for viewing items filtered by a specific parent.
    pub fn new_filtered(level: HierarchyLevel, parent_id: LeafId, parent_title: String) -> Self {
        NavigationContext {
            level,
            parent_id: Some(parent_id),
            parent_title: Some(parent_title),
        }
    }

    /// Human-readable display name.
    pub fn get_display_name(&self) -> String {
        match (&self.parent_id, &self.parent_title) {
            (Some(id), Some(title)) => {
                let parent_type = match self.level {
                    HierarchyLevel::Product => "Project",
                    HierarchyLevel::Epic => "Product",
                    HierarchyLevel::Task => "Epic",
                    HierarchyLevel::Subtask => "Task",
                    HierarchyLevel::Milestone => "Parent",
                    HierarchyLevel::Project => "Parent",
                };
                format!("All {:?}s for {} {} {}", self.level, parent_type, id, title)
            }
            _ => format!("All {:?}s", self.level),
        }
    }
}
