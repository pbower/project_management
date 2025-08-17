//! Retro-style main menu for project selection and management.
//!
//! This module provides a terminal-based menu system for selecting projects,
//! creating new projects, and viewing application information.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::project::{discover_projects, create_project, get_legacy_project, Project};
use crate::tui::utils::centered_rect;

/// Main menu application state.
pub struct MenuApp {
    pm_dir: std::path::PathBuf,
    state: MenuState,
    list_state: ListState,
    projects: Vec<Project>,
    menu_items: Vec<String>,
    input_mode: InputMode,
    input_buffer: String,
    status_message: String,
    should_exit: bool,
    selected_project: Option<Project>,
    project_to_delete: Option<Project>,
    open_workflow: bool,  // Flag to indicate workflow should be opened
}

#[derive(Debug, Clone)]
enum MenuState {
    MainMenu,
    ProjectList,
    ProjectActionMenu,  // New state for choosing TUI or Workflow
    NewProject,
    DeleteProjectList,
    DeleteConfirmation,
    About,
}

#[derive(Debug, Clone)]
enum InputMode {
    None,
    TextInput,
}

impl MenuApp {
    /// Create a new menu application.
    pub fn new(pm_dir: std::path::PathBuf) -> io::Result<Self> {
        let projects = discover_projects(&pm_dir).unwrap_or_else(|_| Vec::new());

        let menu_items = vec![
            "Open Project".to_string(),
            "New Project".to_string(),
            "Delete Project".to_string(),
            "Workflow Manager".to_string(),
            "About".to_string(),
            "Exit".to_string(),
        ];

        let mut app = MenuApp {
            pm_dir,
            state: MenuState::MainMenu,
            list_state: ListState::default(),
            projects,
            menu_items,
            input_mode: InputMode::None,
            input_buffer: String::new(),
            status_message: String::new(),
            should_exit: false,
            selected_project: None,
            project_to_delete: None,
            open_workflow: false,
        };

        app.list_state.select(Some(0));
        Ok(app)
    }

    /// Start the menu directly in workflow selection mode.
    pub fn start_workflow_selection(&mut self) {
        self.refresh_projects();
        if !self.projects.is_empty() {
            self.state = MenuState::ProjectActionMenu;
            self.list_state.select(Some(0));
        }
    }

    /// Get the selected project if one was chosen.
    pub fn get_selected_project(&self) -> Option<&Project> {
        self.selected_project.as_ref()
    }

    /// Check if the application should exit.
    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    /// Refresh the projects list.
    fn refresh_projects(&mut self) {
        self.projects = discover_projects(&self.pm_dir).unwrap_or_else(|_| Vec::new());
    }

