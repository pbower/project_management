//! Workflow Kanban board interface.
//!
//! This module implements a Kanban-style board view where tasks are organized
//! into columns by process stage, allowing for visual task management and
//! rapid status updates through drag-and-drop style interactions.

use std::io;
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::{fields::*, tui::colors::{DARK_GREEN, DARK_PURPLE, DARK_RED, GOLD}};
use crate::task::Task;
use crate::{
    db::{Database, format_status},
    tui::enums::{HierarchyLevel, NavigationContext},
};


/// Return value for workflow app to indicate what should happen next
#[derive(Debug)]
pub enum WorkflowExit {
    Quit,
    EditTask(u64),
}

/// Main workflow application state
pub struct WorkflowApp {
    db: Database,
    db_path: std::path::PathBuf,
    navigation_context: NavigationContext,
    navigation_stack: Vec<NavigationContext>,  // For drill-down/up navigation
    selected_column: usize,  // Current process stage column (0-8)
    selected_card: usize,    // Selected card within the column
    column_scroll_offsets: [usize; 9],  // Scroll offset for each column
    status_message: String,
    show_task_detail: bool,  // Whether to show task detail popup
    show_completed: bool,    // Whether to show completed tasks
    edit_task_id: Option<u64>,  // Task ID to edit when exiting
    filter_active: bool,     // Whether filter mode is active
    filter_text: String,     // Current filter text
    
    // Organized tasks by process stage
    columns: [Vec<u64>; 9], // 9 process stages: None, Ideation, Design, Prototyping, Ready to Implement, Implementation, Testing, Refinement, Release
}

impl WorkflowApp {
    /// Create a new WorkflowApp instance
    pub fn new(db_path: &Path) -> io::Result<Self> {
        let db = Database::load(db_path);
        
        let mut app = WorkflowApp {
            db,
            db_path: db_path.to_path_buf(),
            navigation_context: NavigationContext::new_all_level(HierarchyLevel::Product), // Default to products view
            navigation_stack: Vec::new(),
            selected_column: 0,
            selected_card: 0,
            column_scroll_offsets: [0; 9],
            status_message: String::new(),
            show_task_detail: false,
            show_completed: false,  // Hide completed tasks by default
            edit_task_id: None,
            filter_active: false,
            filter_text: String::new(),
            columns: Default::default(),
        };
        
        app.update_columns();
        Ok(app)
    }

    /// Get the theme color for the current hierarchy level
    fn get_hierarchy_color(&self) -> Color {
        match self.navigation_context.level {
            HierarchyLevel::Product => Color::Blue,        // Navy
            HierarchyLevel::Epic => DARK_GREEN,           // Green
            HierarchyLevel::Task => GOLD,                 // Gold
            HierarchyLevel::Subtask => DARK_RED,          // Red
            HierarchyLevel::Milestone => DARK_PURPLE,     // Purple (shouldn't appear in workflow but just in case)
        }
    }

    /// Get the current project name from the database path
    fn get_current_project_name(&self) -> String {
        use crate::project::Project;
        
        if let Some(project) = Project::from_file(self.db_path.clone()) {
            project.display_name
        } else {
            "Default (Legacy)".to_string()
        }
    }

