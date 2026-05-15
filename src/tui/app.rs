//! Main application logic for the terminal user interface.
//!
//! This module contains the `App` struct which manages the TUI state,
//! handles user input, renders the interface, and coordinates between
//! different screens (task list, forms, dialogs).

use std::io;
use std::path::Path;
use std::time::Duration;

use chrono::Local;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap},
    Frame, Terminal,
};

use std::collections::HashMap;

use chrono::Utc;

use crate::{fields::*, tui::colors::{DARK_GREEN, DARK_PURPLE, DARK_RED, GOLD}};
use crate::store::{IdInput, LeafId, MemoryRef};
use crate::store::locks::{self, LockFile, LockMode, AcquireOutcome, DEFAULT_TTL_SECONDS};
use crate::store::events;
use crate::store::git;
use crate::store::artifacts;
use crate::task::Task;
use crate::{
    db::{*, format_kind, format_status, format_priority, format_urgency, format_process_stage, format_due_relative, project_label, kind_to_prefix},
    tui::{
        enums::{
            AppState, DocumentsState, HierarchyLevel, InputMode, Mode, NavigationContext,
            Overlay, PendingAction, PromptState, PromptType,
        },
        task_form::{
            TaskForm, ARTIFACTS_GLOBAL_ORDER, DESCRIPTION_GLOBAL_ORDER, DUE_GLOBAL_ORDER,
            ISSUE_LINK_GLOBAL_ORDER, KIND_GLOBAL_ORDER, PARENT_GLOBAL_ORDER, PRIORITY_GLOBAL_ORDER,
            PROCESS_STAGE_GLOBAL_ORDER, PROJECT_SELECTOR_GLOBAL_ORDER, PR_LINK_GLOBAL_ORDER,
            REQUIREMENTS_GLOBAL_ORDER, STATUS_GLOBAL_ORDER, SUMMARY_GLOBAL_ORDER,
            TAGS_GLOBAL_ORDER, TITLE_GLOBAL_ORDER, URGENCY_GLOBAL_ORDER, USER_STORY_GLOBAL_ORDER,
        },
        utils::centered_rect,
    },
};

/// State snapshot for navigation history.
#[derive(Clone)]
struct NavigationSnapshot {
    state: AppState,
    context: NavigationContext,
    selected_task: Option<LeafId>,
}

/// Main application state for the terminal user interface.
///
/// Manages all TUI state including current screen, database operations,
/// task filtering, navigation context, and user interactions.
pub struct App {
    mode: Mode,
    state: AppState,
    db: Database,
    db_path: std::path::PathBuf,
    task_list_state: TableState,
    filtered_tasks: Vec<LeafId>,
    selected_task: Option<LeafId>,
    task_form: TaskForm,
    input_mode: InputMode,
    status_message: String,
    show_completed: bool,
    filter_text: String,
    filter_active: bool,
    confirm_action: Option<String>,
    dialog_text: String,
    dialog_cursor_x: usize,
    dialog_cursor_y: usize,
    dialog_scroll_y: usize,
    navigation_context: NavigationContext,
    navigation_stack: Vec<NavigationContext>,
    navigation_history: Vec<NavigationSnapshot>,
    max_history: usize,
    pm_dir: std::path::PathBuf,
    /// The transient surface layered over the current mode, if any. One field
    /// rather than a scatter of flags so at most one overlay can be active.
    overlay: Overlay,
    /// A deferred terminal-suspending action picked up by the run loop. Kept
    /// separate from `overlay` because it is an action to run, not a surface.
    pending_action: Option<PendingAction>,
    /// State for Mode 2 - the Document Workspace. Maintained across mode
    /// switches so the cursor and crumb persist when the user returns.
    documents: DocumentsState,
}

impl App {
    /// Create a new App instance, loading the database from the specified path.
    pub fn new(db_path: &Path) -> io::Result<Self> {
        let db = Database::load(db_path);
        let navigation_context = NavigationContext::new_all_projects();
        let pm_dir = db_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        
        let mut app = App {
            mode: Mode::Tickets,
            state: AppState::TaskList,
            db,
            db_path: db_path.to_path_buf(),
            task_list_state: TableState::default(),
            filtered_tasks: Vec::new(),
            selected_task: None,
            task_form: TaskForm::new_with_pm_dir(&pm_dir),
            input_mode: InputMode::None,
            status_message: String::new(),
            show_completed: false,
            filter_text: String::new(),
            filter_active: false,
            confirm_action: None,
            dialog_text: String::new(),
            dialog_cursor_x: 0,
            dialog_cursor_y: 0,
            dialog_scroll_y: 0,
            navigation_context,
            navigation_stack: Vec::new(),
            navigation_history: Vec::new(),
            max_history: 10,
            pm_dir,
            overlay: Overlay::None,
            pending_action: None,
            documents: DocumentsState::default(),
        };
        
        app.update_filtered_tasks();
        Ok(app)
    }
    
    /// Push current state to navigation history and transition to new state.
    fn push_state(&mut self, new_state: AppState, new_context: Option<NavigationContext>) {
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
    fn go_back(&mut self) -> bool {
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
    fn has_navigation_history(&self) -> bool {
        !self.navigation_history.is_empty()
    }
    
    /// Get the current project name from the database path.
    fn get_current_project_name(&self) -> String {
        use crate::project::Project;
        
        if let Some(project) = Project::from_file(self.db_path.clone()) {
            project.display_name
        } else {
            // Fallback for legacy tasks.json
            "Default (Legacy)".to_string()
        }
    }
    
    /// Open a specific task for editing.
    pub fn open_task_for_edit(&mut self, task_id: LeafId) {
        if let Some(task) = self.db.get(task_id) {
            self.selected_task = Some(task_id);
            self.task_form = TaskForm::from_task_with_pm_dir(task, &self.pm_dir);
            self.task_form.update_active_field();
            self.push_state(AppState::EditTask, None);
            self.input_mode = InputMode::Text;
        }
    }
    
    /// Check if the user wants to return to the main menu.

    /// Reload the database from disk and refresh the filtered task list.
    fn refresh_tasks(&mut self) {
        self.db = Database::load(&self.db_path);
        self.update_filtered_tasks();
    }

    /// Update the filtered task list based on current filters and navigation context.
    /// 
    /// Applies completion status filter, hierarchy level filter, parent context filter,
    /// and search text filter. Attempts to preserve selection when possible.
    fn update_filtered_tasks(&mut self) {
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

    /// Save the database to disk and refresh the task list.
    fn save_db(&mut self) -> io::Result<()> {
        self.db.save(&self.db_path)?;
        self.refresh_tasks();
        Ok(())
    }

    /// Get a reference to the currently selected task.
    fn get_selected_task(&self) -> Option<&Task> {
        self.selected_task.and_then(|id| self.db.get(id))
    }

    /// The `LeafId` highlighted in the task table, if any.
    fn selected_task_id(&self) -> Option<LeafId> {
        self.task_list_state
            .selected()
            .and_then(|idx| self.filtered_tasks.get(idx))
            .copied()
    }

    /// Checkout the highlighted ticket - acquire a soft lock and emit a
    /// `checkout` event. Soft locks warn on overlap but still proceed.
    fn do_checkout(&mut self) {
        let Some(task_id) = self.selected_task_id() else {
            self.set_status_message("No ticket selected".to_string());
            return;
        };
        let base_commit = git::head_commit(&self.pm_dir).ok().flatten();
        let lock = LockFile::new(task_id, None, DEFAULT_TTL_SECONDS, LockMode::Soft, base_commit);
        match locks::acquire(&self.pm_dir, &lock, Utc::now()) {
            Ok(AcquireOutcome::Acquired) => {
                let _ = events::emit_event(&self.pm_dir, "checkout", Some(task_id), None);
                self.set_status_message(format!("{task_id} checked out by {}", lock.agent));
            }
            Ok(AcquireOutcome::Overlapped { previous }) => {
                let _ = events::emit_event(&self.pm_dir, "checkout", Some(task_id), None);
                self.set_status_message(format!(
                    "{task_id} checked out (soft lock; was held by {})",
                    previous.agent
                ));
            }
            Ok(AcquireOutcome::Blocked { holder }) => {
                self.set_status_message(format!(
                    "{task_id} is hard-locked by {}; cannot check out",
                    holder.agent
                ));
            }
            Err(e) => self.set_status_message(format!("checkout failed: {e}")),
        }
    }

    /// Checkin the highlighted ticket - release its lock and emit a `checkin`
    /// event. The git squash that `pm checkin` performs is left to the CLI
    /// verb; the in-TUI path releases the lock and records the event.
    fn do_checkin(&mut self) {
        let Some(task_id) = self.selected_task_id() else {
            self.set_status_message("No ticket selected".to_string());
            return;
        };
        match locks::release(&self.pm_dir, task_id) {
            Ok(true) => {
                let _ = events::emit_event(&self.pm_dir, "checkin", Some(task_id), None);
                self.set_status_message(format!("{task_id} checked in"));
            }
            Ok(false) => self.set_status_message(format!("{task_id} was not checked out")),
            Err(e) => self.set_status_message(format!("checkin failed: {e}")),
        }
    }

    /// Set a status message to display in the status bar.
    fn set_status_message(&mut self, msg: String) {
        self.status_message = msg;
    }

    /// Clear the current status message.
    fn clear_status_message(&mut self) {
        self.status_message.clear();
    }
    
    /// Get the theme color for the current hierarchy level.
    fn get_hierarchy_color(&self) -> Color {
        match self.navigation_context.level {
            HierarchyLevel::Project => Color::Cyan,      // Cyan for the top-level Project tickets
            HierarchyLevel::Product => Color::Blue,        // Dark Blue (keeping existing)
            HierarchyLevel::Epic => DARK_GREEN,          // Forest Green
            HierarchyLevel::Task => GOLD,               // Gold
            HierarchyLevel::Subtask => DARK_RED,         // Crimson Red
            HierarchyLevel::Milestone => DARK_PURPLE,   // Magenta for milestones
        }
    }

    /// Navigate between hierarchy levels without parent filtering.
    /// 
    /// Shows all items of the target hierarchy level (Product, Epic, Task, etc.)
    /// rather than drilling down into a specific parent's children.
    fn navigate_hierarchy_unfiltered(&mut self, forward: bool) {
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
        self.set_status_message(format!("Navigated to {}", self.navigation_context.get_display_name()));
    }
    
    /// Navigate contextually through the hierarchy by drilling down or going back.
    /// 
    /// Forward navigation drills down into the selected item's children.
    /// Backward navigation returns to the previous context using the navigation stack.
    fn navigate_hierarchy_contextual(&mut self, forward: bool) {
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
                            task.title.clone()
                        );
                        
                        self.update_filtered_tasks();
                        self.set_status_message(format!("Navigated to {}", self.navigation_context.get_display_name()));
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
                self.set_status_message(format!("Navigated back to {}", self.navigation_context.get_display_name()));
            } else {
                self.set_status_message("Already at top level".to_string());
            }
        }
    }

