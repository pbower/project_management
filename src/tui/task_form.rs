//! Task form handling for the terminal user interface.
//!
//! This module provides the `TaskForm` structure and related functionality
//! for creating and editing tasks in the TUI, including field ordering
//! and form state management.

use crate::{
    fields::{Kind, Priority, ProcessStage, Status, Urgency}, 
    task::Task, 
    tui::{input::InputField, enums::{HierarchyLevel, NavigationContext}},
    project::{discover_projects, get_legacy_project}
};
use std::path::Path;

/// Global order constants for task editing view fields.
pub const TITLE_GLOBAL_ORDER: usize = 0;
pub const SUMMARY_GLOBAL_ORDER: usize = 1;
pub const DESCRIPTION_GLOBAL_ORDER: usize = 2;
pub const PROJECT_SELECTOR_GLOBAL_ORDER: usize = 3;
pub const TAGS_GLOBAL_ORDER: usize = 4;
pub const DUE_GLOBAL_ORDER: usize = 5;
pub const PARENT_GLOBAL_ORDER: usize = 6;
pub const ISSUE_LINK_GLOBAL_ORDER: usize = 7;
pub const PR_LINK_GLOBAL_ORDER: usize = 8;
pub const ARTIFACTS_GLOBAL_ORDER: usize = 9;
pub const KIND_GLOBAL_ORDER: usize = 10;
pub const STATUS_GLOBAL_ORDER: usize = 11;
pub const PRIORITY_GLOBAL_ORDER: usize = 12;
pub const URGENCY_GLOBAL_ORDER: usize = 13;
pub const PROCESS_STAGE_GLOBAL_ORDER: usize = 14;
pub const USER_STORY_GLOBAL_ORDER: usize = 15;
pub const REQUIREMENTS_GLOBAL_ORDER: usize = 16;

/// Task form for editing fields
pub struct TaskForm {
    pub title: InputField,
    pub summary: InputField,
    pub description: InputField,
    pub tags: InputField,
    pub due: InputField,
    pub parent: InputField,
    pub issue_link: InputField,
    pub pr_link: InputField,
    pub artifacts: InputField,
    pub project_selector: usize,
    pub kind: usize,
    pub status: usize,
    pub priority_level: usize,
    pub urgency: usize,
    pub process_stage: usize,
    pub current_field: usize,
    pub kinds: Vec<Kind>,
    pub statuses: Vec<Status>,
    pub priorities: Vec<Option<Priority>>,
    pub urgencies: Vec<Option<Urgency>>,
    pub process_stages: Vec<Option<ProcessStage>>,
    pub available_projects: Vec<String>,
    pub user_story: InputField,
    pub requirements: InputField,
}

impl TaskForm {
    /// Create a new task form with default PM directory.
    pub fn new() -> Self {
        Self::new_with_pm_dir(&Path::new(".pm"))
    }
    
    /// Create a new task form with the specified PM directory.
    pub fn new_with_pm_dir(pm_dir: &Path) -> Self {
        let available_projects = Self::discover_project_names(pm_dir);
        Self {
            title: InputField::new(),
            summary: InputField::new(),
            description: InputField::new(),
            tags: InputField::new(),
            due: InputField::new(),
            parent: InputField::new(),
            issue_link: InputField::new(),
            pr_link: InputField::new(),
            user_story: InputField::new(),
            requirements: InputField::new(),
            artifacts: InputField::new(),
            project_selector: 0, // Default to first available project
            kind: 2, // Task
            status: 0, // Open
            priority_level: 0, // None (first item)
            urgency: 0, // None (first item)
            process_stage: 0, // None (first item),
            current_field: 0,
            kinds: vec![Kind::Product, Kind::Epic, Kind::Task, Kind::Subtask, Kind::Milestone],
            statuses: vec![Status::Open, Status::InProgress, Status::Done],
            priorities: vec![None, Some(Priority::MustHave), Some(Priority::NiceToHave), Some(Priority::CutFirst)],
            urgencies: vec![None, Some(Urgency::UrgentImportant), Some(Urgency::UrgentNotImportant), 
                          Some(Urgency::NotUrgentImportant), Some(Urgency::NotUrgentNotImportant)],
            process_stages: vec![None, Some(ProcessStage::Ideation), Some(ProcessStage::Design), 
                               Some(ProcessStage::Prototyping), Some(ProcessStage::ReadyToImplement),
                               Some(ProcessStage::Implementation), Some(ProcessStage::Testing), 
                               Some(ProcessStage::Refinement), Some(ProcessStage::Release)],
            available_projects,
        }
    }
    