    /// Handle keyboard input based on current state.
    fn handle_input(&mut self) -> io::Result<()> {
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                self.status_message.clear();

                match self.state {
                    MenuState::MainMenu => self.handle_main_menu_input(key.code),
                    MenuState::ProjectList => self.handle_project_list_input(key.code),
                    MenuState::ProjectActionMenu => self.handle_project_action_menu_input(key.code),
                    MenuState::NewProject => self.handle_new_project_input(key.code),
                    MenuState::DeleteProjectList => self.handle_delete_project_list_input(key.code),
                    MenuState::DeleteConfirmation => self.handle_delete_confirmation_input(key.code),
                    MenuState::About => self.handle_about_input(key.code),
                }
            }
        }
        Ok(())
    }

    /// Handle input for the main menu state.
    fn handle_main_menu_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => {
                if let Some(selected) = self.list_state.selected() {
                    if selected > 0 {
                        self.list_state.select(Some(selected - 1));
                    }
                }
            },
            KeyCode::Down => {
                if let Some(selected) = self.list_state.selected() {
                    if selected < self.menu_items.len() - 1 {
                        self.list_state.select(Some(selected + 1));
                    }
                }
            },
            KeyCode::Enter => {
                if let Some(selected) = self.list_state.selected() {
                    match selected {
                        0 => {
                            // Open Project
                            self.refresh_projects();
                            if self.projects.is_empty() {
                                // Check for legacy project
                                if let Some(legacy) = get_legacy_project(&self.pm_dir) {
                                    self.projects.push(legacy);
                                }
                            }

                            if self.projects.is_empty() {
                                self.status_message = "No projects found. Create a new project first.".to_string();
                            } else {
                                self.state = MenuState::ProjectList;
                                self.list_state.select(Some(0));
                            }
                        },
                        1 => {
                            // New Project
                            self.state = MenuState::NewProject;
                            self.input_mode = InputMode::TextInput;
                            self.input_buffer.clear();
                        },
                        2 => {
                            // Delete Project
                            self.refresh_projects();
                            if self.projects.is_empty() {
                                // Check for legacy project
                                if let Some(legacy) = get_legacy_project(&self.pm_dir) {
                                    self.projects.push(legacy);
                                }
                            }

                            if self.projects.is_empty() {
                                self.status_message = "No projects found to delete.".to_string();
                            } else {
                                self.state = MenuState::DeleteProjectList;
                                self.list_state.select(Some(0));
                            }
                        },
                        3 => {
                            // Workflow
                            self.refresh_projects();
                            if self.projects.is_empty() {
                                // Check for legacy project
                                if let Some(legacy) = get_legacy_project(&self.pm_dir) {
                                    self.projects.push(legacy);
                                }
                            }

                            if self.projects.is_empty() {
                                self.status_message = "No projects found. Create a new project first.".to_string();
                            } else {
                                self.state = MenuState::ProjectActionMenu;
                                self.list_state.select(Some(0));
                            }
                        },
                        4 => {
                            // About
                            self.state = MenuState::About;
                        },
                        5 => {
                            // Exit
                            self.should_exit = true;
                        },
                        _ => {}
                    }
                }
            },
            KeyCode::Esc | KeyCode::Char('q') => {
                self.should_exit = true;
            },
            _ => {}
        }
    }

    /// Handle input for the project action menu (workflow selection).
    fn handle_project_action_menu_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => {
                if let Some(selected) = self.list_state.selected() {
                    if selected > 0 {
                        self.list_state.select(Some(selected - 1));
                    }
                }
            },
            KeyCode::Down => {
                if let Some(selected) = self.list_state.selected() {
                    if selected < self.projects.len() - 1 {
                        self.list_state.select(Some(selected + 1));
                    }
                }
            },
            KeyCode::Enter => {
                if let Some(selected) = self.list_state.selected() {
                    if let Some(project) = self.projects.get(selected) {
                        self.selected_project = Some(project.clone());
                        self.open_workflow = true;
                        self.should_exit = true;
                    }
                }
            },
            KeyCode::Esc => {
                self.state = MenuState::MainMenu;
                self.list_state.select(Some(0));
            },
            _ => {}
        }
    }

    /// Handle input for the project list state.
    fn handle_project_list_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => {
                if let Some(selected) = self.list_state.selected() {
                    if selected > 0 {
                        self.list_state.select(Some(selected - 1));
                    }
                }
            },
            KeyCode::Down => {
                if let Some(selected) = self.list_state.selected() {
                    if selected < self.projects.len() - 1 {
                        self.list_state.select(Some(selected + 1));
                    }
                }
            },
            KeyCode::Enter => {
                if let Some(selected) = self.list_state.selected() {
                    if let Some(project) = self.projects.get(selected) {
                        self.selected_project = Some(project.clone());
                        self.should_exit = true;
                    }
                }
            },
            KeyCode::Esc => {
                self.state = MenuState::MainMenu;
                self.list_state.select(Some(0));
            },
            _ => {}
        }
    }

    /// Handle input for the new project creation state.
    fn handle_new_project_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.state = MenuState::MainMenu;
                self.input_mode = InputMode::None;
                self.input_buffer.clear();
                self.list_state.select(Some(0));
            },
            KeyCode::Enter => {
                if !self.input_buffer.trim().is_empty() {
                    match create_project(&self.input_buffer, &self.pm_dir) {
                        Ok(project) => {
                            self.selected_project = Some(project);
                            self.should_exit = true;
                        },
                        Err(e) => {
                            self.status_message = format!("Error: {}", e);
                        }
                    }
                }
            },
            KeyCode::Backspace => {
                self.input_buffer.pop();
            },
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            },
            _ => {}
        }
    }

    /// Handle input for the delete project selection state.
    fn handle_delete_project_list_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => {
                if let Some(selected) = self.list_state.selected() {
                    if selected > 0 {
                        self.list_state.select(Some(selected - 1));
                    }
                }
            },
            KeyCode::Down => {
                if let Some(selected) = self.list_state.selected() {
                    if selected < self.projects.len() - 1 {
                        self.list_state.select(Some(selected + 1));
                    }
                }
            },
            KeyCode::Enter => {
                if let Some(selected) = self.list_state.selected() {
                    if let Some(project) = self.projects.get(selected) {
                        self.project_to_delete = Some(project.clone());
                        self.state = MenuState::DeleteConfirmation;
                    }
                }
            },
            KeyCode::Esc => {
                self.state = MenuState::MainMenu;
                self.list_state.select(Some(0));
            },
            _ => {}
        }
    }

    /// Handle input for the delete confirmation dialog.
    fn handle_delete_confirmation_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Perform deletion
                if let Some(ref project) = self.project_to_delete {
                    if let Err(e) = std::fs::remove_file(&project.file_path) {
                        self.status_message = format!("Failed to delete project: {}", e);
                    } else {
                        self.status_message = format!("Project '{}' deleted successfully.", project.display_name);
                        self.refresh_projects();
                    }
                }
                self.project_to_delete = None;
                self.state = MenuState::MainMenu;
                self.list_state.select(Some(0));
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel deletion
                self.project_to_delete = None;
                self.state = MenuState::MainMenu;
                self.list_state.select(Some(0));
            },
            _ => {}
        }
    }

    /// Handle input for the about screen.
    fn handle_about_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                self.state = MenuState::MainMenu;
                self.list_state.select(Some(0));
            },
            _ => {}
        }
    }

    /// Main render function that dispatches to state-specific renderers.
    fn render(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)].as_ref())
            .split(f.area());

        match self.state {
            MenuState::MainMenu => self.render_main_menu(f, chunks[0]),
            MenuState::ProjectList => self.render_project_list(f, chunks[0]),
            MenuState::ProjectActionMenu => self.render_project_list(f, chunks[0]),  // Reuse project list for workflow selection
            MenuState::NewProject => self.render_new_project(f, chunks[0]),
            MenuState::DeleteProjectList => self.render_delete_project_list(f, chunks[0]),
            MenuState::DeleteConfirmation => self.render_delete_confirmation(f, chunks[0]),
            MenuState::About => self.render_about(f, chunks[0]),
        }

        self.render_status_bar(f, chunks[1]);
    }

    /// Render the main menu with project management options.
    fn render_main_menu(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Standard header
                Constraint::Min(0),     // Menu items
            ])
            .split(area);

        // Standard header text matching the task view
        let header_text = vec![
            Line::from(vec![
                Span::styled(
                    "PROJECT MANAGEMENT",
                    Style::default().add_modifier(Modifier::BOLD)
                )
            ])
        ];

        let header = Paragraph::new(header_text)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::White));

        f.render_widget(header, chunks[0]);

        // Menu items
        let menu_items: Vec<ListItem> = self.menu_items
            .iter()
            .map(|item| ListItem::new(Line::from(format!("  {}", item))))
            .collect();

        let menu = List::new(menu_items)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("Project Management Menu"))
            .highlight_style(Style::default().bg(Color::Gray).fg(Color::Black))
            .highlight_symbol("► ");

        f.render_stateful_widget(menu, chunks[1], &mut self.list_state);
    }

    /// Render the project selection list.
    fn render_project_list(&mut self, f: &mut Frame, area: Rect) {
        let project_items: Vec<ListItem> = self.projects
            .iter()
            .map(|project| {
                let line = if project.name == "default" {
                    Line::from(format!("  {} (legacy tasks.json)", project.display_name))
                } else {
                    Line::from(format!("  {}", project.display_name))
                };
                ListItem::new(line)
            })
            .collect();

        let projects_list = List::new(project_items)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("Select Project"))
            .highlight_style(Style::default().bg(Color::Gray).fg(Color::Black))
            .highlight_symbol("► ");

        f.render_stateful_widget(projects_list, area, &mut self.list_state);
    }

    /// Render the new project creation dialog.
    fn render_new_project(&mut self, f: &mut Frame, area: Rect) {
        let area = centered_rect(60, 30, area);
        f.render_widget(Clear, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Instructions
                Constraint::Length(3),  // Input field
                Constraint::Min(0),     // Spacer
            ])
            .split(area);

        let instructions = Paragraph::new("Enter project name:")
            .block(Block::default()
                .borders(Borders::ALL)
                .title("New Project"))
            .alignment(Alignment::Left);

        f.render_widget(instructions, chunks[0]);

        let input = Paragraph::new(self.input_buffer.as_str())
            .block(Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)));

        f.render_widget(input, chunks[1]);

        // Set cursor
        f.set_cursor_position((
            chunks[1].x + self.input_buffer.len() as u16 + 1,
            chunks[1].y + 1,
        ));
    }

    /// Render the about screen with application information.
    fn render_about(&mut self, f: &mut Frame, area: Rect) {
        let about_text = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("PM - Project Management CLI",
                    Style::default().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from("A hierarchical task management system with"),
            Line::from("support for multiple projects and workflows."),
            Line::from(""),
            Line::from("Version: 0.1.0"),
            Line::from(""),
            Line::from(vec![
                Span::raw("© Peter Garfield Bower "),
                Span::styled("github.com/pbower",
                    Style::default().fg(Color::Cyan)),
            ]),
            Line::from(""),
            Line::from("Press any key to return to main menu"),
        ];

        let about = Paragraph::new(about_text)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("About"))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(about, area);
    }

    /// Render the project selection list for deletion.
    fn render_delete_project_list(&mut self, f: &mut Frame, area: Rect) {
        let project_items: Vec<ListItem> = self.projects
            .iter()
            .map(|project| {
                let line = if project.name == "default" {
                    Line::from(format!("  {} (legacy tasks.json)", project.display_name))
                } else {
                    Line::from(format!("  {}", project.display_name))
                };
                ListItem::new(line)
            })
            .collect();

        let projects_list = List::new(project_items)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("Select Project to Delete"))
            .highlight_style(Style::default().bg(Color::Red).fg(Color::White))
            .highlight_symbol("► ");

        f.render_stateful_widget(projects_list, area, &mut self.list_state);
    }

    /// Render the delete confirmation dialog with warning.
    fn render_delete_confirmation(&mut self, f: &mut Frame, area: Rect) {
        let area = centered_rect(70, 40, area);
        f.render_widget(Clear, area);

        let project_name = self.project_to_delete
            .as_ref()
            .map(|p| p.display_name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let confirmation_text = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Are you sure?",
                    Style::default().add_modifier(Modifier::BOLD).fg(Color::Red)),
            ]),
            Line::from(""),
            Line::from(format!("This will permanently delete project: {}", project_name)),
            Line::from(""),
            Line::from(vec![
                Span::styled("This action is unrecoverable.",
                    Style::default().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from("Note: We recommend you apply git source control"),
            Line::from("to ~/.pm for backup purposes."),
            Line::from(""),
            Line::from(""),
            Line::from("Press Y to confirm deletion, N or Esc to cancel"),
        ];

        let confirmation = Paragraph::new(confirmation_text)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("Delete Project")
                .border_style(Style::default().fg(Color::Red)))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(confirmation, area);
    }

    /// Render the status bar with context-appropriate help text.
    fn render_status_bar(&mut self, f: &mut Frame, area: Rect) {
        let status_text = if !self.status_message.is_empty() {
            self.status_message.clone()
        } else {
            match self.state {
                MenuState::MainMenu => "Use ↑↓ to navigate, Enter to select, q/Esc to quit".to_string(),
                MenuState::ProjectList => "Use ↑↓ to navigate, Enter to select, Esc to go back".to_string(),
                MenuState::ProjectActionMenu => "Select a project for Workflow - Use ↑↓ to navigate, Enter to select, Esc to go back".to_string(),
                MenuState::NewProject => "Type project name, Enter to create, Esc to cancel".to_string(),
                MenuState::DeleteProjectList => "Use ↑↓ to navigate, Enter to select, Esc to go back".to_string(),
                MenuState::DeleteConfirmation => "Press Y to confirm, N or Esc to cancel".to_string(),
                MenuState::About => "Press any key to return".to_string(),
            }
        };

        let status = Paragraph::new(status_text)
            .style(Style::default().bg(Color::Blue).fg(Color::White))
            .alignment(Alignment::Left);

        f.render_widget(status, area);
    }

    /// Main event loop for the menu application.
    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            self.handle_input()?;

            if self.should_exit {
                break;
            }
        }
        Ok(())
    }

    /// Check if workflow should be opened.
    pub fn should_open_workflow(&self) -> bool {
        self.open_workflow
    }

    /// Reset the workflow flag after it's been used.
    pub fn reset_workflow_flag(&mut self) {
        self.open_workflow = false;
        self.selected_project = None;
        self.should_exit = false;
    }
}
