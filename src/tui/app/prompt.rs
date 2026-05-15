//! Single-line input prompts overlaid on the current mode. The `Overlay`
//! enum holds the active prompt and its buffer; this module owns the
//! keystroke handling while the prompt is collecting text and the actions
//! taken when the user confirms it.

use crossterm::event::KeyCode;

use crate::store::{artifacts, events};
use crate::tui::enums::{Overlay, PromptState, PromptType};

use super::App;

impl App {
    /// Handle a keystroke while an input prompt is collecting text.
    pub(super) fn handle_prompt_input(&mut self, key: KeyCode) {
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
}