    /// Discover available project names in the PM directory.
    fn discover_project_names(pm_dir: &Path) -> Vec<String> {
        let mut project_names = Vec::new();
        
        // Add discovered projects
        if let Ok(projects) = discover_projects(pm_dir) {
            for project in projects {
                project_names.push(project.display_name);
            }
        }
        
        // Add legacy project if it exists
        if let Some(legacy) = get_legacy_project(pm_dir) {
            project_names.push(legacy.display_name);
        }
        
        // Ensure we have at least one project option
        if project_names.is_empty() {
            project_names.push("Default".to_string());
        }
        
        project_names
    }

    /// Create a new task form with navigation context for smart defaults.
    pub fn new_with_context(context: &NavigationContext) -> Self {
        Self::new_with_context_and_pm_dir(context, &Path::new(".pm"))
    }
    
    /// Create a new task form with navigation context and PM directory.
    pub fn new_with_context_and_pm_dir(context: &NavigationContext, pm_dir: &Path) -> Self {
        let mut form = Self::new_with_pm_dir(pm_dir);
        
        // Set parent ID if we're in a filtered view
        if let Some(parent_id) = context.parent_id {
            form.parent = InputField::with_value(&parent_id.to_string());
        }
        
        // Set the appropriate child kind based on the current navigation level
        // When viewing Products, create Epics (children of products)
        // When viewing Epics, create Tasks (children of epics), etc.
        let target_kind = match context.level {
            HierarchyLevel::Product => Kind::Epic,   // Products contain Epics
            HierarchyLevel::Epic => Kind::Task,      // Epics contain Tasks
            HierarchyLevel::Task => Kind::Subtask,   // Tasks contain Subtasks
            HierarchyLevel::Subtask => Kind::Subtask, // Subtasks can contain Subtasks
            HierarchyLevel::Milestone => Kind::Task,  // Default for Milestones
        };
        
        form.kind = form.kinds.iter().position(|&k| k == target_kind).unwrap_or(2);
        form
    }

    /// Create a task form populated from an existing task.
    pub fn from_task(task: &Task) -> Self {
        Self::from_task_with_pm_dir(task, &Path::new(".pm"))
    }
    
    /// Create a task form populated from an existing task with PM directory.
    pub fn from_task_with_pm_dir(task: &Task, pm_dir: &Path) -> Self {
        let mut form = Self::new_with_pm_dir(pm_dir);
        form.title = InputField::with_value(&task.title);
        form.summary = InputField::with_value(
            &task.summary.clone().unwrap_or_default());
        form.description = InputField::with_value(
            &task.description.clone().unwrap_or_default());
        // Set project selector based on task's project
        if let Some(ref project_name) = task.project {
            if let Some(index) = form.available_projects.iter().position(|p| p == project_name) {
                form.project_selector = index;
            }
        }
        form.tags = InputField::with_value(&task.tags.join(","));
        form.due = InputField::with_value(
            &task.due.map(|d| d.to_string()).unwrap_or_default());
        form.parent = InputField::with_value(
            &task.parent.map(|p| p.to_string()).unwrap_or_default());
        form.issue_link = InputField::with_value(
            &task.issue_link.clone().unwrap_or_default());
        form.pr_link = InputField::with_value(
            &task.pr_link.clone().unwrap_or_default());
        form.user_story = InputField::with_value(
            &task.user_story.clone().unwrap_or_default());
        form.requirements = InputField::with_value(
            &task.requirements.clone().unwrap_or_default());
        form.artifacts = InputField::with_value(
            &task.artifacts.join(","));
        form.kind = form.kinds.iter().position(|&k| k == task.kind).unwrap_or(2);
        form.status = form.statuses.iter().position(|&s| s == task.status).unwrap_or(0);
        form.priority_level = form.priorities.iter().position(|&p| p == task.priority_level).unwrap_or(0);
        form.urgency = form.urgencies.iter().position(|&u| u == task.urgency).unwrap_or(0);
        form.process_stage = form.process_stages.iter().position(|&s| s == task.process_stage).unwrap_or(0);
        form
    }

