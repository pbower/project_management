//! Enumerations for TUI state management.

use crate::store::LeafId;

/// Top-level TUI mode. Mode 1 (Tickets) hosts the existing per-screen
/// [`AppState`] flow; Modes 2 and 3 are their own surfaces, stubbed until
/// Phases 8 and 9 land.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Mode 1 - the hierarchical Ticket View.
    Tickets,
    /// Mode 2 - the Document Workspace (Phase 8).
    Documents,
    /// Mode 3 - the Activity View (Phase 9).
    Activity,
}

impl Mode {
    /// The next mode in the `Tab` cycle.
    pub fn next(self) -> Mode {
        match self {
            Mode::Tickets => Mode::Documents,
            Mode::Documents => Mode::Activity,
            Mode::Activity => Mode::Tickets,
        }
    }

    /// The previous mode in the `Shift+Tab` cycle.
    pub fn prev(self) -> Mode {
        match self {
            Mode::Tickets => Mode::Activity,
            Mode::Documents => Mode::Tickets,
            Mode::Activity => Mode::Documents,
        }
    }

    /// Short label shown in the header.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Tickets => "Mode 1 Tickets",
            Mode::Documents => "Mode 2 Documents",
            Mode::Activity => "Mode 3 Activity",
        }
    }
}

/// Application state for the terminal user interface.
#[derive(Clone, Copy, PartialEq)]
pub enum AppState {
    TaskList,
    TaskDetail,
    AddTask,
    EditTask,
    UserStoryDialog,
    RequirementsDialog,
    Help,
    Confirm,
}

/// Input mode for text entry fields.
#[derive(Clone)]
pub enum InputMode {
    None,
    Text,
}

/// Hierarchy levels for navigation context.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HierarchyLevel {
    Project,
    Product,
    Epic,
    Task,
    Subtask,
    Milestone,
}

/// Context for hierarchical navigation in the TUI.
#[derive(Clone, PartialEq, Debug)]
pub struct NavigationContext {
    pub level: HierarchyLevel,
    pub parent_id: Option<LeafId>,
    pub parent_title: Option<String>,
}

impl NavigationContext {
    /// Create a context for viewing all products.
    pub fn new_all_products() -> Self {
        NavigationContext {
            level: HierarchyLevel::Product,
            parent_id: None,
            parent_title: None,
        }
    }

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

    /// Get a human-readable display name for this navigation context.
    pub fn get_display_name(&self) -> String {
        match (&self.parent_id, &self.parent_title) {
            (Some(id), Some(title)) => {
                let parent_type = match self.level {
                    HierarchyLevel::Product => "Project",
                    HierarchyLevel::Epic => "Product",
                    HierarchyLevel::Task => "Epic",
                    HierarchyLevel::Subtask => "Task",
                    HierarchyLevel::Milestone => "Parent", // Special case
                    HierarchyLevel::Project => "Parent",   // Top of the hierarchy
                };
                format!("All {}s for {} {} {}",
                    format!("{:?}", self.level),
                    parent_type,
                    id,
                    title)
            },
            _ => format!("All {}s", format!("{:?}", self.level)),
        }
    }
}