    /// Update the task columns based on current context and filters
    fn update_columns(&mut self) {
        // Clear all columns and reset scroll offsets
        for (i, column) in self.columns.iter_mut().enumerate() {
            column.clear();
            self.column_scroll_offsets[i] = 0;
        }

        let hierarchy_level = self.navigation_context.level;
        let parent_filter = self.navigation_context.parent_id;

        // Filter tasks based on context
        for task in &self.db.tasks {
            // Filter out completed tasks unless show_completed is true
            if task.status == Status::Done && !self.show_completed {
                continue;
            }
            
            // Filter by hierarchy level
            let required_kind = match hierarchy_level {
                HierarchyLevel::Product => Kind::Product,
                HierarchyLevel::Epic => Kind::Epic,
                HierarchyLevel::Task => Kind::Task,
                HierarchyLevel::Subtask => Kind::Subtask,
                HierarchyLevel::Milestone => continue, // Skip milestones in workflow view
            };
            
            if task.kind != required_kind {
                continue;
            }

            // Filter by parent if in filtered mode
            if let Some(parent_id) = parent_filter {
                if task.parent != Some(parent_id) {
                    continue;
                }
            }
            
            // Apply text filter if active
            if !self.filter_text.is_empty() {
                let filter_lower = self.filter_text.to_lowercase();
                let title_matches = task.title.to_lowercase().contains(&filter_lower);
                let tags_match = task.tags.iter().any(|tag| tag.to_lowercase().contains(&filter_lower));
                let project_matches = task.project.as_ref()
                    .map_or(false, |p| p.to_lowercase().contains(&filter_lower));
                
                if !title_matches && !tags_match && !project_matches {
                    continue;
                }
            }

            // Organize into columns by process stage
            let column_index = match task.process_stage {
                None => 0,
                Some(ProcessStage::Ideation) => 1,
                Some(ProcessStage::Design) => 2,
                Some(ProcessStage::Prototyping) => 3,
                Some(ProcessStage::ReadyToImplement) => 4,
                Some(ProcessStage::Implementation) => 5,
                Some(ProcessStage::Testing) => 6,
                Some(ProcessStage::Refinement) => 7,
                Some(ProcessStage::Release) => 8, // Release is now its own column
            };

            self.columns[column_index].push(task.id);
        }

        // Ensure selected card is valid
        self.clamp_selection();
    }

    /// Ensure selected column and card indices are valid
    fn clamp_selection(&mut self) {
        if self.selected_column >= self.columns.len() {
            self.selected_column = 0;
        }

        let column_len = self.columns[self.selected_column].len();
        if column_len == 0 {
            self.selected_card = 0;
            self.column_scroll_offsets[self.selected_column] = 0;
        } else if self.selected_card >= column_len {
            self.selected_card = column_len - 1;
        }
        
        self.update_scroll_for_selection();
    }
    
    /// Update scroll offset to ensure selected card is visible
    fn update_scroll_for_selection(&mut self) {
        // This will be called when navigating to ensure the selected card is visible
        // The actual calculation happens in render_column where we know the available height
    }
    
    /// Toggle completion status of the selected task
    fn toggle_task_completion(&mut self) {
        if self.columns[self.selected_column].is_empty() {
            return;
        }
        
        let task_id = self.columns[self.selected_column][self.selected_card];
        
        if let Some(task) = self.db.get_mut(task_id) {
            // Toggle between Done and Open (or InProgress if it was InProgress)
            let new_status = if task.status == Status::Done {
                // Uncomplete: restore to Open or InProgress based on process stage
                if task.process_stage == Some(ProcessStage::Implementation) 
                    || task.process_stage == Some(ProcessStage::Testing) {
                    Status::InProgress
                } else {
                    Status::Open
                }
            } else {
                Status::Done
            };
            
            task.status = new_status;
            
            if let Err(e) = self.save_db() {
                self.set_status_message(format!("Error saving: {}", e));
            } else {
                let status_text = match new_status {
                    Status::Done => "Task marked as completed",
                    Status::InProgress => "Task marked as in progress",
                    Status::Open => "Task marked as open",
                };
                self.set_status_message(status_text.to_string());
                
                // If we just completed a task and we're hiding completed, it will disappear
                if new_status == Status::Done && !self.show_completed {
                    self.update_columns();
                }
            }
        }
    }

    /// Save the database to disk and refresh columns
    fn save_db(&mut self) -> io::Result<()> {
        self.db.save(&self.db_path)?;
        self.db = Database::load(&self.db_path); // Reload to ensure consistency
        self.update_columns();
        Ok(())
    }

    /// Set a status message
    fn set_status_message(&mut self, msg: String) {
        self.status_message = msg;
    }

    /// Clear the status message
    fn clear_status_message(&mut self) {
        self.status_message.clear();
    }

    /// Get column titles
    fn get_column_titles() -> [&'static str; 9] {
        ["Unassigned", "Ideation", "Design", "Prototyping", "Ready to Implement", "Implementation", "Testing", "Refinement", "Release"]
    }

