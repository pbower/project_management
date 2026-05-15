//! Mode 1 ticket-detail screen. Drilling into a list row pushes
//! `AppState::TaskDetail`; this module handles the keys that work there
//! (Esc/q back, e edit, d delete confirm, p / c parent / first child) and
//! renders the metadata block including hierarchy navigation hints.

use std::io;

use chrono::Local;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::db::{
    build_children_map, format_due_relative, format_kind, format_priority, format_process_stage,
    format_status, format_urgency, project_label,
};
use crate::tui::enums::{AppState, InputMode};
use crate::tui::task_form::TaskForm;

use super::App;

impl App {
    /// Handle keyboard input when viewing task details.
    ///
    /// Returns true if the application should quit.
    pub(super) fn handle_detail_input(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> io::Result<bool> {
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

    /// Render the detailed view of a single task.
    pub(super) fn render_task_detail(&mut self, f: &mut Frame, area: Rect) {
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
}
