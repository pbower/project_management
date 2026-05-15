//! Fullscreen text-editing dialogs - currently used for editing the User
//! Story and Requirements prose fields, which the slimmer Phase 8 quick-entry
//! form does not surface. Owns the dialog cursor model, its keystroke
//! handling, and the rendering of the editor with instruction footer.

use std::io;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::enums::{AppState, InputMode};

use super::App;

impl App {
    /// Handle keyboard input in fullscreen text editing dialogs.
    ///
    /// Used for editing user stories and requirements in dedicated fullscreen mode.
    /// Returns true if the application should quit.
    pub(super) fn handle_dialog_input(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        is_user_story: bool,
    ) -> io::Result<bool> {
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
    pub(super) fn get_dialog_cursor_position(&self) -> usize {
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
    pub(super) fn move_dialog_cursor_left(&mut self) {
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
    pub(super) fn move_dialog_cursor_right(&mut self) {
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
    pub(super) fn move_dialog_cursor_up(&mut self) {
        if self.dialog_cursor_y > 0 {
            self.dialog_cursor_y -= 1;
            let lines: Vec<&str> = self.dialog_text.lines().collect();
            if let Some(new_line) = lines.get(self.dialog_cursor_y) {
                self.dialog_cursor_x = self.dialog_cursor_x.min(new_line.len());
            }
        }
    }

    /// Move the dialog cursor down by one line.
    pub(super) fn move_dialog_cursor_down(&mut self) {
        let lines: Vec<&str> = self.dialog_text.lines().collect();
        if self.dialog_cursor_y + 1 < lines.len() {
            self.dialog_cursor_y += 1;
            if let Some(new_line) = lines.get(self.dialog_cursor_y) {
                self.dialog_cursor_x = self.dialog_cursor_x.min(new_line.len());
            }
        }
    }

    /// Initialize dialog cursor position when opening a dialog.
    pub(super) fn init_dialog_cursor(&mut self) {
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

    /// Render a fullscreen text editing dialog for user stories or requirements.
    pub(super) fn render_dialog(&mut self, f: &mut Frame, area: Rect, title: &str) {
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
                inner.y + cursor_y_visible as u16,
            ));
        }
    }
}