    /// Handle keyboard input when in the task list view.
    /// 
    /// Returns true if the application should quit.
    fn handle_task_list_input(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> io::Result<bool> {
        if self.filter_active {
            match key {
                KeyCode::Esc => {
                    self.filter_active = false;
                    self.filter_text.clear();
                    self.input_mode = InputMode::None;
                    self.update_filtered_tasks();
                    self.clear_status_message();
                }
                KeyCode::Enter => {
                    self.filter_active = false;
                    self.input_mode = InputMode::None;
                    if self.filter_text.is_empty() {
                        self.set_status_message("Filter cleared".to_string());
                    } else {
                        self.set_status_message(format!(
                            "Filter applied: '{}' ({} tasks)",
                            self.filter_text,
                            self.filtered_tasks.len()
                        ));
                    }
                }
                KeyCode::Backspace => {
                    if !self.filter_text.is_empty() {
                        self.filter_text.pop();
                        self.update_filtered_tasks();
                    }
                }
                KeyCode::Char(c) => {
                    self.filter_text.push(c);
                    self.update_filtered_tasks();
                }
                _ => {}
            }
            return Ok(false);
        }

        match key {
            KeyCode::Char('q') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            KeyCode::Left if modifiers.contains(KeyModifiers::ALT) => {
                if self.go_back() {
                    self.set_status_message("Navigated back".to_string());
                } else {
                    self.set_status_message("No navigation history".to_string());
                }
            },
            KeyCode::Esc => {
                if self.filter_active || !self.filter_text.is_empty() {
                    self.filter_active = false;
                    self.filter_text.clear();
                    self.update_filtered_tasks();
                    self.clear_status_message();
                    self.input_mode = InputMode::None;
                } else {
                    return Ok(true);
                }
            }

            KeyCode::Up => {
                if let Some(selected) = self.task_list_state.selected() {
                    if selected > 0 {
                        self.task_list_state.select(Some(selected - 1));
                    }
                } else if !self.filtered_tasks.is_empty() {
                    self.task_list_state.select(Some(0));
                }
            }
            KeyCode::Down => {
                if let Some(selected) = self.task_list_state.selected() {
                    if selected + 1 < self.filtered_tasks.len() {
                        self.task_list_state.select(Some(selected + 1));
                    }
                } else if !self.filtered_tasks.is_empty() {
                    self.task_list_state.select(Some(0));
                }
            }
            KeyCode::Left => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    // Shift+Left: Navigate to previous hierarchy level (unfiltered)
                    self.navigate_hierarchy_unfiltered(false);
                } else {
                    // Left: Navigate back in contextual hierarchy
                    self.navigate_hierarchy_contextual(false);
                }
            }
            KeyCode::Right => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    // Shift+Right: Navigate to next hierarchy level (unfiltered)
                    self.navigate_hierarchy_unfiltered(true);
                } else {
                    // Right: Navigate forward in contextual hierarchy (drill down)
                    self.navigate_hierarchy_contextual(true);
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(selected) = self.task_list_state.selected() {
                    if let Some(&task_id) = self.filtered_tasks.get(selected) {
                        self.selected_task = Some(task_id);
                        self.push_state(AppState::TaskDetail, None);
                    }
                }
            }
            // `n` opens the quick-entry form for a new child ticket.
            KeyCode::Char('n') => {
                self.task_form = TaskForm::new_with_context_and_pm_dir(&self.navigation_context, &self.pm_dir);
                self.task_form.update_active_field();
                self.push_state(AppState::AddTask, None);
                self.input_mode = InputMode::Text;
            }
            // `f` opens the quick-entry form on the selected ticket.
            KeyCode::Char('f') => {
                if let Some(selected) = self.task_list_state.selected() {
                    if let Some(&task_id) = self.filtered_tasks.get(selected) {
                        if let Some(task) = self.db.get(task_id) {
                            self.selected_task = Some(task_id);
                            self.task_form = TaskForm::from_task_with_pm_dir(task, &self.pm_dir);
                            self.task_form.update_active_field();
                            self.push_state(AppState::EditTask, None);
                            self.input_mode = InputMode::Text;
                        }
                    }
                }
            }
            // `e` opens the selected ticket's CLAUDE.md in `$EDITOR`. The run
            // loop performs the terminal suspend/resume around the handoff.
            KeyCode::Char('e') => {
                if let Some(task_id) = self.selected_task_id() {
                    self.pending_action = Some(PendingAction::EditTicket(task_id));
                } else {
                    self.set_status_message("No ticket selected".to_string());
                }
            }
            // `a` adds an artifact to the selected ticket via a path prompt.
            KeyCode::Char('a') => {
                if let Some(task_id) = self.selected_task_id() {
                    self.overlay = Overlay::Prompt(PromptState {
                        prompt_type: PromptType::ArtifactPath(task_id),
                        buffer: String::new(),
                    });
                } else {
                    self.set_status_message("No ticket selected".to_string());
                }
            }
            KeyCode::Char('i') => self.do_checkin(),
            KeyCode::Char('m') => {
                self.overlay = if matches!(self.overlay, Overlay::MemoryPanel) {
                    Overlay::None
                } else {
                    Overlay::MemoryPanel
                };
            }
            KeyCode::Char('d') => {
                if let Some(selected) = self.task_list_state.selected() {
                    if let Some(&task_id) = self.filtered_tasks.get(selected) {
                        self.selected_task = Some(task_id);
                        self.confirm_action = Some(format!("Delete task #{}", task_id));
                        self.state = AppState::Confirm;
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(selected) = self.task_list_state.selected() {
                    if let Some(&task_id) = self.filtered_tasks.get(selected) {
                        if let Some(task) = self.db.get_mut(task_id) {
                            // Cycle through all three status states: Open -> InProgress -> Done -> Open
                            let new_status = match task.status {
                                Status::Open => Status::InProgress,
                                Status::InProgress => Status::Done,
                                Status::Done => Status::Open,
                            };
                            task.status = new_status;
                            if let Err(e) = self.save_db() {
                                self.set_status_message(format!("Error saving: {}", e));
                            } else {
                                self.set_status_message(format!("Task status updated to {}", format_status(new_status)));
                            }
                        }
                    }
                }
            }
            // `c` checks out the selected ticket (acquires a soft lock).
            // Status toggling lives on `s`, which cycles through Done.
            KeyCode::Char('c') => self.do_checkout(),
            KeyCode::Char('p') => {
                if let Some(selected) = self.task_list_state.selected() {
                    if let Some(&task_id) = self.filtered_tasks.get(selected) {
                        if let Some(task) = self.db.get_mut(task_id) {
                            // Cycle through process stages: Ideation -> Design -> Prototyping -> Ready to Implement -> Implementation -> Testing -> Refinement -> Release -> Ideation
                            let new_stage = match task.process_stage {
                                Some(ProcessStage::Ideation) => ProcessStage::Design,
                                Some(ProcessStage::Design) => ProcessStage::Prototyping,
                                Some(ProcessStage::Prototyping) => ProcessStage::ReadyToImplement,
                                Some(ProcessStage::ReadyToImplement) => ProcessStage::Implementation,
                                Some(ProcessStage::Implementation) => ProcessStage::Testing,
                                Some(ProcessStage::Testing) => ProcessStage::Refinement,
                                Some(ProcessStage::Refinement) => ProcessStage::Release,
                                Some(ProcessStage::Release) => ProcessStage::Ideation,
                                None => ProcessStage::Ideation, // Start with Ideation if no stage set
                            };
                            task.process_stage = Some(new_stage);
                            if let Err(e) = self.save_db() {
                                self.set_status_message(format!("Error saving: {}", e));
                            } else {
                                self.set_status_message(format!("Process stage updated to {}", format_process_stage(Some(new_stage))));
                            }
                        }
                    }
                }
            }
            KeyCode::Char('t') => {
                self.show_completed = !self.show_completed;
                self.update_filtered_tasks();
                self.set_status_message(if self.show_completed {
                    format!("Showing all tasks ({} total)", self.filtered_tasks.len())
                } else {
                    format!(
                        "Hiding completed tasks ({} visible)",
                        self.filtered_tasks.len()
                    )
                });
            }
            KeyCode::Char('/') => {
                self.filter_active = true;
                self.input_mode = InputMode::Text;
                self.set_status_message(
                    "Filter mode: Type to search title/tags/project, Enter to apply, Esc to cancel"
                        .to_string(),
                );
            }
            KeyCode::Char('h') => {
                self.overlay = Overlay::Help { scroll: 0 };
            }
            KeyCode::Char('r') => {
                self.refresh_tasks();
                self.set_status_message("Tasks refreshed".to_string());
            }
            _ => {}
        }
        Ok(false)
    }