    /// Get mutable references to all input fields in visual order.
    pub fn fields_mut(&mut self) -> Vec<&mut InputField> {
        // Order matches the visual layout: left column first, then right column
        vec![
            &mut self.title,        // 0 - TITLE_GLOBAL_ORDER
            &mut self.summary,      // 1 - SUMMARY_GLOBAL_ORDER  
            &mut self.description,  // 2 - DESCRIPTION_GLOBAL_ORDER
            // PROJECT_SELECTOR at 3 - not in fields_mut(), handled as selector
            &mut self.tags,         // 3 in fields_mut, but TAGS_GLOBAL_ORDER (4) in navigation
            &mut self.due,          // 4 in fields_mut, but DUE_GLOBAL_ORDER (5) in navigation
            &mut self.parent,       // 5 in fields_mut, but PARENT_GLOBAL_ORDER (6) in navigation
            &mut self.issue_link,   // 6 in fields_mut, but ISSUE_LINK_GLOBAL_ORDER (7) in navigation
            &mut self.pr_link,      // 7 in fields_mut, but PR_LINK_GLOBAL_ORDER (8) in navigation
            &mut self.artifacts,    // 8 in fields_mut, but ARTIFACTS_GLOBAL_ORDER (9) in navigation
            // Note: selectors 10-14 are not in fields_mut() - they're handled separately
            &mut self.user_story,   // 9 in fields_mut, but USER_STORY_GLOBAL_ORDER (15) in navigation
            &mut self.requirements, // 10 in fields_mut, but REQUIREMENTS_GLOBAL_ORDER (16) in navigation
        ]
    }

    /// Get the total number of fields (input fields + selectors).
    pub fn field_count(&self) -> usize {
        17 // 11 text fields + 6 selectors (project=3, kind=10, status=11, priority=12, urgency=13, process_stage=14)
    }

    /// Move to the next field in the form.
    pub fn next_field(&mut self) {
        self.current_field = (self.current_field + 1) % self.field_count();
        self.update_active_field();
    }

    /// Move to the previous field in the form.
    pub fn prev_field(&mut self) {
        self.current_field = if self.current_field == 0 {
            self.field_count() - 1
        } else {
            self.current_field - 1
        };
        self.update_active_field();
    }

    /// Update which field is currently active for editing.
    pub fn update_active_field(&mut self) {
        for field in self.fields_mut() {
            field.active = false;
        }
        
        match self.current_field {
            TITLE_GLOBAL_ORDER => self.title.active = true,
            SUMMARY_GLOBAL_ORDER => self.summary.active = true,
            DESCRIPTION_GLOBAL_ORDER => self.description.active = true,
            TAGS_GLOBAL_ORDER => self.tags.active = true,
            DUE_GLOBAL_ORDER => self.due.active = true,
            PARENT_GLOBAL_ORDER => self.parent.active = true,
            ISSUE_LINK_GLOBAL_ORDER => self.issue_link.active = true,
            PR_LINK_GLOBAL_ORDER => self.pr_link.active = true,
            ARTIFACTS_GLOBAL_ORDER => self.artifacts.active = true,
            PROJECT_SELECTOR_GLOBAL_ORDER => {}, // project selector
            KIND_GLOBAL_ORDER => {}, // kind selector
            STATUS_GLOBAL_ORDER => {}, // status selector
            PRIORITY_GLOBAL_ORDER => {}, // priority_level selector
            URGENCY_GLOBAL_ORDER => {}, // urgency selector
            PROCESS_STAGE_GLOBAL_ORDER => {}, // process_stage selector
            USER_STORY_GLOBAL_ORDER => self.user_story.active = true,
            REQUIREMENTS_GLOBAL_ORDER => self.requirements.active = true,
            _ => {}
        }
    }

    /// Handle character input for the currently active field.
    pub fn handle_char(&mut self, c: char) {
        match self.current_field {
            TITLE_GLOBAL_ORDER => self.title.handle_char(c),
            SUMMARY_GLOBAL_ORDER => self.summary.handle_char(c),
            DESCRIPTION_GLOBAL_ORDER => self.description.handle_char(c),
            TAGS_GLOBAL_ORDER => self.tags.handle_char(c),
            DUE_GLOBAL_ORDER => self.due.handle_char(c),
            PARENT_GLOBAL_ORDER => self.parent.handle_char(c),
            ISSUE_LINK_GLOBAL_ORDER => self.issue_link.handle_char(c),
            PR_LINK_GLOBAL_ORDER => self.pr_link.handle_char(c),
            ARTIFACTS_GLOBAL_ORDER => self.artifacts.handle_char(c),
            USER_STORY_GLOBAL_ORDER => self.user_story.handle_char(c),
            REQUIREMENTS_GLOBAL_ORDER => self.requirements.handle_char(c),
            _ => {}
        }
    }

