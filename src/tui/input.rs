//! Input field handling for the terminal user interface.

/// A text input field with cursor position and active state management.
#[derive(Clone)]
pub struct InputField {
    pub value: String,
    pub cursor: usize,
    pub active: bool,
}

impl InputField {
    /// Create a new empty input field.
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            active: false,
        }
    }

    /// Create an input field with initial text value.
    pub fn with_value(value: &str) -> Self {
        Self {
            value: value.to_string(),
            cursor: value.len(),
            active: false,
        }
    }

    /// Insert a character at the current cursor position.
    pub fn handle_char(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor.
    pub fn handle_backspace(&mut self) {
        if self.cursor > 0 {
            self.value.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    /// Delete the character at the cursor position.
    pub fn handle_delete(&mut self) {
        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
        }
    }

    /// Move cursor one position to the left.
    pub fn move_cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor one position to the right.
    pub fn move_cursor_right(&mut self) {
        if self.cursor < self.value.len() {
            self.cursor += 1;
        }
    }

}