    /// Handle keyboard input when viewing task details.
    /// 
    /// Returns true if the application should quit.
    fn handle_detail_input(&mut self, key: KeyCode, _modifiers: KeyModifiers) -> io::Result<bool> {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state = AppState::TaskList;
            }
            KeyCode::Char('e') => {
                if let Some(task_id) = self.selected_task {
                    if let Some(task) = self.db.get(task_id) {
                        self.task_form = TaskForm::from_task_with_pm_dir(task, &self.pm_dir);
                        self.task_form.update_active_field();
                        self.push_state(AppState::EditTask, None);
                        self.input_mode = InputMode::Text;
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(task_id) = self.selected_task {
                    self.confirm_action = Some(format!("Delete task #{}", task_id));
                    self.push_state(AppState::Confirm, None);
                }
            }
            KeyCode::Char('p') => {
                // Go to parent
                if let Some(task_id) = self.selected_task {
                    if let Some(task) = self.db.get(task_id) {
                        if let Some(parent_id) = task.parent {
                            self.selected_task = Some(parent_id);
                            self.set_status_message(format!(
                                "Navigated to parent task #{}",
                                parent_id
                            ));
                        } else {
                            self.set_status_message("No parent task".to_string());
                        }
                    }
                }
            }
            KeyCode::Char('c') => {
                // Go to first child
                if let Some(task_id) = self.selected_task {
                    let child_map = build_children_map(&self.db.tasks);
                    if let Some(children) = child_map.get(&task_id) {
                        if let Some(&first_child) = children.first() {
                            self.selected_task = Some(first_child);
                            self.set_status_message(format!(
                                "Navigated to child task #{}",
                                first_child
                            ));
                        } else {
                            self.set_status_message("No child tasks".to_string());
                        }
                    } else {
                        self.set_status_message("No child tasks".to_string());
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    /// Handle keyboard input when in task creation or editing forms.
    /// 
    /// Returns true if the application should quit.
    fn handle_form_input(&mut self, key: KeyCode, _modifiers: KeyModifiers, is_edit: bool) -> io::Result<bool> {
        match key {
            KeyCode::Esc => {
                self.state = AppState::TaskList;
                self.input_mode = InputMode::None;
            }
            KeyCode::Tab => {
                self.task_form.next_field();
            }
            KeyCode::BackTab => {
                self.task_form.prev_field();
            }
            KeyCode::Up => {
                self.task_form.prev_field();
            }
            KeyCode::Down => {
                self.task_form.next_field();
            }
            KeyCode::Left => {
                self.task_form.handle_left_right(false);
            }
            KeyCode::Right => {
                self.task_form.handle_left_right(true);
            }
            KeyCode::Backspace => {
                self.task_form.handle_backspace();
            }
            KeyCode::Delete => match self.task_form.current_field {
                TITLE_GLOBAL_ORDER => self.task_form.title.handle_delete(),
                SUMMARY_GLOBAL_ORDER => self.task_form.summary.handle_delete(),
                DESCRIPTION_GLOBAL_ORDER => self.task_form.description.handle_delete(),
                PROJECT_SELECTOR_GLOBAL_ORDER => {}, // Project selector doesn't support delete
                TAGS_GLOBAL_ORDER => self.task_form.tags.handle_delete(),
                DUE_GLOBAL_ORDER => self.task_form.due.handle_delete(),
                PARENT_GLOBAL_ORDER => self.task_form.parent.handle_delete(),
                ISSUE_LINK_GLOBAL_ORDER => self.task_form.issue_link.handle_delete(),
                PR_LINK_GLOBAL_ORDER => self.task_form.pr_link.handle_delete(),
                ARTIFACTS_GLOBAL_ORDER => self.task_form.artifacts.handle_delete(),
                USER_STORY_GLOBAL_ORDER => self.task_form.user_story.handle_delete(),
                REQUIREMENTS_GLOBAL_ORDER => self.task_form.requirements.handle_delete(),
                _ => {}
            },
            KeyCode::Enter => {
                // Check if we're on User Story or Requirements field for fullscreen dialog
                match self.task_form.current_field {
                    USER_STORY_GLOBAL_ORDER => {
                        // User Story field
                        self.push_state(AppState::UserStoryDialog, None);
                        self.dialog_text = self.task_form.user_story.value.clone();
                        self.init_dialog_cursor();
                        return Ok(false);
                    }
                    REQUIREMENTS_GLOBAL_ORDER => {
                        // Requirements field
                        self.push_state(AppState::RequirementsDialog, None);
                        self.dialog_text = self.task_form.requirements.value.clone();
                        self.init_dialog_cursor();
                        return Ok(false);
                    }
                    _ => {
                        // Regular form submission
                        if self.task_form.title.value.trim().is_empty() {
                            self.set_status_message("Title is required".to_string());
                            return Ok(false);
                        }

                        let result = if is_edit {
                            self.update_task()
                        } else {
                            self.create_task()
                        };

                        match result {
                            Ok(_) => {
                                self.state = AppState::TaskList;
                                self.input_mode = InputMode::None;
                                self.set_status_message(
                                    if is_edit {
                                        "Task updated"
                                    } else {
                                        "Task created"
                                    }
                                    .to_string(),
                                );
                            }
                            Err(e) => {
                                self.set_status_message(format!("Error: {}", e));
                            }
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                self.task_form.handle_char(c);
            }
            _ => {}
        }
        Ok(false)
    }

    /// Create a new task from the current form data.
    ///
    /// Validates input, enforces hierarchy rules, and adds the task to the database.
    fn create_task(&mut self) -> io::Result<()> {
        let now_utc = chrono::Utc::now().timestamp();
        let task_kind = self.task_form.kinds[self.task_form.kind];
        let id = self.db.allocate_id(kind_to_prefix(task_kind));

        let parent = if self.task_form.parent.value.trim().is_empty() {
            None
        } else {
            match self.task_form.parent.value.trim().parse::<IdInput>() {
                Ok(parsed) => {
                    let pid = parsed.leaf();
                    if pid == id {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Task cannot be its own parent",
                        ));
                    }
                    if self.db.get(pid).is_none() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Parent {} does not exist", pid),
                        ));
                    }

                    // Validate hierarchy rules
                    if let Some(parent_task) = self.db.get(pid) {
                        if !validate_hierarchy(parent_task.kind, task_kind) {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!("Invalid hierarchy: {} cannot be child of {}. Valid hierarchy: Project > Product > Epic > Task > Subtask",
                                    format_kind(task_kind), format_kind(parent_task.kind)),
                            ));
                        }
                    }

                    Some(pid)
                }
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid parent ID",
                    ))
                }
            }
        };

        let due = if self.task_form.due.value.trim().is_empty() {
            None
        } else {
            parse_due_input(&self.task_form.due.value)
        };

        let task = Task {
            id,
            title: self.task_form.title.value.trim().to_string(),
            summary: if self.task_form.summary.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.summary.value.trim().to_string())
            },
            description: if self.task_form.description.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.description.value.trim().to_string())
            },
            user_story: if self.task_form.user_story.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.user_story.value.trim().to_string())
            },
            requirements: if self.task_form.requirements.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.requirements.value.trim().to_string())
            },
            tags: split_and_normalise_tags(&[self.task_form.tags.value.clone()]),
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
            due,
            parent,
            kind: task_kind,
            status: self.task_form.statuses[self.task_form.status],
            priority_level: self.task_form.priorities[self.task_form.priority_level],
            urgency: self.task_form.urgencies[self.task_form.urgency],
            process_stage: self.task_form.process_stages[self.task_form.process_stage],
            issue_link: if self.task_form.issue_link.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.issue_link.value.trim().to_string())
            },
            pr_link: if self.task_form.pr_link.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.pr_link.value.trim().to_string())
            },
            artifacts: if self.task_form.artifacts.value.trim().is_empty() {
                Vec::new()
            } else {
                self.task_form
                    .artifacts
                    .value
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            },
            created_at_utc: now_utc,
            updated_at_utc: now_utc,
        };

        self.db.tasks.push(task);
        self.save_db()
    }

    /// Update the selected task with data from the current form.
    /// 
    /// Validates input and saves changes to the database.
    fn update_task(&mut self) -> io::Result<()> {
        let task_id = self
            .selected_task
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "No task selected"))?;

        let parent = if self.task_form.parent.value.trim().is_empty() {
            None
        } else {
            match self.task_form.parent.value.trim().parse::<IdInput>() {
                Ok(parsed) => {
                    let pid = parsed.leaf();
                    if pid == task_id {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Task cannot be its own parent",
                        ));
                    }
                    if self.db.get(pid).is_none() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Parent {} does not exist", pid),
                        ));
                    }
                    Some(pid)
                }
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid parent ID",
                    ))
                }
            }
        };

        let due = if self.task_form.due.value.trim().is_empty() {
            None
        } else {
            parse_due_input(&self.task_form.due.value)
        };

        if let Some(task) = self.db.get_mut(task_id) {
            task.title = self.task_form.title.value.trim().to_string();
            task.description = if self.task_form.description.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.description.value.trim().to_string())
            };
            task.tags = split_and_normalise_tags(&[self.task_form.tags.value.clone()]);
            task.due = due;
            task.parent = parent;
            task.kind = self.task_form.kinds[self.task_form.kind];
            task.status = self.task_form.statuses[self.task_form.status];
            task.priority_level = self.task_form.priorities[self.task_form.priority_level];
            task.urgency = self.task_form.urgencies[self.task_form.urgency];
            task.process_stage = self.task_form.process_stages[self.task_form.process_stage];
            task.summary = if self.task_form.summary.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.summary.value.trim().to_string())
            };
            task.user_story = if self.task_form.user_story.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.user_story.value.trim().to_string())
            };
            task.requirements = if self.task_form.requirements.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.requirements.value.trim().to_string())
            };
            task.issue_link = if self.task_form.issue_link.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.issue_link.value.trim().to_string())
            };
            task.pr_link = if self.task_form.pr_link.value.trim().is_empty() {
                None
            } else {
                Some(self.task_form.pr_link.value.trim().to_string())
            };
            task.artifacts = if self.task_form.artifacts.value.trim().is_empty() {
                Vec::new()
            } else {
                self.task_form
                    .artifacts
                    .value
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            task.updated_at_utc = chrono::Utc::now().timestamp();
        }

        self.save_db()
    }

    /// Delete the selected task and all its descendants.
    /// 
    /// Cascades deletion to all child tasks in the hierarchy.
    fn delete_selected_task(&mut self) -> io::Result<()> {
        if let Some(task_id) = self.selected_task {
            let child_map = build_children_map(&self.db.tasks);
            let mut to_delete = std::collections::HashSet::new();

            fn collect_descendants(
                id: LeafId,
                child_map: &std::collections::BTreeMap<LeafId, Vec<LeafId>>,
                out: &mut std::collections::HashSet<LeafId>,
            ) {
                if let Some(children) = child_map.get(&id) {
                    for &child in children {
                        if out.insert(child) {
                            collect_descendants(child, child_map, out);
                        }
                    }
                }
            }

            to_delete.insert(task_id);
            collect_descendants(task_id, &child_map, &mut to_delete);

            self.db.remove_ids(&to_delete);
            self.save_db()?;
            self.set_status_message(format!("Deleted {} task(s)", to_delete.len()));
        }
        Ok(())
    }

    /// Handle keyboard input in the confirmation dialog.
    /// 
    /// Returns true if the application should quit.
    fn handle_confirm_input(&mut self, key: KeyCode, _modifiers: KeyModifiers) -> io::Result<bool> {
        match key {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if self.confirm_action.is_some() {
                    if let Err(e) = self.delete_selected_task() {
                        self.set_status_message(format!("Error deleting task: {}", e));
                    }
                }
                self.state = AppState::TaskList;
                self.confirm_action = None;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.state = AppState::TaskList;
                self.confirm_action = None;
            }
            _ => {}
        }
        Ok(false)
    }

    /// Handle keyboard input in fullscreen text editing dialogs.
    /// 
    /// Used for editing user stories and requirements in dedicated fullscreen mode.
    /// Returns true if the application should quit.
    fn handle_dialog_input(&mut self, key: KeyCode, modifiers: KeyModifiers, is_user_story: bool) -> io::Result<bool> {
        match key {
            KeyCode::Esc => {
                // Save the dialog text back to the form and return to form
                if is_user_story {
                    self.task_form.user_story.value = self.dialog_text.clone();
                } else {
                    self.task_form.requirements.value = self.dialog_text.clone();
                }
                self.state = if self.selected_task.is_some() {
                    AppState::EditTask
                } else {
                    AppState::AddTask
                };
                self.input_mode = InputMode::Text;
            }
            KeyCode::Char(c) => {
                // Insert character at cursor position
                let cursor_pos = self.get_dialog_cursor_position();
                self.dialog_text.insert(cursor_pos, c);
                self.move_dialog_cursor_right();
            }
            KeyCode::Backspace => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Backspace: Clear entire field
                    self.dialog_text.clear();
                    self.dialog_cursor_x = 0;
                    self.dialog_cursor_y = 0;
                    self.dialog_scroll_y = 0;
                } else {
                    // Regular Backspace: Remove character before cursor
                    let cursor_pos = self.get_dialog_cursor_position();
                    if cursor_pos > 0 {
                        self.dialog_text.remove(cursor_pos - 1);
                        self.move_dialog_cursor_left();
                    }
                }
            }
            KeyCode::Delete => {
                if modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Delete: Clear entire field
                    self.dialog_text.clear();
                    self.dialog_cursor_x = 0;
                    self.dialog_cursor_y = 0;
                    self.dialog_scroll_y = 0;
                } else {
                    // Regular Delete: Remove character at cursor
                    let cursor_pos = self.get_dialog_cursor_position();
                    if cursor_pos < self.dialog_text.len() {
                        self.dialog_text.remove(cursor_pos);
                    }
                }
            }
            KeyCode::Enter => {
                let cursor_pos = self.get_dialog_cursor_position();
                self.dialog_text.insert(cursor_pos, '\n');
                self.dialog_cursor_x = 0;
                self.dialog_cursor_y += 1;
            }
            KeyCode::Left => {
                self.move_dialog_cursor_left();
            }
            KeyCode::Right => {
                self.move_dialog_cursor_right();
            }
            KeyCode::Up => {
                self.move_dialog_cursor_up();
            }
            KeyCode::Down => {
                self.move_dialog_cursor_down();
            }
            KeyCode::Home => {
                self.dialog_cursor_x = 0;
            }
            KeyCode::End => {
                let lines: Vec<&str> = self.dialog_text.lines().collect();
                if let Some(current_line) = lines.get(self.dialog_cursor_y) {
                    self.dialog_cursor_x = current_line.len();
                }
            }
            _ => {}
        }
        Ok(false)
    }

    /// Get the current cursor position in the dialog text as a character index.
    fn get_dialog_cursor_position(&self) -> usize {
        let lines: Vec<&str> = self.dialog_text.lines().collect();
        let mut pos = 0;
        
        for (i, line) in lines.iter().enumerate() {
            if i == self.dialog_cursor_y {
                return pos + self.dialog_cursor_x.min(line.len());
            }
            pos += line.len() + 1; // +1 for the newline character
        }
        
        // If cursor_y is beyond the last line, position at end
        self.dialog_text.len()
    }

    /// Move the dialog cursor left by one character.
    fn move_dialog_cursor_left(&mut self) {
        if self.dialog_cursor_x > 0 {
            self.dialog_cursor_x -= 1;
        } else if self.dialog_cursor_y > 0 {
            // Move to end of previous line
            self.dialog_cursor_y -= 1;
            let lines: Vec<&str> = self.dialog_text.lines().collect();
            if let Some(prev_line) = lines.get(self.dialog_cursor_y) {
                self.dialog_cursor_x = prev_line.len();
            }
        }
    }

    /// Move the dialog cursor right by one character.
    fn move_dialog_cursor_right(&mut self) {
        let lines: Vec<&str> = self.dialog_text.lines().collect();
        if let Some(current_line) = lines.get(self.dialog_cursor_y) {
            if self.dialog_cursor_x < current_line.len() {
                self.dialog_cursor_x += 1;
            } else if self.dialog_cursor_y + 1 < lines.len() {
                // Move to beginning of next line
                self.dialog_cursor_y += 1;
                self.dialog_cursor_x = 0;
            }
        }
    }

    /// Move the dialog cursor up by one line.
    fn move_dialog_cursor_up(&mut self) {
        if self.dialog_cursor_y > 0 {
            self.dialog_cursor_y -= 1;
            let lines: Vec<&str> = self.dialog_text.lines().collect();
            if let Some(new_line) = lines.get(self.dialog_cursor_y) {
                self.dialog_cursor_x = self.dialog_cursor_x.min(new_line.len());
            }
        }
    }

    /// Move the dialog cursor down by one line.
    fn move_dialog_cursor_down(&mut self) {
        let lines: Vec<&str> = self.dialog_text.lines().collect();
        if self.dialog_cursor_y + 1 < lines.len() {
            self.dialog_cursor_y += 1;
            if let Some(new_line) = lines.get(self.dialog_cursor_y) {
                self.dialog_cursor_x = self.dialog_cursor_x.min(new_line.len());
            }
        }
    }

    /// Initialize dialog cursor position when opening a dialog.
    fn init_dialog_cursor(&mut self) {
        let lines: Vec<&str> = self.dialog_text.lines().collect();
        if lines.is_empty() {
            self.dialog_cursor_x = 0;
            self.dialog_cursor_y = 0;
        } else {
            self.dialog_cursor_y = lines.len() - 1;
            self.dialog_cursor_x = lines.last().unwrap_or(&"").len();
        }
        self.dialog_scroll_y = 0;
    }

    /// Handle keyboard input when viewing the help screen.
    /// 
    /// Returns true if the application should quit.
    /// True when a keystroke should be treated as literal text rather than a
    /// navigation command - inside a form field, an active filter, or an
    /// input prompt. Mode-switch keys and the help shortcut are suppressed
    /// in this situation.
    fn is_capturing_text(&self) -> bool {
        matches!(self.input_mode, InputMode::Text)
            || self.filter_active
            || matches!(self.overlay, Overlay::Prompt(_))
    }

    /// Handle a keystroke while an input prompt is collecting text.
    fn handle_prompt_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.overlay = Overlay::None;
            }
            KeyCode::Enter => {
                // Replace the overlay with None and consume the prompt.
                let prev = std::mem::replace(&mut self.overlay, Overlay::None);
                if let Overlay::Prompt(prompt) = prev {
                    self.complete_prompt(prompt);
                }
            }
            KeyCode::Backspace => {
                if let Overlay::Prompt(prompt) = &mut self.overlay {
                    prompt.buffer.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Overlay::Prompt(prompt) = &mut self.overlay {
                    prompt.buffer.push(c);
                }
            }
            _ => {}
        }
    }

    /// Act on a confirmed prompt.
    fn complete_prompt(&mut self, prompt: PromptState) {
        match prompt.prompt_type {
            PromptType::ArtifactPath(leaf) => {
                let raw = prompt.buffer.trim();
                if raw.is_empty() {
                    return;
                }
                let src = std::path::PathBuf::from(raw);
                let Some(entry) = self.db.state.items.get(&leaf) else {
                    self.set_status_message(format!("{leaf}: not in state.json"));
                    return;
                };
                let artifacts_dir = self.pm_dir.join(&entry.path).join("artifacts");
                if let Err(e) = std::fs::create_dir_all(&artifacts_dir) {
                    self.set_status_message(format!("artifact add: {e}"));
                    return;
                }
                let Some(file_name) = src.file_name() else {
                    self.set_status_message("artifact add: source has no file name".to_string());
                    return;
                };
                let target = artifacts_dir.join(file_name);
                match std::fs::copy(&src, &target) {
                    Ok(_) => {
                        let _ = artifacts::sweep_dir(&artifacts_dir, leaf);
                        let name = file_name.to_string_lossy().into_owned();
                        let _ = events::emit_event(
                            &self.pm_dir,
                            "artifact-add",
                            Some(leaf),
                            Some(&name),
                        );
                        self.refresh_tasks();
                        self.set_status_message(format!("Added artifact {name} to {leaf}"));
                    }
                    Err(e) => self.set_status_message(format!("artifact add failed: {e}")),
                }
            }
        }
    }

    /// Handle a keystroke while the help overlay is open. `?`, `Esc`, `h`,
    /// and `F1` close it; `Up`/`Down` scroll. Mode-switch keys are handled
    /// before this is reached, so they close help and switch in one stroke.
    fn handle_help_overlay_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => {
                self.overlay = Overlay::None;
            }
            KeyCode::Up => {
                if let Overlay::Help { scroll } = &mut self.overlay {
                    *scroll = scroll.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Overlay::Help { scroll } = &mut self.overlay {
                    *scroll = scroll.saturating_add(1);
                }
            }
            _ => {}
        }
    }

    /// Intercept the mode-switch keys. Returns `true` if the key was consumed
    /// as a mode switch.
    fn try_mode_switch(&mut self, key: KeyCode) -> bool {
        if self.is_capturing_text() {
            return false;
        }
        let target = match key {
            KeyCode::Tab => Some(self.mode.next()),
            KeyCode::BackTab => Some(self.mode.prev()),
            KeyCode::Char('1') => Some(Mode::Tickets),
            KeyCode::Char('2') => Some(Mode::Documents),
            KeyCode::Char('3') => Some(Mode::Activity),
            _ => None,
        };
        if let Some(new_mode) = target {
            if new_mode == Mode::Documents && self.documents.crumb.is_empty() {
                // First-time entry into Mode 2 seeds the breadcrumb from the
                // currently-selected ticket so the doc list and preview have
                // something to anchor to. If nothing is selected the renderer
                // shows the "pick a ticket via Mode 1" hint.
                if let Some(leaf) = self.selected_task {
                    self.documents.crumb = self.build_doc_crumb(leaf);
                    self.documents.active_level =
                        self.documents.crumb.len().saturating_sub(1);
                    self.documents.doc_cursor = 0;
                }
            }
            self.mode = new_mode;
            true
        } else {
            false
        }
    }

    /// Walk the parent chain from `leaf` up to the root, returning the chain
    /// root-first. Used to seed the Mode 2 breadcrumb from a selected ticket.
    fn build_doc_crumb(&self, leaf: LeafId) -> Vec<LeafId> {
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

    fn handle_input(&mut self) -> io::Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                self.clear_status_message();

                // An active input prompt owns every keystroke until it is
                // confirmed or cancelled.
                if matches!(self.overlay, Overlay::Prompt(_)) {
                    self.handle_prompt_input(key.code);
                    return Ok(false);
                }

                // Mode-switch keys win from any non-text-capturing surface,
                // and close any active overlay as they switch.
                if self.try_mode_switch(key.code) {
                    self.overlay = Overlay::None;
                    return Ok(false);
                }

                // The help overlay is modal: while it is open it owns input.
                if matches!(self.overlay, Overlay::Help { .. }) {
                    self.handle_help_overlay_input(key.code);
                    return Ok(false);
                }

                // `?` / `F1` open the help overlay from any mode.
                if !self.is_capturing_text()
                    && matches!(key.code, KeyCode::Char('?') | KeyCode::F(1))
                {
                    self.overlay = Overlay::Help { scroll: 0 };
                    return Ok(false);
                }

                let should_quit = match self.mode {
                    Mode::Tickets => match self.state {
                        AppState::TaskList => self.handle_task_list_input(key.code, key.modifiers)?,
                        AppState::TaskDetail => self.handle_detail_input(key.code, key.modifiers)?,
                        AppState::AddTask => self.handle_form_input(key.code, key.modifiers, false)?,
                        AppState::EditTask => self.handle_form_input(key.code, key.modifiers, true)?,
                        AppState::UserStoryDialog => self.handle_dialog_input(key.code, key.modifiers, true)?,
                        AppState::RequirementsDialog => self.handle_dialog_input(key.code, key.modifiers, false)?,
                        AppState::Confirm => self.handle_confirm_input(key.code, key.modifiers)?,
                    },
                    Mode::Documents => self.handle_documents_input(key.code, key.modifiers)?,
                    Mode::Activity => self.handle_mode_stub_input(key.code)?,
                };
                if should_quit {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Input handler for the not-yet-built Modes 2 and 3. Only `q` is live -
    /// it exits the TUI back to the launcher. Mode-switch keys are already
    /// consumed before dispatch reaches here.
    fn handle_mode_stub_input(&mut self, key: KeyCode) -> io::Result<bool> {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => Ok(true),
            _ => Ok(false),
        }
    }

    /// Render the main task list view with table and hierarchy context.
    fn render_task_list(&mut self, f: &mut Frame, area: Rect) {
        let today = Local::now().date_naive();
        let hierarchy_color = self.get_hierarchy_color();
        
        // Split the area to accommodate the ASCII header
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // ASCII header height
                Constraint::Min(0),    // Rest for the table
            ])
            .split(area);
        
        // Render ASCII header with consistent app styling and context
        let project_name = self.get_current_project_name();
        let context_display = format!("Current Project: {}  Current View: {}",
            project_name, self.navigation_context.get_display_name());
        let header_text = vec![
            Line::from(vec![
                Span::styled(
                    format!("[ {} ]", self.mode.label()),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                ),
                Span::raw("  "),
                Span::styled(
                    "PROJECT MANAGEMENT",
                    Style::default().add_modifier(Modifier::BOLD)
                ),
                Span::raw("  "),
                Span::styled(
                    context_display,
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC)
                )
            ])
        ];
        
        let header_block = Paragraph::new(header_text)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center);
        f.render_widget(header_block, chunks[0]);
    
        let header_cells = [
            "ID", "Kind", "Status", "Priority", "Urgency", "Stage", "Due", "Project", "Lock", "Title",
        ]
        .iter()
        .map(|h| {
            ratatui::widgets::Cell::from(*h).style(Style::default().add_modifier(Modifier::BOLD))
        });
    
        let text_color = match hierarchy_color {
            GOLD => Color::Rgb(20, 20, 20),
            _ => Color::White
        };
        
        let header = Row::new(header_cells)
            .style(Style::default().bg(hierarchy_color).fg(text_color))
            .height(1);
    
        // Calculate depth map for tree view
        let mut depth_map: HashMap<LeafId, usize> = HashMap::new();
        for task in &self.db.tasks {
            let mut depth = 0usize;
            let mut cur = task.parent;
            while let Some(pid) = cur {
                depth += 1;
                cur = self.db.get(pid).and_then(|p| p.parent);
                if depth > 64 {
                    break;
                } // cycle guard
            }
            depth_map.insert(task.id, depth);
        }

        // Load active locks once per render, keyed by ticket id, so each row
        // can show its lock state without a per-row directory read.
        let lock_map: HashMap<LeafId, LockFile> = locks::list(&self.pm_dir)
            .unwrap_or_default()
            .into_iter()
            .map(|lock| (lock.id, lock))
            .collect();
        let now = Utc::now();

        let rows: Vec<Row> = self
            .filtered_tasks
            .iter()
            .filter_map(|&id| self.db.get(id))
            .map(|task| {
                let due_str = format_due_relative(task.due, today);
                let project_label_str = project_label(&self.db, task);
                let project_str = if project_label_str == "-" {
                    "-".to_string()
                } else {
                    project_label_str
                };
                let tags_str = if task.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", task.tags.join(","))
                };
    
                // Determine hierarchy color
                let hierarchy_color = match task.kind {
                    Kind::Project => Color::Cyan,        // Cyan for the top-level Project tickets
                    Kind::Product => Color::Blue,        // Dark Blue (keeping existing)
                    Kind::Epic => DARK_GREEN,          // Forest Green
                    Kind::Task => GOLD,         // Gold
                    Kind::Subtask => DARK_RED,         // Crimson Red
                    Kind::Milestone => DARK_PURPLE,   // Magenta for milestones
                };
                
                let style = match task.status {
                    Status::Done => Style::default().fg(Color::DarkGray),
                    Status::InProgress => Style::default().fg(hierarchy_color).add_modifier(Modifier::BOLD),
                    _ => Style::default().fg(Color::White),
                };
    
                let depth = depth_map.get(&task.id).copied().unwrap_or(0);
                let indent_str = " ".repeat(depth);
                // M:n badge for tickets with linked memories. The count comes
                // straight from front-matter; memory file content is a Mode 2
                // concern, not loaded here.
                let memory_badge = if task.memories.is_empty() {
                    String::new()
                } else {
                    format!("  M:{}", task.memories.len())
                };
                let title_with_tags = format!("{}{}{}", task.title, tags_str, memory_badge);

                // Lock state: empty when free, STALE past the TTL window,
                // otherwise the holding agent (truncated to the column).
                let lock_cell = match lock_map.get(&task.id) {
                    None => ratatui::widgets::Cell::from(""),
                    Some(lock) if lock.is_stale(now) => ratatui::widgets::Cell::from("STALE")
                        .style(Style::default().fg(DARK_RED).add_modifier(Modifier::BOLD)),
                    Some(lock) => ratatui::widgets::Cell::from(truncate(&lock.agent, 16))
                        .style(Style::default().fg(GOLD)),
                };

                Row::new(vec![
                    ratatui::widgets::Cell::from(task.id.to_string()),
                    ratatui::widgets::Cell::from(format_kind(task.kind)),
                    ratatui::widgets::Cell::from(format_status(task.status)),
                    ratatui::widgets::Cell::from(format_priority(task.priority_level)),
                    ratatui::widgets::Cell::from(format_urgency(task.urgency)),
                    ratatui::widgets::Cell::from(format_process_stage(task.process_stage)),
                    ratatui::widgets::Cell::from(due_str),
                    ratatui::widgets::Cell::from(project_str),
                    lock_cell,
                    ratatui::widgets::Cell::from(if depth == 0 {
                        title_with_tags
                    } else {
                        format!("{}{}", indent_str, title_with_tags)
                    }),
                ])
                .style(style)
            })
            .collect();
    
        let widths = [
            Constraint::Length(4),  // ID
            Constraint::Length(10), // Kind
            Constraint::Length(12), // Status
            Constraint::Length(15), // Priority
            Constraint::Length(18), // Urgency
            Constraint::Length(13), // Stage
            Constraint::Length(12), // Due
            Constraint::Length(12), // Project
            Constraint::Length(16), // Lock
            Constraint::Min(25),    // Title
        ];
    
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(format!(
                "Tasks ({}/{}) - Press 'h' for help",
                self.filtered_tasks.len(),
                self.db.tasks.len()
            )))
            .row_highlight_style(Style::default().bg(Color::Gray).fg(Color::Black))
            .highlight_symbol(">> ");
    
        f.render_stateful_widget(table, chunks[1], &mut self.task_list_state);
    }

    /// Render the detailed view of a single task.
    fn render_task_detail(&mut self, f: &mut Frame, area: Rect) {
        if let Some(task) = self.get_selected_task() {
            let today = Local::now().date_naive();

            // Get parent and children info for navigation
            let parent_name = task
                .parent
                .and_then(|pid| self.db.get(pid).map(|p| format!("#{} - {}", p.id, p.title)));

            let child_map = build_children_map(&self.db.tasks);
            let children_names: Vec<String> = child_map
                .get(&task.id)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|&cid| self.db.get(cid))
                        .map(|c| format!("#{} - {}", c.id, c.title))
                        .collect()
                })
                .unwrap_or_default();

            let mut text = vec![
                Line::from(vec![
                    Span::styled("ID: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(task.id.to_string()),
                ]),
                Line::from(vec![
                    Span::styled("Title: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(&task.title),
                ]),
            ];

            if let Some(summary) = &task.summary {
                text.push(Line::from(vec![
                    Span::styled("Summary: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(summary),
                ]));
            }

            text.extend(vec![
                Line::from(vec![
                    Span::styled("Kind: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(format_kind(task.kind)),
                ]),
                Line::from(vec![
                    Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(format_status(task.status)),
                ]),
                Line::from(vec![
                    Span::styled("Priority: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(format_priority(task.priority_level)),
                ]),
                Line::from(vec![
                    Span::styled("Urgency: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(format_urgency(task.urgency)),
                ]),
                Line::from(vec![
                    Span::styled(
                        "Process Stage: ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format_process_stage(task.process_stage)),
                ]),
                Line::from(vec![
                    Span::styled("Project: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(project_label(&self.db, task)),
                ]),
                Line::from(vec![
                    Span::styled("Due: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(match task.due {
                        Some(d) => format!("{} ({})", d, format_due_relative(Some(d), today)),
                        None => "-".to_string(),
                    }),
                ]),
            ]);

            // Parent navigation
            if let Some(ref parent_name) = parent_name {
                text.push(Line::from(vec![
                    Span::styled("Parent: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(parent_name, Style::default().fg(Color::Blue)),
                    Span::raw(" (Press 'p' to go to parent)"),
                ]));
            } else {
                text.push(Line::from(vec![
                    Span::styled("Parent: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("-"),
                ]));
            }

            // Children navigation
            if !children_names.is_empty() {
                text.push(Line::from(vec![
                    Span::styled("Children: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("(Press 'c' to cycle through children)"),
                ]));
                for child_name in children_names.iter().take(3) {
                    // Show first 3
                    text.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(child_name, Style::default().fg(Color::Blue)),
                    ]));
                }
                if children_names.len() > 3 {
                    text.push(Line::from(vec![Span::raw(format!(
                        "  ... and {} more",
                        children_names.len() - 3
                    ))]));
                }
            } else {
                text.push(Line::from(vec![
                    Span::styled("Children: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw("-"),
                ]));
            }

            text.extend(vec![Line::from(vec![
                Span::styled("Tags: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(if task.tags.is_empty() {
                    "-".to_string()
                } else {
                    task.tags.join(", ")
                }),
            ])]);

            // Links section
            if task.issue_link.is_some() || task.pr_link.is_some() {
                text.push(Line::from(""));
                text.push(Line::from(vec![Span::styled(
                    "Links:",
                    Style::default().add_modifier(Modifier::BOLD),
                )]));

                if let Some(issue_link) = &task.issue_link {
                    text.push(Line::from(vec![
                        Span::raw("Issue: "),
                        Span::styled(issue_link, Style::default().fg(Color::Blue)),
                    ]));
                }

                if let Some(pr_link) = &task.pr_link {
                    text.push(Line::from(vec![
                        Span::raw("PR: "),
                        Span::styled(pr_link, Style::default().fg(Color::Blue)),
                    ]));
                }
            }

            text.push(Line::from(""));
            text.push(Line::from(vec![Span::styled(
                "Description:",
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            text.push(Line::from(task.description.as_deref().unwrap_or("-")));

            if let Some(user_story) = &task.user_story {
                text.push(Line::from(""));
                text.push(Line::from(vec![Span::styled(
                    "User Story:",
                    Style::default().add_modifier(Modifier::BOLD),
                )]));
                text.push(Line::from(user_story.as_str()));
            }

            let paragraph = Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Task Details - [e]dit, [d]elete, [p]arent, [c]hild, [Esc] back"),
                )
                .wrap(Wrap { trim: true });

            f.render_widget(paragraph, area);
        }
    }

    /// Render the task creation or editing form.
    fn render_task_form(&mut self, f: &mut Frame, area: Rect, is_edit: bool) {
        // Split into two columns to fit all fields
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(area);

        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(3), // Title
                    Constraint::Length(3), // Summary
                    Constraint::Length(4), // Description (taller)
                    Constraint::Length(3), // Project
                    Constraint::Length(3), // Tags
                    Constraint::Length(3), // Due Date
                    Constraint::Length(3), // Parent
                    Constraint::Length(3), // Issue Link
                    Constraint::Length(3), // PR Link
                    Constraint::Length(3), // Artifacts
                    Constraint::Length(3), // Kind
                    Constraint::Length(3), // Status
                    Constraint::Length(3), // Priority Level
                    Constraint::Length(3), // Urgency
                    Constraint::Length(3), // Process Stage
                ]
                .as_ref(),
            )
            .split(main_chunks[0]);

        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(20), // User Story
                    Constraint::Length(20), // Requirements
                    Constraint::Min(1),     // Instructions
                ]
                .as_ref(),
            )
            .split(main_chunks[1]);

        // LEFT COLUMN - Main task fields

        // Title (field 0)
        let title_style = if self.task_form.current_field == TITLE_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let title_input = Paragraph::new(self.task_form.title.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Title *")
                .border_style(title_style),
        );
        f.render_widget(title_input, left_chunks[0]);

        // Summary (field 1)
        let summary_style = if self.task_form.current_field == SUMMARY_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let summary_input = Paragraph::new(self.task_form.summary.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Summary")
                .border_style(summary_style),
        );
        f.render_widget(summary_input, left_chunks[1]);

        // Description (field 2)
        let desc_style = if self.task_form.current_field == DESCRIPTION_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let desc_input = Paragraph::new(self.task_form.description.value.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Description")
                    .border_style(desc_style),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(desc_input, left_chunks[2]);

        // Project (field 3)
        let project_style = if self.task_form.current_field == PROJECT_SELECTOR_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let selected_project = self.task_form.get_selected_project().unwrap_or("None".to_string());
        let project_selector = Paragraph::new(format!("< {} >", selected_project)).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Project")
                .border_style(project_style),
        );
        f.render_widget(project_selector, left_chunks[3]);

        // Tags (field 4)
        let tags_style = if self.task_form.current_field == TAGS_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let tags_input = Paragraph::new(self.task_form.tags.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Tags (comma-separated)")
                .border_style(tags_style),
        );
        f.render_widget(tags_input, left_chunks[4]);

        // Due Date (field 5)
        let due_style = if self.task_form.current_field == DUE_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let due_input = Paragraph::new(self.task_form.due.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Due (YYYY-MM-DD, today, tomorrow, in Nd)")
                .border_style(due_style),
        );
        f.render_widget(due_input, left_chunks[5]);

        // Parent ID (field 6)
        let parent_style = if self.task_form.current_field == PARENT_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };

        // Add parent navigation info
        let parent_title = if !self.task_form.parent.value.trim().is_empty() {
            if let Ok(parsed) = self.task_form.parent.value.trim().parse::<IdInput>() {
                let pid = parsed.leaf();
                if let Some(parent_task) = self.db.get(pid) {
                    format!("Parent ID (→ {})", parent_task.title)
                } else {
                    "Parent ID".to_string()
                }
            } else {
                "Parent ID".to_string()
            }
        } else {
            "Parent ID".to_string()
        };

        let parent_input = Paragraph::new(self.task_form.parent.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(parent_title.as_str())
                .border_style(parent_style),
        );
        f.render_widget(parent_input, left_chunks[6]);

        // Issue Link (field 7)
        let issue_style = if self.task_form.current_field == ISSUE_LINK_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let issue_input = Paragraph::new(self.task_form.issue_link.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Issue Link")
                .border_style(issue_style),
        );
        f.render_widget(issue_input, left_chunks[7]);

        // PR Link (field 8)
        let pr_style = if self.task_form.current_field == PR_LINK_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let pr_input = Paragraph::new(self.task_form.pr_link.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("PR Link")
                .border_style(pr_style),
        );
        f.render_widget(pr_input, left_chunks[8]);

        // Artifacts (field 9)
        let artifacts_style = if self.task_form.current_field == ARTIFACTS_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let artifacts_input = Paragraph::new(self.task_form.artifacts.value.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Artifacts (comma-separated)")
                .border_style(artifacts_style),
        );
        f.render_widget(artifacts_input, left_chunks[9]);

        // Kind (field 10)
        let kind_style = if self.task_form.current_field == KIND_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let kind_text = format!(
            "< {} >",
            format_kind(self.task_form.kinds[self.task_form.kind])
        );
        let kind_selector = Paragraph::new(kind_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Kind (← →)")
                    .border_style(kind_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(kind_selector, left_chunks[10]);

        // Status (field 11)
        let status_style = if self.task_form.current_field == STATUS_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let status_text = format!(
            "< {} >",
            format_status(self.task_form.statuses[self.task_form.status])
        );
        let status_selector = Paragraph::new(status_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Status (← →)")
                    .border_style(status_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(status_selector, left_chunks[11]);

        // Priority Level (field 12)
        let priority_level_style = if self.task_form.current_field == PRIORITY_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let priority_text = format!(
            "< {} >",
            format_priority(self.task_form.priorities[self.task_form.priority_level])
        );
        let priority_selector = Paragraph::new(priority_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Priority Level (← →)")
                    .border_style(priority_level_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(priority_selector, left_chunks[12]);

        // Urgency (field 13)
        let urgency_style = if self.task_form.current_field == URGENCY_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let urgency_text = format!(
            "< {} >",
            format_urgency(self.task_form.urgencies[self.task_form.urgency])
        );
        let urgency_selector = Paragraph::new(urgency_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Urgency (← →)")
                    .border_style(urgency_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(urgency_selector, left_chunks[13]);

        // Process Stage (field 14)
        let stage_style = if self.task_form.current_field == PROCESS_STAGE_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let stage_text = format!(
            "< {} >",
            format_process_stage(self.task_form.process_stages[self.task_form.process_stage])
        );
        let stage_selector = Paragraph::new(stage_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Process Stage (← →)")
                    .border_style(stage_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(stage_selector, left_chunks[14]);

        // RIGHT COLUMN - User Story, Requirements

        // User Story (field 15) - Third last, bigger
        let user_story_style = if self.task_form.current_field == USER_STORY_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let user_story_title = "User Story (Enter for fullscreen)";
        let user_story_input = Paragraph::new(self.task_form.user_story.value.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(user_story_title)
                    .border_style(user_story_style),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(user_story_input, right_chunks[0]);

        // Requirements (field 16) - Second last, bigger
        let requirements_style = if self.task_form.current_field == REQUIREMENTS_GLOBAL_ORDER {
            Style::default().fg(GOLD)
        } else {
            Style::default()
        };
        let requirements_title = "Requirements (Enter for fullscreen)";
        let requirements_input = Paragraph::new(self.task_form.requirements.value.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(requirements_title)
                    .border_style(requirements_style),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(requirements_input, right_chunks[1]);

        // Instructions at bottom of left column
        let help_text = if is_edit {
            "Tab/↑↓/jk: Navigate • ← →: Change selectors • Enter: Save/Dialog • Esc: Cancel • User Story & Requirements have fullscreen dialogs!"
        } else {
            "Tab/↑↓/jk: Navigate • ← →: Change selectors • Enter: Create/Dialog • Esc: Cancel • User Story & Requirements have fullscreen dialogs!"
        };

        let instructions = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL).title("Instructions"))
            .wrap(Wrap { trim: true });
        f.render_widget(instructions, right_chunks[2]);

        // Render cursor for active text fields
        let cursor_field = match self.task_form.current_field {
            TITLE_GLOBAL_ORDER => Some((left_chunks[0], &self.task_form.title)),
            SUMMARY_GLOBAL_ORDER => Some((left_chunks[1], &self.task_form.summary)),
            DESCRIPTION_GLOBAL_ORDER => Some((left_chunks[2], &self.task_form.description)),
            PROJECT_SELECTOR_GLOBAL_ORDER => None, // Project selector doesn't need cursor
            TAGS_GLOBAL_ORDER => Some((left_chunks[4], &self.task_form.tags)),
            DUE_GLOBAL_ORDER => Some((left_chunks[5], &self.task_form.due)),
            PARENT_GLOBAL_ORDER => Some((left_chunks[6], &self.task_form.parent)),
            ISSUE_LINK_GLOBAL_ORDER => Some((left_chunks[7], &self.task_form.issue_link)),
            PR_LINK_GLOBAL_ORDER => Some((left_chunks[8], &self.task_form.pr_link)),
            ARTIFACTS_GLOBAL_ORDER => Some((left_chunks[9], &self.task_form.artifacts)),
            /*  Skips 5x non-cursor fields here */
            USER_STORY_GLOBAL_ORDER => Some((right_chunks[0], &self.task_form.user_story)),
            REQUIREMENTS_GLOBAL_ORDER => Some((right_chunks[1], &self.task_form.requirements)),
            _ => None,
        };

        if let Some((chunk, field)) = cursor_field {
            f.set_cursor_position((chunk.x + field.cursor as u16 + 1, chunk.y + 1));
        }
    }

    /// Render the help screen with keyboard shortcuts and usage instructions.
    /// Render the modal help overlay. Mode-independent: drawn over whatever
    /// the current mode produced. Scrollable with Up/Down; closed with `?`,
    /// `Esc`, `h`, or `F1`. Layout follows PM_DESIGN.md Section 8.3.5 -
    /// current-mode keybindings first, then a concepts panel, then workflows.
    fn render_help(&mut self, f: &mut Frame, area: Rect) {
        let heading = |text: &str| {
            Line::from(vec![Span::styled(
                text.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )])
        };

        let mut lines: Vec<Line> = Vec::new();
        lines.push(heading(&format!("{} - keybindings", self.mode.label())));
        match self.mode {
            Mode::Tickets => {
                lines.push(Line::from("  <- ->        Traverse hierarchy levels"));
                lines.push(Line::from("  ^ v          Move within the list"));
                lines.push(Line::from("  Enter        Drill into the selected ticket"));
                lines.push(Line::from("  e            Open the ticket's CLAUDE.md in $EDITOR"));
                lines.push(Line::from("  f            Open the quick-entry form"));
                lines.push(Line::from("  n            Add a child ticket"));
                lines.push(Line::from("  c / i        Checkout / checkin the selected ticket"));
                lines.push(Line::from("  a            Add an artifact"));
                lines.push(Line::from("  m            Toggle the memory side-panel"));
                lines.push(Line::from("  d            Delete the selected ticket"));
                lines.push(Line::from("  s            Cycle status   p   cycle process stage"));
                lines.push(Line::from("  t            Toggle show/hide completed   r refresh"));
                lines.push(Line::from("  /            Filter by title / tags / project"));
            }
            Mode::Documents => {
                lines.push(Line::from("  Document Workspace arrives in Phase 8."));
                lines.push(Line::from("  q            Exit to the launcher"));
            }
            Mode::Activity => {
                lines.push(Line::from("  Activity View arrives in Phase 9."));
                lines.push(Line::from("  q            Exit to the launcher"));
            }
        }
        lines.push(Line::from("  Tab / S-Tab  Cycle modes      1 / 2 / 3  jump to a mode"));
        lines.push(Line::from("  ? / F1       Toggle this help   q  back to launcher"));
        lines.push(Line::from(""));

        lines.push(heading("Concepts"));
        lines.push(Line::from("  Hierarchy    PRJ Project > PRD Product > EPC Epic > TSK Task > SBT Subtask"));
        lines.push(Line::from("  MLS          Milestone - a cross-cutting marker, project-scoped by default"));
        lines.push(Line::from("  Locks        A checkout claims a ticket; the Lock column shows the holder,"));
        lines.push(Line::from("               or STALE once the heartbeat TTL has passed"));
        lines.push(Line::from("  Memories     Three tiers - user, project, ticket. M:n counts linked refs"));
        lines.push(Line::from("  Composition  A ticket's CLAUDE.md carries front-matter plus prose sections"));
        lines.push(Line::from("  Git          Every state change commits; checkin squashes the checkout span"));
        lines.push(Line::from(""));

        lines.push(heading("Workflows"));
        lines.push(Line::from("  File a task            n, fill the quick-entry form, save"));
        lines.push(Line::from("  Hand off to an agent   c to checkout, share the ticket id, i to checkin"));
        lines.push(Line::from("  Monitor parallel work  Mode 3 (Phase 9) or `pm tv` on a second screen"));
        lines.push(Line::from("  Write a project memory `pm memory write --scope project ...` (Phase 10)"));

        let overlay = centered_rect(80, 80, area);
        f.render_widget(Clear, overlay);
        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Help - ^v scroll, ? or Esc to close"),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.help_scroll(), 0));
        f.render_widget(paragraph, overlay);
    }

    /// Current help-overlay scroll offset. Returns 0 when the overlay is not
    /// the help variant; callers should only invoke `render_help` while it is.
    fn help_scroll(&self) -> u16 {
        match self.overlay {
            Overlay::Help { scroll } => scroll,
            _ => 0,
        }
    }

    /// Render a fullscreen text editing dialog for user stories or requirements.
    fn render_dialog(&mut self, f: &mut Frame, area: Rect, title: &str) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)].as_ref())
            .split(area);

        // Main text area
        let block = Block::default()
            .title(format!("{} - Fullscreen Editor", title))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White).bg(Color::Blue));

        let inner = block.inner(chunks[0]);
        f.render_widget(block, chunks[0]);

        // Split text into lines and handle scrolling
        let lines: Vec<&str> = self.dialog_text.lines().collect();
        let visible_height = inner.height as usize;
        
        // Adjust scroll to keep cursor visible
        if self.dialog_cursor_y >= self.dialog_scroll_y + visible_height {
            self.dialog_scroll_y = self.dialog_cursor_y.saturating_sub(visible_height - 1);
        } else if self.dialog_cursor_y < self.dialog_scroll_y {
            self.dialog_scroll_y = self.dialog_cursor_y;
        }

        // Get visible lines based on scroll position
        let visible_lines: Vec<Line> = lines
            .iter()
            .skip(self.dialog_scroll_y)
            .take(visible_height)
            .map(|&line| Line::from(line))
            .collect();

        let paragraph = Paragraph::new(visible_lines);
        f.render_widget(paragraph, inner);

        // Instructions with improved text
        let instructions = Paragraph::new(
            "Arrow keys to navigate • Type to edit • Enter for new line • Backspace/Delete • Ctrl+Backspace/Delete to clear all • Home/End • Esc to save and return",
        )
        .block(Block::default().borders(Borders::ALL).title("Instructions"))
        .alignment(Alignment::Center);
        f.render_widget(instructions, chunks[1]);

        // Calculate cursor position relative to visible area
        let cursor_y_visible = self.dialog_cursor_y.saturating_sub(self.dialog_scroll_y);
        let cursor_x_clamped = self.dialog_cursor_x.min(inner.width as usize);
        
        // Only show cursor if it's in the visible area
        if cursor_y_visible < visible_height {
            f.set_cursor_position((
                inner.x + cursor_x_clamped as u16,
                inner.y + cursor_y_visible as u16
            ));
        }
    }

    /// Render a confirmation dialog for destructive actions.
    fn render_confirm(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Confirm Action")
            .borders(Borders::ALL)
            .style(Style::default().bg(DARK_RED));

        let area = centered_rect(50, 20, area);
        f.render_widget(Clear, area);

        let text = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Are you sure you want to:",
                Style::default().add_modifier(Modifier::BOLD),
            )]),
            Line::from(self.confirm_action.as_deref().unwrap_or("")),
            Line::from(""),
            Line::from("This action cannot be undone."),
            Line::from(""),
            Line::from("Press 'y' to confirm, 'n' to cancel"),
        ];

        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    }

    /// Render the context-sensitive help row at the bottom of the screen.
    fn render_status_bar(&mut self, f: &mut Frame, area: Rect) {
        let status_text = if !self.status_message.is_empty() {
            self.status_message.clone()
        } else if matches!(self.overlay, Overlay::Help { .. }) {
            "Help: ^v scroll   ? / Esc close   Tab / 1 / 2 / 3 switch mode".to_string()
        } else if self.filter_active {
            format!(
                "Search: {} (Esc to clear, Enter to confirm)",
                self.filter_text
            )
        } else if !self.filter_text.is_empty() {
            format!(
                "Tasks: {} (filtered by '{}') | ? for help",
                self.filtered_tasks.len(),
                self.filter_text
            )
        } else {
            match self.mode {
                Mode::Documents | Mode::Activity => {
                    format!("{}   Tab / 1 / 2 / 3 switch mode   ? help   q exit", self.mode.label())
                }
                Mode::Tickets => match self.state {
                    AppState::TaskList => {
                        let back_tip = if self.has_navigation_history() {
                            " | Alt+<- Back"
                        } else {
                            ""
                        };
                        format!("Tasks: {} | ? for help | Tab / 1 / 2 / 3 mode{}",
                            self.filtered_tasks.len(), back_tip)
                    }
                    AppState::TaskDetail => "Task Details".to_string(),
                    AppState::AddTask => "Add New Task".to_string(),
                    AppState::EditTask => "Edit Task".to_string(),
                    AppState::UserStoryDialog => {
                        "User Story - Fullscreen Editor (Esc to save & return)".to_string()
                    }
                    AppState::RequirementsDialog => {
                        "Requirements - Fullscreen Editor (Esc to save & return)".to_string()
                    }
                    AppState::Confirm => "Confirm Action".to_string(),
                },
            }
        };

        let hierarchy_color = self.get_hierarchy_color();
        let text_color = match hierarchy_color {
            GOLD => Color::Rgb(20, 20, 20),
            _ => Color::White
        };
        let status = Paragraph::new(status_text)
            .style(Style::default().bg(hierarchy_color).fg(text_color))
            .alignment(Alignment::Left);

        f.render_widget(status, area);
    }

    /// Main render function that dispatches to appropriate view renderers.
    fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),    // mode content
                Constraint::Length(5), // activity footer (bordered, 3 events)
                Constraint::Length(1), // context-sensitive help row
            ].as_ref())
            .split(f.area());

        match self.mode {
            Mode::Tickets => {
                match self.state {
                    AppState::TaskList => self.render_task_list(f, chunks[0]),
                    AppState::TaskDetail => self.render_task_detail(f, chunks[0]),
                    AppState::AddTask => self.render_task_form(f, chunks[0], false),
                    AppState::EditTask => self.render_task_form(f, chunks[0], true),
                    AppState::UserStoryDialog => self.render_dialog(f, chunks[0], "User Story"),
                    AppState::RequirementsDialog => self.render_dialog(f, chunks[0], "Requirements"),
                    AppState::Confirm => {
                        self.render_task_list(f, chunks[0]);
                        self.render_confirm(f, chunks[0]);
                    }
                }
                // The memory side-panel overlays the right edge of the list.
                if matches!(self.overlay, Overlay::MemoryPanel) && self.state == AppState::TaskList {
                    self.render_memory_panel(f, chunks[0]);
                }
            }
            Mode::Documents => self.render_documents(f, chunks[0]),
            Mode::Activity => self.render_mode_stub(f, chunks[0], "Activity View", "Phase 9"),
        }

        self.render_activity_footer(f, chunks[1]);
        self.render_status_bar(f, chunks[2]);

        // An active input prompt overlays the current mode.
        if let Overlay::Prompt(prompt) = &self.overlay {
            let label = match prompt.prompt_type {
                PromptType::ArtifactPath(_) => {
                    "Add artifact - path to file (Enter to add, Esc to cancel)"
                }
            };
            let area = centered_rect(70, 20, f.area());
            f.render_widget(Clear, area);
            let widget = Paragraph::new(prompt.buffer.as_str())
                .block(Block::default().borders(Borders::ALL).title(label));
            f.render_widget(widget, area);
        }

        // The help overlay is modal and mode-independent: drawn last so it
        // sits on top of whatever the current mode rendered.
        if matches!(self.overlay, Overlay::Help { .. }) {
            self.render_help(f, f.area());
        }
    }

    /// Render the activity footer - the last three entries from `events.log`
    /// in a bordered block. Shown beneath every mode (PM_DESIGN.md 8.3.1).
    fn render_activity_footer(&mut self, f: &mut Frame, area: Rect) {
        let all = events::read_events(&self.pm_dir).unwrap_or_default();
        // Take the last three, then re-order oldest-first for display.
        let mut tail: Vec<&events::Event> = all.iter().rev().take(3).collect();
        tail.reverse();

        let mut lines: Vec<Line> = Vec::new();
        for ev in tail {
            let time = ev.ts.format("%H:%M:%S");
            let id = ev.id.map(|i| i.to_string()).unwrap_or_else(|| "-".to_string());
            let detail = ev
                .detail
                .as_deref()
                .map(|d| format!("  \"{d}\""))
                .unwrap_or_default();
            lines.push(Line::from(format!(
                "  {time}  {:<18}  {:<10}  {id}{detail}",
                truncate(&ev.actor, 18),
                ev.verb
            )));
        }
        if lines.is_empty() {
            lines.push(Line::from("  (no activity yet)"));
        }

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Activity"));
        f.render_widget(widget, area);
    }

    /// Render the memory side-panel over the right edge of the task list.
    /// Lists the selected ticket's linked memories with their tier; full
    /// memory content is a Mode 2 concern (Phase 8 / 10).
    fn render_memory_panel(&mut self, f: &mut Frame, area: Rect) {
        let width = (area.width / 3).max(20).min(area.width);
        let panel = Rect {
            x: area.x + area.width.saturating_sub(width),
            y: area.y,
            width,
            height: area.height,
        };
        f.render_widget(Clear, panel);

        let mut lines: Vec<Line> = Vec::new();
        match self.get_selected_task() {
            Some(task) if !task.memories.is_empty() => {
                for memory in &task.memories {
                    let (tier, name) = match memory {
                        MemoryRef::User(name) => ("user", name),
                        MemoryRef::Project(name) => ("project", name),
                        MemoryRef::Ticket(name) => ("ticket", name),
                    };
                    lines.push(Line::from(format!("  @{name}  [{tier}]")));
                }
            }
            Some(_) => lines.push(Line::from("  (no linked memories)")),
            None => lines.push(Line::from("  (no ticket selected)")),
        }

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Memories  (m to close)"))
            .wrap(Wrap { trim: false });
        f.render_widget(widget, panel);
    }

    /// Input dispatch for Mode 2 - the Document Workspace. Mode-switch keys
    /// and `?` / `F1` for help are already consumed before this is reached.
    /// Later commits in Phase 8 layer on $EDITOR shell-out, the artifact /
    /// memory / rename modals, and Left / Right breadcrumb navigation.
    fn handle_documents_input(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> io::Result<bool> {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Up => self.documents_cursor_move(-1),
            KeyCode::Down => self.documents_cursor_move(1),
            _ => {}
        }
        Ok(false)
    }

    /// Move the LHS cursor by `delta` rows, snapping past header rows.
    fn documents_cursor_move(&mut self, delta: isize) {
        let items = self.documents_pane_items();
        if items.is_empty() {
            return;
        }
        let len = items.len();
        let mut idx = self.documents.doc_cursor as isize + delta;
        // Clamp first so the snap-past-headers loop has a real anchor.
        idx = idx.clamp(0, (len - 1) as isize);
        let step: isize = if delta >= 0 { 1 } else { -1 };
        while idx >= 0 && (idx as usize) < len && items[idx as usize].is_header() {
            idx += step;
        }
        if idx < 0 || idx as usize >= len {
            // Fell off the end while skipping headers; reverse the snap.
            let mut back = self.documents.doc_cursor as isize;
            while back >= 0 && (back as usize) < len && items[back as usize].is_header() {
                back += if step > 0 { -1 } else { 1 };
            }
            if back >= 0 && (back as usize) < len {
                self.documents.doc_cursor = back as usize;
            }
            return;
        }
        self.documents.doc_cursor = idx as usize;
    }

    /// Render Mode 2 - the Document Workspace. The breadcrumb at the top
    /// shows the address chain for the focused ticket; the body below is a
    /// two-pane split with the doc list on the left and the preview on the
    /// right.
    fn render_documents(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Breadcrumb header
                Constraint::Min(0),    // Two-pane body
            ])
            .split(area);

        let breadcrumb = self.documents_breadcrumb_line();
        let header = Paragraph::new(breadcrumb)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("[ {} ]", self.mode.label())),
            )
            .alignment(Alignment::Left);
        f.render_widget(header, chunks[0]);

        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[1]);

        if self.documents.crumb.is_empty() {
            // Without a focused ticket the body has nothing to render. Tell
            // the user how to get one rather than show an empty pair of
            // boxes that look broken.
            let lhs = Paragraph::new(Line::from(
                "No ticket focused. Switch to Mode 1 (key `1`) and pick one.",
            ))
            .block(Block::default().borders(Borders::ALL).title("Documents"))
            .wrap(Wrap { trim: true });
            f.render_widget(lhs, panes[0]);

            let rhs = Paragraph::new("")
                .block(Block::default().borders(Borders::ALL).title("Preview"));
            f.render_widget(rhs, panes[1]);
            return;
        }

        let items = self.documents_pane_items();
        // Clamp the cursor in case the focused ticket changed and the list
        // shrank since the last render.
        if self.documents.doc_cursor >= items.len() {
            self.documents.doc_cursor = items.len().saturating_sub(1);
        }
        // If the cursor landed on a header after a clamp, push it past.
        if !items.is_empty() && items[self.documents.doc_cursor].is_header() {
            let mut idx = self.documents.doc_cursor;
            while idx + 1 < items.len() && items[idx].is_header() {
                idx += 1;
            }
            self.documents.doc_cursor = idx;
        }

        let lhs_lines: Vec<Line> = items
            .iter()
            .enumerate()
            .map(|(i, item)| item.to_line(i == self.documents.doc_cursor))
            .collect();
        let lhs = Paragraph::new(lhs_lines)
            .block(Block::default().borders(Borders::ALL).title("Documents"))
            .wrap(Wrap { trim: false });
        f.render_widget(lhs, panes[0]);

        let preview = items
            .get(self.documents.doc_cursor)
            .map(|item| self.documents_preview(item))
            .unwrap_or_default();
        let rhs = Paragraph::new(preview)
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .wrap(Wrap { trim: false });
        f.render_widget(rhs, panes[1]);
    }

    /// Compose the flat LHS list for the focused ticket. The list is built
    /// fresh each render from the on-disk state, so artifact additions or
    /// section renames surface without an explicit refresh.
    fn documents_pane_items(&self) -> Vec<DocsPaneItem> {
        let mut out: Vec<DocsPaneItem> = Vec::new();
        let Some(&focus) = self.documents.crumb.last() else {
            return out;
        };
        let address = match crate::store::AddressId::new(self.documents.crumb.clone()) {
            Ok(a) => a,
            Err(_) => return out,
        };
        let layout = crate::store::Layout::at(&self.pm_dir);
        let rel = layout.directory_for(&address);
        let abs = layout.root.join(&rel);

        let claude_md = abs.join("CLAUDE.md");
        let ticket = if claude_md.exists() {
            crate::store::Ticket::read(&claude_md).ok()
        } else {
            None
        };

        out.push(DocsPaneItem::header("Documents"));
        if claude_md.exists() {
            out.push(DocsPaneItem::doc("CLAUDE.md", claude_md));
        }
        let artifacts_dir = abs.join("artifacts");
        if let Ok(read_dir) = std::fs::read_dir(&artifacts_dir) {
            let mut artifact_files: Vec<std::path::PathBuf> = read_dir
                .filter_map(|d| d.ok())
                .filter(|d| d.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|d| d.path())
                .collect();
            artifact_files.sort();
            for path in artifact_files {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| format!("artifacts/{n}"))
                    .unwrap_or_else(|| path.display().to_string());
                out.push(DocsPaneItem::doc(name, path));
            }
        }

        out.push(DocsPaneItem::header("Memories (linked)"));
        if let Some(t) = &ticket {
            if t.front_matter.memories.is_empty() {
                out.push(DocsPaneItem::note("  (none)"));
            } else {
                for memref in &t.front_matter.memories {
                    out.push(DocsPaneItem::memory(memref.clone()));
                }
            }
        } else {
            out.push(DocsPaneItem::note("  (CLAUDE.md missing)"));
        }

        out.push(DocsPaneItem::header("Section quick-edit"));
        if let Some(t) = &ticket {
            if t.body.sections.is_empty() {
                out.push(DocsPaneItem::note("  (no sections)"));
            } else {
                for section in &t.body.sections {
                    out.push(DocsPaneItem::section(section.name.clone()));
                }
            }
        }

        let _ = focus; // reserved for later commits that scope to the focus level.
        out
    }

    /// Build the right-hand preview text for the highlighted LHS item.
    fn documents_preview(&self, item: &DocsPaneItem) -> String {
        match item {
            DocsPaneItem::Header(_) | DocsPaneItem::Note(_) => String::new(),
            DocsPaneItem::Doc { path, .. } => documents_preview_file(path),
            DocsPaneItem::Memory(reference) => documents_preview_memory(reference),
            DocsPaneItem::Section { name } => self.documents_preview_section(name),
        }
    }

    /// Pull the body of the named section from the focused ticket's
    /// CLAUDE.md. Returns an empty string if the ticket can't be loaded.
    fn documents_preview_section(&self, name: &str) -> String {
        let Some(_) = self.documents.crumb.last() else {
            return String::new();
        };
        let Ok(address) = crate::store::AddressId::new(self.documents.crumb.clone()) else {
            return String::new();
        };
        let layout = crate::store::Layout::at(&self.pm_dir);
        let abs = layout.root.join(layout.directory_for(&address));
        let claude_md = abs.join("CLAUDE.md");
        match crate::store::Ticket::read(&claude_md) {
            Ok(t) => t
                .body
                .find(name)
                .map(|s| {
                    let mut out = format!("# {}\n\n", s.name);
                    out.push_str(&s.body);
                    out
                })
                .unwrap_or_else(|| format!("# {name}\n\n(section missing)")),
            Err(_) => String::new(),
        }
    }

    /// Compose the single-line breadcrumb shown in the Mode 2 header. Each
    /// segment names the type and leaf id of one level in the focused
    /// ticket's parent chain; the active level is highlighted.
    fn documents_breadcrumb_line(&self) -> Line<'_> {
        if self.documents.crumb.is_empty() {
            return Line::from(Span::styled(
                "(no ticket focused)",
                Style::default().fg(Color::DarkGray),
            ));
        }
        let mut spans: Vec<Span> = Vec::with_capacity(self.documents.crumb.len() * 2);
        for (idx, leaf) in self.documents.crumb.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::raw(" > "));
            }
            // The leaf's Display already prints the typed prefix, so the
            // segment reads e.g. "TSK7" without further dressing. Later
            // commits will append the ticket's title once it's loaded.
            let label = leaf.to_string();
            // The active segment is the one the cursor sits on in the
            // breadcrumb; the rest are rendered plainly.
            if idx == self.documents.active_level {
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(label));
            }
        }
        Line::from(spans)
    }

    /// Placeholder screen for Mode 3 until Phase 9 builds it out. Shows which
    /// mode is selected and how to switch away.
    fn render_mode_stub(&mut self, f: &mut Frame, area: Rect, title: &str, arrives_in: &str) {
        let body = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                self.mode.label(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(format!("{title} arrives in {arrives_in}.")),
            Line::from(""),
            Line::from("Tab / Shift+Tab / 1 / 2 / 3 to switch modes      q to exit"),
        ];
        let widget = Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("[ {} ]", self.mode.label())),
            )
            .alignment(Alignment::Center);
        f.render_widget(widget, area);
    }

    /// Main event loop for the TUI application.
    /// 
    /// Handles rendering and input processing until the user exits.
    pub fn run<B: Backend + std::io::Write>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if self.handle_input()? {
                break;
            }

            // Deferred terminal-suspending work runs here, outside the draw
            // and input phases, so the editor handoff has the terminal to
            // itself.
            if let Some(action) = self.pending_action.take() {
                self.run_pending_action(terminal, action)?;
            }
        }
        Ok(())
    }

    /// Run a deferred action that needs the terminal suspended. Currently the
    /// only case is the `$EDITOR` handoff for editing a ticket's CLAUDE.md.
    fn run_pending_action<B: Backend + std::io::Write>(
        &mut self,
        terminal: &mut Terminal<B>,
        action: PendingAction,
    ) -> io::Result<()> {
        match action {
            PendingAction::EditTicket(leaf) => {
                let claude_path = match self.db.state.items.get(&leaf) {
                    Some(entry) => self.pm_dir.join(&entry.path).join("CLAUDE.md"),
                    None => {
                        self.set_status_message(format!("{leaf}: not in state.json"));
                        return Ok(());
                    }
                };

                // Hand the terminal to the editor, then take it back.
                disable_raw_mode()?;
                execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                let status = std::process::Command::new(&editor).arg(&claude_path).status();
                enable_raw_mode()?;
                execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
                terminal.clear()?;

                match status {
                    Ok(_) => {
                        self.refresh_tasks();
                        let _ = events::emit_event(&self.pm_dir, "edit", Some(leaf), None);
                        self.set_status_message(format!("{leaf}: edited in $EDITOR"));
                    }
                    Err(e) => self.set_status_message(format!("editor failed: {e}")),
                }
            }
        }
        Ok(())
    }
}

