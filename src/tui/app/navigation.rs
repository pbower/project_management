//! Navigation between hierarchy levels and across the history of past
//! contexts. Push/pop of the navigation stack, traversal of parent-child
//! relationships, and the hierarchy-level theme colour live here so the
//! ticket-list and ticket-detail surfaces stay focused on their own input
//! and rendering concerns.

use ratatui::style::Color;

use crate::fields::Kind;
use crate::store::LeafId;
use crate::tui::colors::{DARK_GREEN, DARK_PURPLE, DARK_RED, GOLD};
use crate::tui::enums::{AppState, HierarchyLevel, NavigationContext};

use super::{App, NavigationSnapshot};

impl App {
    /// Push current state to navigation history and transition to new state.
    pub(super) fn push_state(
        &mut self,
        new_state: AppState,
        new_context: Option<NavigationContext>,
    ) {
        // Create snapshot of current state
        let snapshot = NavigationSnapshot {
            state: self.state,
            context: self.navigation_context.clone(),
            selected_task: self.selected_task,
        };

        // Add to history
        self.navigation_history.push(snapshot);

        // Limit history size
        if self.navigation_history.len() > self.max_history {
            self.navigation_history.remove(0);
        }

        // Transition to new state
        self.state = new_state;
        if let Some(context) = new_context {
            self.navigation_context = context;
        }

        // Clear status message
        self.status_message.clear();
    }

    /// Go back to the previous state if history exists.
    pub(super) fn go_back(&mut self) -> bool {
        if let Some(snapshot) = self.navigation_history.pop() {
            self.state = snapshot.state;
            self.navigation_context = snapshot.context;
            self.selected_task = snapshot.selected_task;

            // Update filtered tasks and UI state for the restored context
            self.update_filtered_tasks();

            // Clear any status messages
            self.status_message.clear();

            true
        } else {
            false
        }
    }

    /// Check if navigation history exists.
    pub(super) fn has_navigation_history(&self) -> bool {
        !self.navigation_history.is_empty()
    }

    /// Get the theme color for the current hierarchy level.
    pub(super) fn get_hierarchy_color(&self) -> Color {
        match self.navigation_context.level {
            HierarchyLevel::Project => Color::Cyan,      // Cyan for the top-level Project tickets
            HierarchyLevel::Product => Color::Blue,      // Dark Blue (keeping existing)
            HierarchyLevel::Epic => DARK_GREEN,          // Forest Green
            HierarchyLevel::Task => GOLD,                // Gold
            HierarchyLevel::Subtask => DARK_RED,         // Crimson Red
            HierarchyLevel::Milestone => DARK_PURPLE,    // Magenta for milestones
        }
    }

    /// Navigate between hierarchy levels without parent filtering.
    ///
    /// Shows all items of the target hierarchy level (Product, Epic, Task, etc.)
    /// rather than drilling down into a specific parent's children.
    pub(super) fn navigate_hierarchy_unfiltered(&mut self, forward: bool) {
        let new_level = if forward {
            match self.navigation_context.level {
                HierarchyLevel::Project => HierarchyLevel::Product,
                HierarchyLevel::Product => HierarchyLevel::Epic,
                HierarchyLevel::Epic => HierarchyLevel::Task,
                HierarchyLevel::Task => HierarchyLevel::Subtask,
                HierarchyLevel::Subtask => HierarchyLevel::Milestone,
                HierarchyLevel::Milestone => return, // Can't go further
            }
        } else {
            match self.navigation_context.level {
                HierarchyLevel::Project => return, // Can't go back beyond Project
                HierarchyLevel::Product => HierarchyLevel::Project,
                HierarchyLevel::Epic => HierarchyLevel::Product,
                HierarchyLevel::Task => HierarchyLevel::Epic,
                HierarchyLevel::Subtask => HierarchyLevel::Task,
                HierarchyLevel::Milestone => HierarchyLevel::Subtask,
            }
        };

        self.navigation_context = NavigationContext::new_all_level(new_level);
        self.update_filtered_tasks();
        self.set_status_message(format!(
            "Navigated to {}",
            self.navigation_context.get_display_name()
        ));
    }

    /// Navigate contextually through the hierarchy by drilling down or going back.
    ///
    /// Forward navigation drills down into the selected item's children.
    /// Backward navigation returns to the previous context using the navigation stack.
    pub(super) fn navigate_hierarchy_contextual(&mut self, forward: bool) {
        if forward {
            // Drill down into selected item
            if let Some(selected) = self.task_list_state.selected() {
                if let Some(&task_id) = self.filtered_tasks.get(selected) {
                    if let Some(task) = self.db.get(task_id) {
                        let child_level = match task.kind {
                            Kind::Project => HierarchyLevel::Product,
                            Kind::Product => HierarchyLevel::Epic,
                            Kind::Epic => HierarchyLevel::Task,
                            Kind::Task => HierarchyLevel::Subtask,
                            Kind::Subtask => {
                                self.set_status_message("No child level for Subtask".to_string());
                                return;
                            }
                            Kind::Milestone => {
                                self.set_status_message("No child level for Milestone".to_string());
                                return;
                            }
                        };

                        // Push current context to stack
                        self.navigation_stack.push(self.navigation_context.clone());

                        // Create new filtered context
                        self.navigation_context = NavigationContext::new_filtered(
                            child_level,
                            task_id,
                            task.title.clone(),
                        );

                        self.update_filtered_tasks();
                        self.set_status_message(format!(
                            "Navigated to {}",
                            self.navigation_context.get_display_name()
                        ));
                    }
                }
            } else {
                self.set_status_message("No item selected".to_string());
            }
        } else {
            // Go back to previous level
            if let Some(previous_context) = self.navigation_stack.pop() {
                self.navigation_context = previous_context;
                self.update_filtered_tasks();
                self.set_status_message(format!(
                    "Navigated back to {}",
                    self.navigation_context.get_display_name()
                ));
            } else {
                self.set_status_message("Already at top level".to_string());
            }
        }
    }

    /// Walk the parent chain from `leaf` up to the root, returning the chain
    /// root-first. Used to seed the Mode 2 breadcrumb from a selected ticket.
    pub(super) fn build_doc_crumb(&self, leaf: LeafId) -> Vec<LeafId> {
        let mut chain = vec![leaf];
        let mut cursor = leaf;
        while let Some(t) = self.db.get(cursor) {
            if let Some(parent) = t.parent {
                chain.insert(0, parent);
                cursor = parent;
            } else {
                break;
            }
        }
        chain
    }
}
