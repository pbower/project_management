//! End-to-end orchestration of CLAUDE.md scaffolding, reading, and writing.
//!
//! This module is the public API the rest of the crate (and Phase 4 CLI verbs,
//! and Phase 11 MCP tools) talks to. It composes the lower-level modules:
//!
//! - [`crate::store::front_matter`] for YAML metadata.
//! - [`crate::store::sections`] for markdown body parsing.
//! - [`crate::store::templates`] for per-kind section templates.
//!
//! Each ticket lives at `<ticket-dir>/CLAUDE.md`. Files always close with a
//! single trailing line `@artifacts/ARTIFACTS.md` so Claude Code's `@`-import
//! pulls in the artifact index without callers having to load the directory
//! explicitly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;

use super::front_matter::{Document, FrontMatter, FrontMatterError};
use super::id::LeafId;
use super::sections::ParsedBody;
use super::templates;

/// Filename of the per-ticket markdown.
pub const CLAUDE_MD: &str = "CLAUDE.md";

/// Final trailer line on every scaffolded CLAUDE.md. Pulls in the artifact
/// index via Claude Code's `@`-import.
pub const ARTIFACTS_IMPORT: &str = "@artifacts/ARTIFACTS.md";

/// A full ticket as it lives on disk: typed metadata + structured body.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticket {
    pub front_matter: FrontMatter,
    pub body: ParsedBody,
}

impl Ticket {
    /// Build a brand-new ticket from a leaf id, title, and template content.
    /// The template is applied to an empty body via [`templates::apply`] so
    /// the section headings appear in template order with empty bodies, plus
    /// the trailing `@artifacts/ARTIFACTS.md` line.
    pub fn scaffold(leaf: LeafId, title: impl Into<String>, template: &str) -> Self {
        let front_matter = FrontMatter::new(leaf, title);
        let body = templates::scaffold(template);
        Ticket { front_matter, body }
    }

    /// Read a `CLAUDE.md` file into a parsed ticket.
    pub fn read(path: &Path) -> Result<Self, TicketError> {
        let doc = Document::read(path).map_err(TicketError::FrontMatter)?;
        let body = ParsedBody::parse(strip_trailer(&doc.body).0);
        Ok(Ticket {
            front_matter: doc.front_matter,
            body,
        })
    }

    /// Render this ticket to a complete CLAUDE.md-shaped string with the
    /// front-matter delimiter lines and trailing `@artifacts/...` import.
    pub fn render(&self) -> Result<String, TicketError> {
        let mut body = self.body.render();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        // Ensure there is a blank line between the last section and the
        // trailer so it does not visually merge with the section body.
        if !body.is_empty() && !body.ends_with("\n\n") {
            body.push('\n');
        }
        body.push_str(ARTIFACTS_IMPORT);
        body.push('\n');

        let doc = Document {
            front_matter: self.front_matter.clone(),
            body,
        };
        doc.render().map_err(TicketError::FrontMatter)
    }

    /// Write the ticket to `<dir>/CLAUDE.md`. Creates the parent directory if
    /// it does not exist (callers usually use `Layout::ensure_node_path`
    /// first, but this is safe either way). Atomic via temp-file+rename.
    pub fn write_to(&self, dir: &Path) -> Result<PathBuf, TicketError> {
        fs::create_dir_all(dir).map_err(TicketError::Io)?;
        // Bump updated timestamp on every write.
        let mut me = self.clone();
        me.front_matter.updated = Utc::now();

        let rendered = me.render()?;
        let path = dir.join(CLAUDE_MD);
        super::state::atomic_write(&path, rendered.as_bytes()).map_err(TicketError::Io)?;
        Ok(path)
    }

    /// Insert or replace a section by name. Returns `true` if a section
    /// existed under that name, `false` if a new one was appended.
    pub fn upsert_section(&mut self, name: &str, body: impl Into<String>) -> bool {
        self.body.upsert(name, body)
    }

    /// Apply (or re-apply) a per-kind template. Existing section content is
    /// preserved for matching names; user-added sections are kept; order
    /// becomes template-first then leftover sections.
    pub fn apply_template(&mut self, template: &str) {
        templates::apply(template, &mut self.body);
    }
}

/// Errors emitted by Ticket operations.
#[derive(Debug)]
pub enum TicketError {
    Io(io::Error),
    FrontMatter(FrontMatterError),
}