    /// Handle backspace input for the currently active field.
    pub fn handle_backspace(&mut self) {
        match self.current_field {
            TITLE_GLOBAL_ORDER => self.title.handle_backspace(),
            SUMMARY_GLOBAL_ORDER => self.summary.handle_backspace(),
            DESCRIPTION_GLOBAL_ORDER => self.description.handle_backspace(),
            TAGS_GLOBAL_ORDER => self.tags.handle_backspace(),
            DUE_GLOBAL_ORDER => self.due.handle_backspace(),
            PARENT_GLOBAL_ORDER => self.parent.handle_backspace(),
            ISSUE_LINK_GLOBAL_ORDER => self.issue_link.handle_backspace(),
            PR_LINK_GLOBAL_ORDER => self.pr_link.handle_backspace(),
            ARTIFACTS_GLOBAL_ORDER => self.artifacts.handle_backspace(),
            USER_STORY_GLOBAL_ORDER => self.user_story.handle_backspace(),
            REQUIREMENTS_GLOBAL_ORDER => self.requirements.handle_backspace(),
            _ => {}
        }
    }

    /// Handle left/right arrow keys for cursor movement or selector changes.
    pub fn handle_left_right(&mut self, right: bool) {
        match self.current_field {
            TITLE_GLOBAL_ORDER => if right { self.title.move_cursor_right() } else { self.title.move_cursor_left() },
            SUMMARY_GLOBAL_ORDER => if right { self.summary.move_cursor_right() } else { self.summary.move_cursor_left() },
            DESCRIPTION_GLOBAL_ORDER => if right { self.description.move_cursor_right() } else { self.description.move_cursor_left() },
            TAGS_GLOBAL_ORDER => if right { self.tags.move_cursor_right() } else { self.tags.move_cursor_left() },
            DUE_GLOBAL_ORDER => if right { self.due.move_cursor_right() } else { self.due.move_cursor_left() },
            PARENT_GLOBAL_ORDER => if right { self.parent.move_cursor_right() } else { self.parent.move_cursor_left() },
            ISSUE_LINK_GLOBAL_ORDER => if right { self.issue_link.move_cursor_right() } else { self.issue_link.move_cursor_left() },
            PR_LINK_GLOBAL_ORDER => if right { self.pr_link.move_cursor_right() } else { self.pr_link.move_cursor_left() },
            ARTIFACTS_GLOBAL_ORDER => if right { self.artifacts.move_cursor_right() } else { self.artifacts.move_cursor_left() },
            PROJECT_SELECTOR_GLOBAL_ORDER => {
                if right {
                    self.project_selector = (self.project_selector + 1) % self.available_projects.len();
                } else {
                    self.project_selector = if self.project_selector == 0 { self.available_projects.len() - 1 } else { self.project_selector - 1 };
                }
            },
            KIND_GLOBAL_ORDER => {
                if right {
                    self.kind = (self.kind + 1) % self.kinds.len();
                } else {
                    self.kind = if self.kind == 0 { self.kinds.len() - 1 } else { self.kind - 1 };
                }
            },
            STATUS_GLOBAL_ORDER => {
                if right {
                    self.status = (self.status + 1) % self.statuses.len();
                } else {
                    self.status = if self.status == 0 { self.statuses.len() - 1 } else { self.status - 1 };
                }
            },
            PRIORITY_GLOBAL_ORDER => {
                if right {
                    self.priority_level = (self.priority_level + 1) % self.priorities.len();
                } else {
                    self.priority_level = if self.priority_level == 0 { self.priorities.len() - 1 } else { self.priority_level - 1 };
                }
            },
            URGENCY_GLOBAL_ORDER => {
                if right {
                    self.urgency = (self.urgency + 1) % self.urgencies.len();
                } else {
                    self.urgency = if self.urgency == 0 { self.urgencies.len() - 1 } else { self.urgency - 1 };
                }
            },
            PROCESS_STAGE_GLOBAL_ORDER => {
                if right {
                    self.process_stage = (self.process_stage + 1) % self.process_stages.len();
                } else {
                    self.process_stage = if self.process_stage == 0 { self.process_stages.len() - 1 } else { self.process_stage - 1 };
                }
            },
            USER_STORY_GLOBAL_ORDER => if right { self.user_story.move_cursor_right() } else { self.user_story.move_cursor_left() },
            REQUIREMENTS_GLOBAL_ORDER => if right { self.requirements.move_cursor_right() } else { self.requirements.move_cursor_left() },
            _ => {}
        }
    }
    
    /// Get the currently selected project name.
    pub fn get_selected_project(&self) -> Option<String> {
        if self.project_selector < self.available_projects.len() {
            Some(self.available_projects[self.project_selector].clone())
        } else {
            None
        }
    }
}
