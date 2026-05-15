//! Destructive-action confirmation dialog. Yes / No prompt overlaid on the
//! ticket list; the only currently wired action is "delete the selected
//! ticket and its descendants".

use std::io;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::db::build_children_map;
use crate::store::LeafId;
use crate::tui::colors::DARK_RED;
use crate::tui::enums::AppState;
use crate::tui::utils::centered_rect;

use super::App;

impl App {
    /// Delete the selected task and all its descendants.
    ///
    /// Cascades deletion to all child tasks in the hierarchy.
    pub(super) fn delete_selected_task(&mut self) -> io::Result<()> {
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
    pub(super) fn handle_confirm_input(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> io::Result<bool> {
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

    /// Render a confirmation dialog for destructive actions.
    pub(super) fn render_confirm(&mut self, f: &mut Frame, area: Rect) {
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
}
