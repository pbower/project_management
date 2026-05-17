//! Single-line input prompts overlaid on the current mode. The `Overlay`
//! enum holds the active prompt and its buffer; this module owns the
//! keystroke handling while the prompt is collecting text and the actions
//! taken when the user confirms it.

use crossterm::event::KeyCode;

use crate::db::format_kind;
use crate::store::{aliases::Aliases, artifacts, events, layout::Layout};
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
            PromptType::RenameTicket(leaf) => {
                let raw = prompt.buffer.trim();
                if raw.is_empty() {
                    return;
                }
                // `move <ADDRESS>` reparents; anything else is a title.
                if let Some(rest) = raw
                    .strip_prefix("move ")
                    .or_else(|| raw.strip_prefix("mv "))
                {
                    self.rename_prompt_move(leaf, rest.trim());
                } else if raw == "move" || raw == "mv" {
                    self.set_status_message(
                        "rename: `move <ADDRESS>` requires a target ticket id".to_string(),
                    );
                } else {
                    self.rename_prompt_title(leaf, raw);
                }
            }
        }
    }

    /// Rewrite the focused ticket's title in front-matter. Emits a `rename`
    /// event with the new title as the summary. Path stays unchanged because
    /// directories are LeafId-named.
    fn rename_prompt_title(&mut self, leaf: crate::store::LeafId, new_title: &str) {
        let new_title = new_title.to_string();
        if let Some(task) = self.db.get_mut(leaf) {
            task.title = new_title.clone();
            task.updated_at_utc = chrono::Utc::now().timestamp();
        } else {
            self.set_status_message(format!("rename: {leaf} not in db"));
            return;
        }
        if let Err(e) = self.db.save(&self.pm_dir) {
            self.set_status_message(format!("rename: save failed: {e}"));
            return;
        }
        let _ = events::emit_event(&self.pm_dir, "rename", Some(leaf), Some(&new_title));
        self.refresh_tasks();
        self.set_status_message(format!("{leaf}: renamed to {new_title}"));
    }

    /// Reparent the ticket under `target`, mirroring `cmd_move`'s semantics
    /// without the `println!` / `process::exit` side effects.
    fn rename_prompt_move(&mut self, leaf: crate::store::LeafId, target: &str) {
        use crate::db::validate_hierarchy;

        // Parse the target as a leaf id directly. Tolerate the same address
        // and label forms the resolver does; pull the last segment as the
        // new parent.
        let input: crate::store::IdInput = match target.parse() {
            Ok(i) => i,
            Err(e) => {
                self.set_status_message(format!("rename: parse target failed: {e}"));
                return;
            }
        };
        let target_parent = input.leaf();
        if target_parent == leaf {
            self.set_status_message("rename: parent cannot equal the ticket itself".to_string());
            return;
        }
        if self.db.get(target_parent).is_none() {
            self.set_status_message(format!("rename: target {target_parent} not found"));
            return;
        }

        let task_kind = match self.db.get(leaf) {
            Some(t) => t.kind,
            None => {
                self.set_status_message(format!("rename: {leaf} not in db"));
                return;
            }
        };
        let parent_kind = self.db.get(target_parent).unwrap().kind;
        if !validate_hierarchy(parent_kind, task_kind) {
            self.set_status_message(format!(
                "rename: invalid hierarchy: {} cannot be child of {}",
                format_kind(task_kind),
                format_kind(parent_kind),
            ));
            return;
        }

        let old_abs_dir = self
            .db
            .state
            .items
            .get(&leaf)
            .map(|entry| self.pm_dir.join(&entry.path));
        let old_address = old_address_for(&self.db, leaf);

        if let Some(task) = self.db.get_mut(leaf) {
            task.parent = Some(target_parent);
            task.updated_at_utc = chrono::Utc::now().timestamp();
        }
        if let Err(e) = self.db.save(&self.pm_dir) {
            self.set_status_message(format!("rename: save failed: {e}"));
            return;
        }

        // Clean up the old directory if the save landed elsewhere.
        let new_abs_dir = self
            .db
            .state
            .items
            .get(&leaf)
            .map(|e| self.pm_dir.join(&e.path));
        if let (Some(old), Some(new)) = (old_abs_dir.as_ref(), new_abs_dir.as_ref()) {
            if old != new && old.exists() {
                let _ = std::fs::remove_dir_all(old);
            }
        }

        // Record an alias for the old address.
        if let Some(old) = old_address {
            if let Some(new) = old_address_for(&self.db, leaf) {
                if old != new {
                    let layout = Layout::at(&self.pm_dir);
                    let aliases_path = layout.aliases_path();
                    let mut aliases = Aliases::load(&aliases_path).unwrap_or_default();
                    aliases.add(old.to_string(), new.to_string());
                    if let Err(e) = aliases.save(&aliases_path) {
                        self.set_status_message(format!("rename: alias write failed: {e}"));
                        return;
                    }
                }
            }
        }

        let _ = events::emit_event(
            &self.pm_dir,
            "move",
            Some(leaf),
            Some(&format!("-> {target_parent}")),
        );
        self.refresh_tasks();
        self.set_status_message(format!("{leaf}: moved under {target_parent}"));
    }
}

/// Walk the parent chain to compute a ticket's current address. Returns
/// `None` if the chain breaks. Duplicate of the helper in cmd.rs - kept
/// local so the prompt path does not have to import a CLI module.
fn old_address_for(
    db: &crate::db::Database,
    leaf: crate::store::LeafId,
) -> Option<crate::store::AddressId> {
    let mut chain = Vec::new();
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(id) = cursor {
        if guard > 16 {
            return None;
        }
        guard += 1;
        let task = db.get(id)?;
        chain.push(task.id);
        cursor = task.parent;
    }
    chain.reverse();
    crate::store::AddressId::new(chain).ok()
}