impl std::fmt::Display for TicketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TicketError::Io(e) => write!(f, "ticket io: {e}"),
            TicketError::FrontMatter(e) => write!(f, "ticket front-matter: {e}"),
        }
    }
}

impl std::error::Error for TicketError {}

/// Strip the trailing `@artifacts/ARTIFACTS.md` import line from a body
/// before re-parsing as sections. Returns the stripped body and whether the
/// import was present. The trailing blank-line separator that precedes the
/// import is left in place; `ParsedBody::parse` strips it as part of
/// closing the last section.
fn strip_trailer(body: &str) -> (&str, bool) {
    let trimmed = body.trim_end_matches('\n');
    if let Some(without) = trimmed.strip_suffix(ARTIFACTS_IMPORT) {
        (without, true)
    } else {
        (body, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;
    use crate::store::templates::builtin;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-claude-md-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn task_leaf() -> LeafId {
        LeafId::new(TypePrefix::Task, 7)
    }

    #[test]
    fn scaffold_produces_template_sections_with_empty_bodies() {
        let t = Ticket::scaffold(task_leaf(), "Lock protocol", builtin(TypePrefix::Task));
        assert_eq!(t.front_matter.title, "Lock protocol");
        assert_eq!(
            t.body.names(),
            vec![
                "Description",
                "User Story",
                "Requirements",
                "Acceptance Criteria",
                "Notes"
            ],
        );
        for s in &t.body.sections {
            assert!(
                s.body.is_empty(),
                "fresh scaffold section {} has unexpected content",
                s.name
            );
        }
    }

    #[test]
    fn render_includes_artifacts_import_trailer() {
        let t = Ticket::scaffold(task_leaf(), "Lock protocol", builtin(TypePrefix::Task));
        let rendered = t.render().unwrap();
        assert!(
            rendered.starts_with("---\n"),
            "must start with YAML delimiter"
        );
        assert!(rendered.contains("\n---\n"), "must close YAML delimiter");
        assert!(rendered.contains("# Description"));
        assert!(
            rendered.ends_with("@artifacts/ARTIFACTS.md\n"),
            "must end with the artifacts @-import"
        );
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tmp_dir();
        let ticket_dir = dir.join("ticket");
        let mut t = Ticket::scaffold(task_leaf(), "Lock protocol", builtin(TypePrefix::Task));
        t.upsert_section("Description", "We need a heartbeat lock.\n");

        let path = t.write_to(&ticket_dir).unwrap();
        assert_eq!(path, ticket_dir.join(CLAUDE_MD));

        let back = Ticket::read(&path).unwrap();
        assert_eq!(back.front_matter.id, t.front_matter.id);
        assert_eq!(back.front_matter.title, t.front_matter.title);
        assert_eq!(
            back.body.find("Description").unwrap().body,
            "We need a heartbeat lock.\n",
        );
        assert_eq!(back.body.names(), t.body.names());
        // The trailer is stripped on read so it does not become part of any
        // section body.
        assert!(!back.body.find("Notes").unwrap().body.contains("@artifacts"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_preserves_other_sections_via_disk() {
        let dir = tmp_dir();
        let ticket_dir = dir.join("ticket");
        let mut t = Ticket::scaffold(task_leaf(), "Lock", builtin(TypePrefix::Task));
        t.upsert_section("Description", "Initial.\n");
        t.upsert_section("Requirements", "Must support TTL.\n");
        t.write_to(&ticket_dir).unwrap();

        // Reload, mutate one section, write, reload.
        let mut reloaded = Ticket::read(&ticket_dir.join(CLAUDE_MD)).unwrap();
        reloaded.upsert_section("User Story", "As an agent...\n");
        reloaded.write_to(&ticket_dir).unwrap();

        let final_state = Ticket::read(&ticket_dir.join(CLAUDE_MD)).unwrap();
        assert_eq!(
            final_state.body.find("Description").unwrap().body,
            "Initial.\n"
        );
        assert_eq!(
            final_state.body.find("Requirements").unwrap().body,
            "Must support TTL.\n"
        );
        assert_eq!(
            final_state.body.find("User Story").unwrap().body,
            "As an agent...\n"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_template_preserves_content_and_user_section() {
        let mut t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Subtask));
        // Body starts with subtask template (Description, Notes).
        t.upsert_section("Description", "Subtask content.\n");
        // Add a user section.
        t.upsert_section("Field Notes", "Hand-added.\n");
        // Upgrade the ticket to a task by reapplying the task template.
        t.apply_template(builtin(TypePrefix::Task));

        // Task template sections come first.
        assert_eq!(
            &t.body.names()[..5],
            &[
                "Description",
                "User Story",
                "Requirements",
                "Acceptance Criteria",
                "Notes"
            ],
        );
        // The user-added section survives at the tail.
        assert!(t.body.names().contains(&"Field Notes"));
        // Description content preserved.
        assert_eq!(
            t.body.find("Description").unwrap().body,
            "Subtask content.\n"
        );
        // New template sections start empty.
        assert!(t.body.find("User Story").unwrap().body.is_empty());
    }

    #[test]
    fn fixture_blank_ticket() {
        let t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Task));
        let rendered = t.render().unwrap();
        // No section contains any body content other than headings.
        for name in [
            "Description",
            "User Story",
            "Requirements",
            "Acceptance Criteria",
            "Notes",
        ] {
            let pat = format!("# {name}\n");
            assert!(rendered.contains(&pat), "missing heading {name}");
        }
    }

    #[test]
    fn fixture_full_ticket_round_trips() {
        let dir = tmp_dir();
        let ticket_dir = dir.join("ticket");
        let mut t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Task));
        t.upsert_section("Description", "D.\n");
        t.upsert_section("User Story", "US.\n");
        t.upsert_section("Requirements", "R.\n");
        t.upsert_section("Acceptance Criteria", "AC.\n");
        t.upsert_section("Notes", "N.\n");
        let path = t.write_to(&ticket_dir).unwrap();
        let back = Ticket::read(&path).unwrap();
        for name in [
            "Description",
            "User Story",
            "Requirements",
            "Acceptance Criteria",
            "Notes",
        ] {
            assert!(
                !back.body.find(name).unwrap().body.is_empty(),
                "{name} body missing"
            );
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fixture_user_added_section_survives_round_trip() {
        let dir = tmp_dir();
        let ticket_dir = dir.join("ticket");
        let mut t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Task));
        t.upsert_section("Performance Notes", "Hand-written.\n");
        t.write_to(&ticket_dir).unwrap();
        let back = Ticket::read(&ticket_dir.join(CLAUDE_MD)).unwrap();
        assert_eq!(
            back.body.find("Performance Notes").unwrap().body,
            "Hand-written.\n"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fixture_reordered_sections_survive_round_trip() {
        let dir = tmp_dir();
        let ticket_dir = dir.join("ticket");
        let mut t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Task));
        // Simulate a hand-edit that reordered sections: Notes first.
        let original_order = t
            .body
            .names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let notes = t.body.remove("Notes").unwrap();
        t.body.sections.insert(0, notes);
        assert_ne!(t.body.names(), original_order, "reorder sanity check");

        t.write_to(&ticket_dir).unwrap();
        let back = Ticket::read(&ticket_dir.join(CLAUDE_MD)).unwrap();
        assert_eq!(
            back.body.names()[0],
            "Notes",
            "reordered Notes-first must survive a round-trip"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fixture_deleted_section_not_resurrected_on_read() {
        let dir = tmp_dir();
        let ticket_dir = dir.join("ticket");
        let mut t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Task));
        t.body.remove("Acceptance Criteria");
        t.write_to(&ticket_dir).unwrap();
        let back = Ticket::read(&ticket_dir.join(CLAUDE_MD)).unwrap();
        assert!(
            back.body.find("Acceptance Criteria").is_none(),
            "deleted section must not reappear without an explicit apply_template"
        );
    }

    #[test]
    fn apply_template_restores_deleted_section_with_empty_body() {
        let mut t = Ticket::scaffold(task_leaf(), "T", builtin(TypePrefix::Task));
        t.upsert_section("Description", "Kept.\n");
        t.body.remove("Acceptance Criteria");
        t.apply_template(builtin(TypePrefix::Task));
        assert_eq!(t.body.find("Description").unwrap().body, "Kept.\n");
        assert!(t.body.find("Acceptance Criteria").unwrap().body.is_empty());
    }
}