    /// Handle keyboard input
    fn handle_input(&mut self) -> io::Result<bool> {
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Handle filter mode input
                if self.filter_active {
                    match key.code {
                        KeyCode::Esc => {
                            self.filter_active = false;
                            self.filter_text.clear();
                            self.update_columns();
                            self.clear_status_message();
                        }
                        KeyCode::Enter => {
                            self.filter_active = false;
                            if self.filter_text.is_empty() {
                                self.set_status_message("Filter cleared".to_string());
                            } else {
                                let total_tasks: usize = self.columns.iter().map(|col| col.len()).sum();
                                self.set_status_message(format!(
                                    "Filter: '{}' ({} tasks shown)",
                                    self.filter_text,
                                    total_tasks
                                ));
                            }
                        }
                        KeyCode::Backspace => {
                            if !self.filter_text.is_empty() {
                                self.filter_text.pop();
                                self.update_columns();
                            }
                        }
                        KeyCode::Char(c) => {
                            self.filter_text.push(c);
                            self.update_columns();
                        }
                        _ => {}
                    }
                    return Ok(false);
                }
                
                self.clear_status_message();

                match key.code {
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
                    KeyCode::Esc => return Ok(true),
                    // Drill down/up navigation
                    KeyCode::Char('d') => {
                        self.drill_down();
                    }
                    KeyCode::Char('u') => {
                        self.drill_up();
                    }

                    // Task detail popup
                    KeyCode::Enter => {
                        self.show_task_detail = !self.show_task_detail;
                        if !self.show_task_detail {
                            self.clear_status_message();
                        }
                    }

                    // Card movement between columns (check first, before regular navigation)
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.move_card_left();
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.move_card_right();
                    }

                    // Shift+Left/Right for filtered/unfiltered switching (check first)
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.toggle_filtered_view(false);
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.toggle_filtered_view(true);
                    }