/// One row in the LHS list of Mode 2. Renderer-private; the variants drive
/// both the displayed line and the preview content.
enum DocsPaneItem {
    /// A category label (Documents / Memories / Sections). Not selectable.
    Header(String),
    /// A free-form note in place of a list (e.g. "(none)"). Selectable but
    /// has no preview content.
    Note(String),
    /// A file inside the focused ticket's directory.
    Doc { label: String, path: std::path::PathBuf },
    /// A linked memory reference from the ticket's front-matter.
    Memory(MemoryRef),
    /// A section heading found in the ticket's CLAUDE.md body.
    Section { name: String },
}

impl DocsPaneItem {
    fn header(s: impl Into<String>) -> Self { DocsPaneItem::Header(s.into()) }
    fn note(s: impl Into<String>) -> Self { DocsPaneItem::Note(s.into()) }
    fn doc(label: impl Into<String>, path: std::path::PathBuf) -> Self {
        DocsPaneItem::Doc { label: label.into(), path }
    }
    fn memory(reference: MemoryRef) -> Self { DocsPaneItem::Memory(reference) }
    fn section(name: String) -> Self { DocsPaneItem::Section { name } }

    fn is_header(&self) -> bool {
        matches!(self, DocsPaneItem::Header(_))
    }

    fn to_line(&self, selected: bool) -> Line<'static> {
        let (text, style) = match self {
            DocsPaneItem::Header(s) => (
                s.clone(),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            DocsPaneItem::Note(s) => (s.clone(), Style::default().fg(Color::DarkGray)),
            DocsPaneItem::Doc { label, .. } => (format!("  {label}"), Style::default()),
            DocsPaneItem::Memory(reference) => {
                let label = match reference {
                    MemoryRef::User(name) => format!("  @{name}  [user]"),
                    MemoryRef::Project(name) => format!("  @{name}  [project]"),
                    MemoryRef::Ticket(name) => format!("  @{name}  [ticket]"),
                };
                (label, Style::default())
            }
            DocsPaneItem::Section { name } => (format!("  {name}"), Style::default()),
        };
        let mut span_style = style;
        if selected && !self.is_header() {
            span_style = span_style.add_modifier(Modifier::REVERSED);
        }
        Line::from(Span::styled(text, span_style))
    }
}

/// Read the first ~4 KiB of a file as a preview. Binary content shows a
/// short note rather than a wall of unprintable bytes.
fn documents_preview_file(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut buf = Vec::with_capacity(4096);
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return format!("(read failed: {e})"),
    };
    let _ = std::io::Read::take(&mut file, 4096).read_to_end(&mut buf);
    match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        Err(_) => format!(
            "(binary file, {} bytes preview suppressed)",
            buf.len(),
        ),
    }
}

/// A short description of a memory reference. The actual content read is a
/// later commit's concern; Mode 2 here only needs to show what the entry is.
fn documents_preview_memory(reference: &MemoryRef) -> String {
    match reference {
        MemoryRef::User(name) => format!("user-scope memory: {name}\n\n@{name}"),
        MemoryRef::Project(name) => format!("project-scope memory: {name}\n\n@{name}"),
        MemoryRef::Ticket(name) => format!("ticket-scope memory: {name}\n\n@{name}"),
    }
}
