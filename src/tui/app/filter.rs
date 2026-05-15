//! Filtered task list maintenance. Owns `update_filtered_tasks` (recomputes
//! `App.filtered_tasks` from the current `Database` against the active
//! navigation context, completion-visibility toggle, and text filter) and
//! `refresh_tasks` (reload from disk + refilter).

use crate::db::{project_label, Database};
use crate::fields::{Kind, Status};
use crate::tui::enums::HierarchyLevel;

use super::App;

impl App {
    /// Reload the database from disk and refresh the filtered task list.
    pub(super) fn refresh_tasks(&mut self) {
        self.db = Database::load(&self.db_path);
        self.update_filtered_tasks();
    }

    /// Update the filtered task list based on current filters and navigation context.
    ///
    /// Applies completion status filter, hierarchy level filter, parent context filter,
    /// and search text filter. Attempts to preserve selection when possible.
    pub(super) fn update_filtered_tasks(&mut self) {
        // Remember the currently selected task ID if any
        let old_selected_id = self
            .task_list_state
            .selected()
            .and_then(|idx| self.filtered_tasks.get(idx))
            .copied();

        self.filtered_tasks = self
            .db
            .tasks
            .iter()
            .filter(|t| {
                // Filter by completion status
                if !self.show_completed && t.status == Status::Done {
                    return false;
                }

                // Filter by hierarchy level
                let required_kind = match self.navigation_context.level {
                    HierarchyLevel::Project => Kind::Project,
                    HierarchyLevel::Product => Kind::Product,
                    HierarchyLevel::Epic => Kind::Epic,
                    HierarchyLevel::Task => Kind::Task,
                    HierarchyLevel::Subtask => Kind::Subtask,
                    HierarchyLevel::Milestone => Kind::Milestone,
                };
                if t.kind != required_kind {
                    return false;
                }

                // Filter by parent context (for contextual drill-down)
                if let Some(parent_id) = self.navigation_context.parent_id {
                    if t.parent != Some(parent_id) {
                        return false;
                    }
                }

                // Filter by search text
                if !self.filter_text.is_empty() {
                    let filter_lower = self.filter_text.to_lowercase();
                    let project = project_label(&self.db, t);
                    if !t.title.to_lowercase().contains(&filter_lower)
                        && !t
                            .tags
                            .iter()
                            .any(|tag| tag.to_lowercase().contains(&filter_lower))
                        && !project.to_lowercase().contains(&filter_lower)
                    {
                        return false;
                    }
                }
                true
            })
            .map(|t| t.id)
            .collect();

        // Try to restore selection, or reset to first item
        if let Some(old_id) = old_selected_id {
            if let Some(new_idx) = self.filtered_tasks.iter().position(|&id| id == old_id) {
                self.task_list_state.select(Some(new_idx));
            } else {
                self.task_list_state
                    .select(if self.filtered_tasks.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
            }
        } else if !self.filtered_tasks.is_empty() && self.task_list_state.selected().is_none() {
            self.task_list_state.select(Some(0));
        } else if self.filtered_tasks.is_empty() {
            self.task_list_state.select(None);
        }
    }
}