                    // Hierarchy navigation (Alt+Left/Right for direct switching)
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                        self.switch_hierarchy_view(false);
                    }
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                        self.switch_hierarchy_view(true);
                    }

                    // Column navigation with hierarchy overflow
                    KeyCode::Left => {
                        if self.selected_column > 0 {
                            self.selected_column -= 1;
                            self.clamp_selection();
                        } else {
                            // Reached leftmost column, switch to previous hierarchy view
                            self.switch_hierarchy_view(false);
                        }
                    }
                    KeyCode::Right => {
                        if self.selected_column < self.columns.len() - 1 {
                            self.selected_column += 1;
                            self.clamp_selection();
                        } else {
                            // Reached rightmost column, switch to next hierarchy view
                            self.switch_hierarchy_view(true);
                        }
                    }

                    // Card navigation within column with scrolling
                    KeyCode::Up => {
                        if self.selected_card > 0 {
                            self.selected_card -= 1;
                            self.update_scroll_for_selection();
                        }
                    }
                    KeyCode::Down => {
                        let column_len = self.columns[self.selected_column].len();
                        if column_len > 0 && self.selected_card < column_len - 1 {
                            self.selected_card += 1;
                            self.update_scroll_for_selection();
                        }
                    }

                    // Edit task
                    KeyCode::Char('e') => {
                        if !self.columns[self.selected_column].is_empty() {
                            self.edit_task_id = Some(self.columns[self.selected_column][self.selected_card]);
                            return Ok(true); // Exit workflow to edit
                        }
                    }
                    
                    // Complete/uncomplete task
                    KeyCode::Char('c') => {
                        self.toggle_task_completion();
                    }
                    
                    // Toggle showing completed tasks
                    KeyCode::Char('t') => {
                        self.show_completed = !self.show_completed;
                        self.update_columns();
                        let status = if self.show_completed {
                            "Showing completed tasks"
                        } else {
                            "Hiding completed tasks"
                        };
                        self.set_status_message(status.to_string());
                    }
                    
                    // Filter mode
                    KeyCode::Char('/') => {
                        self.filter_active = true;
                        self.set_status_message("Filter: Type to search title/tags/project, Enter to apply, Esc to cancel".to_string());
                    }
                    
                    // Help
                    KeyCode::Char('h') => {
                        self.set_status_message("Help: Enter: Details | e: Edit | c: Complete | t: Toggle done | /: Filter | d: Drill | u: Up | m: Menu | Esc: Exit".to_string());
                    }
                    
                    _ => {}
                }
            }
        }
        Ok(false)
    }

    /// Move the selected card to the left column (previous process stage)
    fn move_card_left(&mut self) {
        if self.selected_column == 0 || self.columns[self.selected_column].is_empty() {
            return;
        }

        let task_id = self.columns[self.selected_column][self.selected_card];
        
        if let Some(task) = self.db.get_mut(task_id) {
            let new_stage = match self.selected_column {
                1 => None, // Ideation -> Unassigned
                2 => Some(ProcessStage::Ideation), // Design -> Ideation
                3 => Some(ProcessStage::Design), // Prototyping -> Design
                4 => Some(ProcessStage::Prototyping), // Ready to Implement -> Prototyping
                5 => Some(ProcessStage::ReadyToImplement), // Implementation -> Ready to Implement
                6 => Some(ProcessStage::Implementation), // Testing -> Implementation
                7 => Some(ProcessStage::Testing), // Refinement -> Testing
                8 => Some(ProcessStage::Refinement), // Release -> Refinement
                _ => return,
            };

            task.process_stage = new_stage;
            if let Err(e) = self.save_db() {
                self.set_status_message(format!("Error saving: {}", e));
            } else {
                let target_column = self.selected_column - 1;
                self.set_status_message(format!("Moved task to {}", 
                    Self::get_column_titles()[target_column]));
                self.selected_column = target_column;
                
                // Find the task in the new column and select it
                if let Some(new_position) = self.columns[target_column].iter().position(|&id| id == task_id) {
                    self.selected_card = new_position;
                } else {
                    self.clamp_selection();
                }
            }
        }
    }

    /// Move the selected card to the right column (next process stage)
    fn move_card_right(&mut self) {
        if self.selected_column >= self.columns.len() - 1 || self.columns[self.selected_column].is_empty() {
            return;
        }

        let task_id = self.columns[self.selected_column][self.selected_card];
        
        if let Some(task) = self.db.get_mut(task_id) {
            let new_stage = match self.selected_column {
                0 => Some(ProcessStage::Ideation), // Unassigned -> Ideation
                1 => Some(ProcessStage::Design), // Ideation -> Design
                2 => Some(ProcessStage::Prototyping), // Design -> Prototyping
                3 => Some(ProcessStage::ReadyToImplement), // Prototyping -> Ready to Implement
                4 => Some(ProcessStage::Implementation), // Ready to Implement -> Implementation
                5 => Some(ProcessStage::Testing), // Implementation -> Testing
                6 => Some(ProcessStage::Refinement), // Testing -> Refinement
                7 => Some(ProcessStage::Release), // Refinement -> Release
                _ => return,
            };

            task.process_stage = new_stage;
            if let Err(e) = self.save_db() {
                self.set_status_message(format!("Error saving: {}", e));
            } else {
                let target_column = self.selected_column + 1;
                self.set_status_message(format!("Moved task to {}", 
                    Self::get_column_titles()[target_column]));
                self.selected_column = target_column;
                
                // Find the task in the new column and select it
                if let Some(new_position) = self.columns[target_column].iter().position(|&id| id == task_id) {
                    self.selected_card = new_position;
                } else {
                    self.clamp_selection();
                }
            }
        }
    }

    /// Switch between hierarchy views (Product -> Epic -> Task -> Subtask)
    fn switch_hierarchy_view(&mut self, forward: bool) {
        let new_level = if forward {
            match self.navigation_context.level {
                HierarchyLevel::Product => HierarchyLevel::Epic,
                HierarchyLevel::Epic => HierarchyLevel::Task,
                HierarchyLevel::Task => HierarchyLevel::Subtask,
                HierarchyLevel::Subtask => HierarchyLevel::Product,
                HierarchyLevel::Milestone => HierarchyLevel::Product, // Skip milestones
            }
        } else {
            match self.navigation_context.level {
                HierarchyLevel::Product => HierarchyLevel::Subtask,
                HierarchyLevel::Epic => HierarchyLevel::Product,
                HierarchyLevel::Task => HierarchyLevel::Epic,
                HierarchyLevel::Subtask => HierarchyLevel::Task,
                HierarchyLevel::Milestone => HierarchyLevel::Product, // Skip milestones
            }
        };

        // Preserve the filtered state but change the level
        self.navigation_context = if let Some(parent_id) = self.navigation_context.parent_id {
            NavigationContext::new_filtered(new_level, parent_id, 
                self.navigation_context.parent_title.clone().unwrap_or_default())
        } else {
            NavigationContext::new_all_level(new_level)
        };

        self.update_columns();
        self.selected_column = 0;
        self.selected_card = 0;
        self.set_status_message(format!("Switched to {}", self.navigation_context.get_display_name()));
    }

    /// Toggle between filtered and unfiltered views within the same hierarchy level
    fn toggle_filtered_view(&mut self, _forward: bool) {
        if self.navigation_context.parent_id.is_some() {
            // Currently filtered, switch to unfiltered
            self.navigation_context = NavigationContext::new_all_level(self.navigation_context.level);
            self.set_status_message("Switched to unfiltered view".to_string());
        } else {
            // Currently unfiltered, would need to pick a parent - show message for now
            self.set_status_message("To filter: select a parent item first (feature pending)".to_string());
        }
        
        self.update_columns();
        self.selected_column = 0;
        self.selected_card = 0;
    }

    /// Drill down to the next hierarchy level using the selected task as parent
    /// This follows the exact pattern from the main app's navigate_hierarchy_contextual
    fn drill_down(&mut self) {
        if self.columns[self.selected_column].is_empty() {
            self.set_status_message("No task selected to drill down into".to_string());
            return;
        }

        let selected_task_id = self.columns[self.selected_column][self.selected_card];
        
        if let Some(task) = self.db.get(selected_task_id) {
            // Determine the child hierarchy level based on the task's kind
            let child_level = match task.kind {
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

            // Push current context to stack before changing
            self.navigation_stack.push(self.navigation_context.clone());
            
            // Create new filtered context
            self.navigation_context = NavigationContext::new_filtered(
                child_level,
                selected_task_id,
                task.title.clone()
            );

            self.update_columns();
            self.selected_column = 0;
            self.selected_card = 0;
            self.set_status_message(format!("Drilled down to {}", self.navigation_context.get_display_name()));
        }
    }

    /// Drill back up to the parent hierarchy level
    /// This follows the exact pattern from the main app's navigate_hierarchy_contextual
    fn drill_up(&mut self) {
        // Go back to previous level using the navigation stack
        if let Some(previous_context) = self.navigation_stack.pop() {
            self.navigation_context = previous_context;
            self.update_columns();
            self.selected_column = 0;
            self.selected_card = 0;
            self.set_status_message(format!("Navigated back to {}", self.navigation_context.get_display_name()));
        } else {
            self.set_status_message("No previous context to return to".to_string());
        }
    }

    /// Render the workflow kanban board
    fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Board
                Constraint::Length(1), // Status bar
            ])
            .split(f.area());

        self.render_header(f, chunks[0]);
        self.render_board(f, chunks[1]);
        self.render_status_bar(f, chunks[2]);

        // Render task detail popup if showing
        if self.show_task_detail {
            self.render_task_detail_popup(f);
        }
    }

    /// Render the header
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let project_name = self.get_current_project_name();
        let context_display = format!("Current Project: {}  Current View: {}", 
            project_name, self.navigation_context.get_display_name());
            
        let header_text = vec![
            Line::from(vec![
                Span::styled(
                    "WORKFLOW MANAGEMENT",
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
        f.render_widget(header_block, area);
    }

    /// Render the kanban board
    fn render_board(&mut self, f: &mut Frame, area: Rect) {
        let column_count = self.columns.len();
        let constraints: Vec<Constraint> = (0..column_count)
            .map(|_| Constraint::Percentage(100 / column_count as u16))
            .collect();

        let columns_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(area);

        let column_titles = Self::get_column_titles();
        
        for (i, &column_area) in columns_layout.iter().enumerate() {
            self.render_column(f, column_area, i, column_titles[i]);
        }
    }

    /// Render a single column
    fn render_column(&mut self, f: &mut Frame, area: Rect, column_index: usize, title: &str) {
        let is_selected = column_index == self.selected_column;
        let hierarchy_color = self.get_hierarchy_color();
        
        let border_style = if is_selected {
            Style::default().fg(hierarchy_color).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Render cards in this column
        let cards = &self.columns[column_index];
        if cards.is_empty() {
            return;
        }

        // All cards use the same expanded height for better title visibility
        let card_height = 5; // 5 lines to show full title
        let available_height = inner.height as usize;
        let visible_cards = available_height / card_height;
        
        // Calculate scroll offset for this column
        let scroll_offset = if is_selected {
            // Update scroll to ensure selected card is visible
            let start_visible = self.column_scroll_offsets[column_index];
            let end_visible = start_visible + visible_cards;
            
            if self.selected_card < start_visible {
                // Scroll up
                self.column_scroll_offsets[column_index] = self.selected_card;
                self.selected_card
            } else if self.selected_card >= end_visible && end_visible > 0 {
                // Scroll down
                let new_offset = self.selected_card - visible_cards + 1;
                self.column_scroll_offsets[column_index] = new_offset;
                new_offset
            } else {
                start_visible
            }
        } else {
            self.column_scroll_offsets[column_index]
        };
        
        let mut current_y = 0;
        let mut rendered_cards = 0;

        // Start rendering from the scroll offset
        for (card_index, &task_id) in cards.iter().enumerate().skip(scroll_offset) {
            if let Some(task) = self.db.get(task_id) {
                // Check if this card would fit
                if current_y + card_height > available_height {
                    break;
                }
                
                let is_this_card_selected = is_selected && card_index == self.selected_card;
                
                let card_area = Rect {
                    x: inner.x,
                    y: inner.y + current_y as u16,
                    width: inner.width,
                    height: card_height as u16,
                };

                self.render_card(f, card_area, task, is_this_card_selected);
                
                current_y += card_height;
                rendered_cards += 1;
            }
        }

        // Show scroll indicators
        if scroll_offset > 0 {
            // Show "more above" indicator
            let indicator_text = format!("▲ +{} above", scroll_offset);
            let indicator = Paragraph::new(indicator_text)
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(indicator, Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            });
        }
        
        let remaining = cards.len() - scroll_offset - rendered_cards;
        if remaining > 0 {
            // Show "more below" indicator
            let indicator_text = format!("▼ +{} below", remaining);
            let indicator = Paragraph::new(indicator_text)
                .style(Style::default().fg(Color::Cyan));
            f.render_widget(indicator, Rect {
                x: inner.x,
                y: inner.y + inner.height - 1,
                width: inner.width,
                height: 1,
            });
        }
    }

    /// Render a single task card
    fn render_card(&self, f: &mut Frame, area: Rect, task: &Task, is_selected: bool) {
        let hierarchy_color = self.get_hierarchy_color();
        
        let style = if is_selected {
            Style::default().bg(hierarchy_color).fg(Color::Black).add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(Color::DarkGray)
        };

        // All cards now show full title wrapped across multiple lines
        let mut card_text = vec![];
        
        // Show ID on first line
        card_text.push(Line::from(format!("#{}", task.id)));
        
        // Manually wrap the title to fit in the available width (accounting for borders)
        let available_width = area.width.saturating_sub(2) as usize;
        
        // Simple word wrapping
        let mut current_line = String::new();
        let mut lines = Vec::new();
        
        for word in task.title.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= available_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(current_line.clone());
                current_line = word.to_string();
                if lines.len() >= 2 {
                    break; // Maximum 2 lines of title
                }
            }
        }
        if !current_line.is_empty() && lines.len() < 2 {
            lines.push(current_line);
        }
        
        for line in lines {
            card_text.push(Line::from(line));
        }
        
        // Add status line at the bottom
        card_text.push(Line::from(format!("{} | {}", 
            format_status(task.status),
            task.project.as_deref().unwrap_or("-"))));

        let card_block = Paragraph::new(card_text)
            .block(Block::default().borders(Borders::ALL))
            .style(style)
            .wrap(Wrap { trim: true });

        f.render_widget(card_block, area);
    }

    /// Render the status bar
    fn render_status_bar(&self, f: &mut Frame, area: Rect) {
        let status_text = if self.filter_active {
            format!("Filter: {} | Type to search, Enter to apply, Esc to cancel", self.filter_text)
        } else if !self.status_message.is_empty() {
            self.status_message.clone()
        } else {
            let total_tasks: usize = self.columns.iter().map(|col| col.len()).sum();
            let completed_indicator = if self.show_completed { " [+Done]" } else { "" };
            let filter_indicator = if !self.filter_text.is_empty() { 
                format!(" [Filter: {}]", self.filter_text) 
            } else { 
                String::new() 
            };
            format!("Tasks: {}{}{} | /: Filter | c: Complete | t: Toggle done | d/u: Drill | m: Menu | h: Help", 
                total_tasks, completed_indicator, filter_indicator)
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

    /// Render the task detail popup
    fn render_task_detail_popup(&self, f: &mut Frame) {
        if self.columns[self.selected_column].is_empty() {
            return;
        }

        let task_id = self.columns[self.selected_column][self.selected_card];
        if let Some(task) = self.db.get(task_id) {
            // Create popup area (centered, 80% of screen)
            let popup_area = {
                let area = f.area();
                let popup_width = (area.width * 80) / 100;
                let popup_height = (area.height * 80) / 100;
                let x = (area.width - popup_width) / 2;
                let y = (area.height - popup_height) / 2;
                Rect::new(x, y, popup_width, popup_height)
            };

            // Clear the background
            f.render_widget(Clear, popup_area);

            // Create task detail content
            use crate::db::{format_kind, format_priority, format_urgency, format_process_stage, format_due_relative};
            use chrono::Local;

            let today = Local::now().date_naive();
            let due_str = format_due_relative(task.due, today);
            let parent_str = if let Some(parent_id) = task.parent {
                if let Some(parent_task) = self.db.get(parent_id) {
                    format!("{} ({})", parent_id, parent_task.title)
                } else {
                    parent_id.to_string()
                }
            } else {
                "-".to_string()
            };

            let mut detail_lines = vec![
                Line::from(vec![Span::styled(format!("Task #{}: {}", task.id, task.title), 
                    Style::default().add_modifier(Modifier::BOLD))]),
                Line::from(""),
                Line::from(format!("Kind:         {}", format_kind(task.kind))),
                Line::from(format!("Status:       {}", format_status(task.status))),
                Line::from(format!("Priority:     {}", format_priority(task.priority_level))),
                Line::from(format!("Urgency:      {}", format_urgency(task.urgency))),
                Line::from(format!("Process Stage: {}", format_process_stage(task.process_stage))),
                Line::from(format!("Due:          {}", due_str)),
                Line::from(format!("Parent:       {}", parent_str)),
                Line::from(format!("Project:      {}", task.project.as_deref().unwrap_or("-"))),
                Line::from(format!("Tags:         {}", if task.tags.is_empty() { "-".to_string() } else { task.tags.join(", ") })),
                Line::from(""),
                Line::from("Description:"),
                Line::from(task.description.as_deref().unwrap_or("-")),
            ];

            if let Some(ref summary) = task.summary {
                if !summary.is_empty() {
                    detail_lines.extend(vec![
                        Line::from(""),
                        Line::from("Summary:"),
                        Line::from(summary.clone()),
                    ]);
                }
            }

            let hierarchy_color = self.get_hierarchy_color();
            let popup_block = Block::default()
                .borders(Borders::ALL)
                .title("Task Details (Press Enter to close)")
                .title_alignment(Alignment::Center)
                .border_style(Style::default().fg(hierarchy_color).add_modifier(Modifier::BOLD));

            let popup_paragraph = Paragraph::new(detail_lines)
                .block(popup_block)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(Color::Black));

            f.render_widget(popup_paragraph, popup_area);
        }
    }

    /// Get the exit action requested by the user
    pub fn get_exit_action(&self) -> WorkflowExit {
        if let Some(task_id) = self.edit_task_id {
            WorkflowExit::EditTask(task_id)
        } else {
            WorkflowExit::Quit
        }
    }
    
    /// Main event loop
    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if self.handle_input()? {
                break;
            }
        }
        Ok(())
    }
}