//! Enumerations for TUI state management.

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
    pub parent_id: Option<u64>,
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
    pub fn new_filtered(level: HierarchyLevel, parent_id: u64, parent_title: String) -> Self {
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
                    HierarchyLevel::Epic => "Product",
                    HierarchyLevel::Task => "Epic",
                    HierarchyLevel::Subtask => "Task",
                    HierarchyLevel::Milestone => "Parent", // Special case
                    HierarchyLevel::Product => "Parent", // Shouldn't happen
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