//! Full-screen ticket editor.
//!
//! Replaces the LHP + Workbench + Activity layout while open. Renders
//! a form for the 13 metadata fields plus a list of CLAUDE.md
//! sections; on save, dirty form fields are applied through the same
//! mutators the CLI verbs use so every change emits an `edit` event
//! and a git commit. Section bodies are edited externally via
//! `$EDITOR` (nvim by default); the user never sees the YAML or the
//! other sections while editing one.
//!
//! The editor is robust to agent edits because the ticket is re-read
//! from disk on every open and after every section save. Missing
//! fields render as empty; missing sections just do not appear in the
//! list; malformed YAML returns an error from the constructor instead
//! of panicking.

use std::io;
use std::path::PathBuf;

use chrono::Utc;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::db::Database;
use crate::store::claude_md::CLAUDE_MD;
use crate::store::events::{actor, append_event, Event};
use crate::store::git::commit_workspace;
use crate::store::LeafId;
use crate::style;
use crate::task::Task;

pub mod form;
pub mod sections;

use form::{
    apply_text_keystroke, cycle_kind, cycle_priority, cycle_process_stage, cycle_status,
    cycle_urgency, display_kind, display_priority, display_process_stage, display_status,
    display_urgency, FieldChange, FieldId, FieldKeyOutcome, FormState,
};
use sections::Section;

/// Outcome of a key press inside the editor.
#[derive(Debug, Clone)]
pub enum EditorOutcome {
    Continue,
    Cancel,
    Save,
    /// User asked to edit a section's prose externally. The shell
    /// suspends the alternate screen, opens `$EDITOR` on the temp
    /// file, splices back, and returns to the editor.
    EditSection {
        ticket: LeafId,
        section: Section,
    },
}

/// Cursor position inside the editor. Form fields, then section
/// rows, then the trailing buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorPos {
    Field(usize),   // index into FieldId::ALL
    Section(usize), // index into self.sections
    AddSection,
    SaveButton,
    CancelButton,
}

/// Full-screen editor state.
pub struct TicketEditor {
    pm_dir: PathBuf,
    ticket_id: LeafId,
    /// Snapshot of the task as read from the database; the diff
    /// against `form` decides what gets persisted on save.
    original: Task,
    form: FormState,
    /// CLAUDE.md path under `pm_dir/<scope-path>/`.
    claude_path: PathBuf,
    sections: Vec<Section>,
    cursor: CursorPos,
    /// True while the cursor is typing into a text field. Cleared by
    /// `Esc` or `Enter`.
    typing: bool,
    /// Transient status line shown above the footer (e.g. "saved", or
    /// an error from a validation step).
    status: Option<String>,
}

impl TicketEditor {
    /// Construct the editor for `ticket_id`. Returns `Err` when the
    /// ticket cannot be located, the CLAUDE.md is missing, or the
    /// front-matter is malformed (an agent edit gone wrong).
    pub fn open(pm_dir: PathBuf, db: &Database, ticket_id: LeafId) -> Result<Self, String> {
        let entry = db
            .state
            .items
            .get(&ticket_id)
            .ok_or_else(|| format!("ticket {ticket_id} not in state.json"))?;
        let claude_path = pm_dir.join(&entry.path).join(CLAUDE_MD);
        if !claude_path.is_file() {
            return Err(format!("missing CLAUDE.md at {}", claude_path.display()));
        }
        let task = db
            .get(ticket_id)
            .ok_or_else(|| format!("task {ticket_id} not in database"))?
            .clone();
        let sections =
            sections::parse_sections(&claude_path).map_err(|e| format!("parse sections: {e}"))?;
        let form = FormState::from_task(&task);
        Ok(TicketEditor {
            pm_dir,
            ticket_id,
            original: task,
            form,
            claude_path,
            sections,
            cursor: CursorPos::Field(0),
            typing: false,
            status: None,
        })
    }

    /// Re-parse the section list from disk. Called after `$EDITOR`
    /// splices a body change back so the editor reflects the new
    /// line counts.
    pub fn refresh_sections(&mut self) -> io::Result<()> {
        self.sections = sections::parse_sections(&self.claude_path)?;
        Ok(())
    }

    pub fn ticket_id(&self) -> LeafId {
        self.ticket_id
    }

    pub fn claude_path(&self) -> &PathBuf {
        &self.claude_path
    }

    // ---- input -------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyCode, mods: KeyModifiers) -> EditorOutcome {
        // Typing mode swallows almost everything; only Esc / Enter
        // exit it.
        if self.typing {
            self.status = None;
            let outcome = apply_text_keystroke(self.focused_text_buf(), key);
            if outcome == FieldKeyOutcome::ExitTyping {
                self.typing = false;
            }
            return EditorOutcome::Continue;
        }

        // Ctrl+S saves from anywhere.
        if matches!(key, KeyCode::Char('s')) && mods.contains(KeyModifiers::CONTROL) {
            return EditorOutcome::Save;
        }
        // Esc cancels (when not typing).
        if matches!(key, KeyCode::Esc) {
            return EditorOutcome::Cancel;
        }

        match key {
            KeyCode::Up => {
                self.move_cursor(-1);
                EditorOutcome::Continue
            }
            KeyCode::Down => {
                self.move_cursor(1);
                EditorOutcome::Continue
            }
            KeyCode::Left => {
                self.cycle_focused(-1);
                EditorOutcome::Continue
            }
            KeyCode::Right => {
                self.cycle_focused(1);
                EditorOutcome::Continue
            }
            KeyCode::Enter => self.activate(),
            _ => EditorOutcome::Continue,
        }
    }

    fn activate(&mut self) -> EditorOutcome {
        match self.cursor {
            CursorPos::Field(i) => {
                let field = FieldId::ALL[i];
                if field.is_enum() {
                    // Enums are cycled with Left/Right; Enter is a
                    // no-op so the user does not accidentally enter a
                    // typing mode that does not apply.
                    EditorOutcome::Continue
                } else {
                    self.typing = true;
                    EditorOutcome::Continue
                }
            }
            CursorPos::Section(i) => {
                if let Some(section) = self.sections.get(i).cloned() {
                    EditorOutcome::EditSection {
                        ticket: self.ticket_id,
                        section,
                    }
                } else {
                    EditorOutcome::Continue
                }
            }
            CursorPos::AddSection => {
                // v0.3.3: stub. The Add Section flow that prompts for
                // a heading and inserts it lands with the wider
                // prompts work in v0.3.4.
                self.status = Some("Add Section: drops in v0.3.4".to_string());
                EditorOutcome::Continue
            }
            CursorPos::SaveButton => EditorOutcome::Save,
            CursorPos::CancelButton => EditorOutcome::Cancel,
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let positions = self.cursor_sequence();
        let current = positions
            .iter()
            .position(|p| *p == self.cursor)
            .unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, positions.len() as i32 - 1) as usize;
        self.cursor = positions[next];
    }

    fn cycle_focused(&mut self, delta: i32) {
        if let CursorPos::Field(i) = self.cursor {
            let field = FieldId::ALL[i];
            match field {
                FieldId::Kind => self.form.kind = cycle_kind(self.form.kind, delta),
                FieldId::Status => self.form.status = cycle_status(self.form.status, delta),
                FieldId::Priority => self.form.priority = cycle_priority(self.form.priority, delta),
                FieldId::Urgency => self.form.urgency = cycle_urgency(self.form.urgency, delta),
                FieldId::ProcessStage => {
                    self.form.process_stage = cycle_process_stage(self.form.process_stage, delta)
                }
                _ => {} // text fields ignore Left/Right
            }
        }
    }

    fn cursor_sequence(&self) -> Vec<CursorPos> {
        let mut seq: Vec<CursorPos> = (0..FieldId::ALL.len()).map(CursorPos::Field).collect();
        for i in 0..self.sections.len() {
            seq.push(CursorPos::Section(i));
        }
        seq.push(CursorPos::AddSection);
        seq.push(CursorPos::SaveButton);
        seq.push(CursorPos::CancelButton);
        seq
    }

    fn focused_text_buf(&mut self) -> &mut String {
        let field = match self.cursor {
            CursorPos::Field(i) => FieldId::ALL[i],
            _ => return self.dummy_buf(),
        };
        match field {
            FieldId::Title => &mut self.form.title,
            FieldId::Summary => &mut self.form.summary,
            FieldId::Tags => &mut self.form.tags,
            FieldId::Due => &mut self.form.due,
            FieldId::Parent => &mut self.form.parent,
            FieldId::Milestone => &mut self.form.milestone,
            FieldId::IssueLink => &mut self.form.issue_link,
            FieldId::PrLink => &mut self.form.pr_link,
            _ => self.dummy_buf(),
        }
    }

    fn dummy_buf(&mut self) -> &mut String {
        // Static-ish dummy mutable string. Only reached when the
        // caller misuses focused_text_buf on a non-text field; the
        // typing flag should already be off in that case.
        static mut DUMMY: String = String::new();
        #[allow(static_mut_refs)]
        unsafe {
            &mut DUMMY
        }
    }

    // ---- save --------------------------------------------------------------

    /// Apply all dirty form-field changes through the same mutators
    /// the CLI verbs use, so every change emits an event and a commit.
    /// Returns the count of changes applied (0 means "nothing to do").
    pub fn save(&self, db: &mut Database) -> Result<usize, String> {
        let task = db
            .get(self.ticket_id)
            .ok_or_else(|| format!("task {} disappeared", self.ticket_id))?
            .clone();
        let changes = self.form.diff(&task);
        if changes.is_empty() {
            return Ok(0);
        }
        for change in &changes {
            apply_change(db, &self.pm_dir, self.ticket_id, change)?;
        }
        Ok(changes.len())
    }

    // ---- render ------------------------------------------------------------

    pub fn render(&self, f: &mut Frame, area: Rect) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(style::border_focused())
            .title(Line::from(vec![
                Span::styled(" EDIT ", style::wordmark_accent()),
                Span::styled(
                    format!(" {} · {} ", self.ticket_id, self.original.title.clone()),
                    style::body(),
                ),
            ]))
            .style(style::body());
        let inner = outer.inner(area);
        f.render_widget(outer, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // header
                Constraint::Length(FieldId::ALL.len() as u16 + 2),
                Constraint::Length((self.sections.len() as u16) + 4),
                Constraint::Min(3),    // (reserved for future preview)
                Constraint::Length(3), // buttons
                Constraint::Length(2), // footer hints
            ])
            .split(inner);

        self.render_header(f, rows[0]);
        self.render_form(f, rows[1]);
        self.render_sections(f, rows[2]);
        self.render_preview_or_status(f, rows[3]);
        self.render_buttons(f, rows[4]);
        self.render_footer(f, rows[5]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![
            Span::styled("  Kind ", style::eyebrow()),
            Span::styled(display_kind(self.original.kind), style::id_code()),
            Span::raw("    "),
            Span::styled("Status ", style::eyebrow()),
            Span::styled(display_status(self.original.status), style::id_code()),
        ];
        if let Some(p) = self.original.priority_level {
            spans.push(Span::raw("    "));
            spans.push(Span::styled("Priority ", style::eyebrow()));
            spans.push(Span::styled(display_priority(Some(p)), style::id_code()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(style::body()), area);
    }

    fn render_form(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(style::border())
            .title(Line::styled(" METADATA ", style::eyebrow()))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        for (idx, field) in FieldId::ALL.iter().enumerate() {
            let focused = matches!(self.cursor, CursorPos::Field(i) if i == idx);
            let pointer = if focused { "▸" } else { " " };
            let label = format!(" {pointer}  {:<11}  ", field.label());
            let value_text = self.field_display(*field);
            let value_styled = if focused {
                if field.is_enum() {
                    // Enum focused: show the value bracketed by cycle hints.
                    Line::from(vec![
                        Span::styled(label, style::active()),
                        Span::styled("◀ ", style::eyebrow()),
                        Span::styled(value_text, style::active()),
                        Span::styled(" ▶", style::eyebrow()),
                    ])
                } else if self.typing {
                    Line::from(vec![
                        Span::styled(label, style::active()),
                        Span::styled(value_text, style::active()),
                        Span::styled("█", style::active()),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(label, style::active()),
                        Span::styled(value_text, style::active()),
                    ])
                }
            } else {
                Line::from(vec![
                    Span::styled(label, style::body()),
                    Span::styled(value_text, style::muted_bright()),
                ])
            };
            lines.push(value_styled);
        }
        f.render_widget(Paragraph::new(lines).style(style::body()), inner);
    }

    fn field_display(&self, field: FieldId) -> String {
        match field {
            FieldId::Title => self.form.title.clone(),
            FieldId::Summary => self.form.summary.clone(),
            FieldId::Kind => display_kind(self.form.kind),
            FieldId::Status => display_status(self.form.status),
            FieldId::Priority => display_priority(self.form.priority),
            FieldId::Urgency => display_urgency(self.form.urgency),
            FieldId::ProcessStage => display_process_stage(self.form.process_stage),
            FieldId::Tags => self.form.tags.clone(),
            FieldId::Due => self.form.due.clone(),
            FieldId::Parent => self.form.parent.clone(),
            FieldId::Milestone => self.form.milestone.clone(),
            FieldId::IssueLink => self.form.issue_link.clone(),
            FieldId::PrLink => self.form.pr_link.clone(),
        }
    }

    fn render_sections(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(style::border())
            .title(Line::styled(" SECTIONS ", style::eyebrow()))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        for (idx, section) in self.sections.iter().enumerate() {
            let focused = matches!(self.cursor, CursorPos::Section(i) if i == idx);
            let pointer = if focused { "▸" } else { " " };
            let count = section.body_line_count();
            let count_label = if count == 0 {
                "empty".to_string()
            } else {
                format!("{count} lines")
            };
            let style_for_row = if focused {
                style::active()
            } else {
                style::body()
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {pointer}  "), style_for_row),
                Span::styled(format!("{:<24}", section.name), style_for_row),
                Span::styled(format!("({count_label})"), style::muted()),
            ]));
        }
        let add_focused = matches!(self.cursor, CursorPos::AddSection);
        let add_pointer = if add_focused { "▸" } else { " " };
        let add_style = if add_focused {
            style::active()
        } else {
            style::muted_bright()
        };
        lines.push(Line::from(vec![Span::styled(
            format!(" {add_pointer}  + Add section"),
            add_style,
        )]));
        f.render_widget(Paragraph::new(lines).style(style::body()), inner);
    }

    fn render_preview_or_status(&self, f: &mut Frame, area: Rect) {
        // v0.3.3 leaves the preview pane reserved (markdown rendering
        // lands as a follow-up). The status line for transient
        // messages shows here.
        if let Some(msg) = &self.status {
            let block = Block::default()
                .borders(Borders::TOP)
                .border_style(style::border())
                .title(Line::styled(" STATUS ", style::eyebrow()))
                .style(style::body());
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new(Line::styled(format!("  {msg}"), style::muted_bright()))
                    .style(style::body()),
                inner,
            );
        }
    }

    fn render_buttons(&self, f: &mut Frame, area: Rect) {
        let save_focused = matches!(self.cursor, CursorPos::SaveButton);
        let cancel_focused = matches!(self.cursor, CursorPos::CancelButton);
        let save_label = if save_focused {
            " ▸ SAVE & RETURN "
        } else {
            "   SAVE & RETURN "
        };
        let cancel_label = if cancel_focused {
            " ▸ CANCEL "
        } else {
            "   CANCEL "
        };
        let save_style = if save_focused {
            style::active()
        } else {
            style::body()
        };
        let cancel_style = if cancel_focused {
            style::active()
        } else {
            style::muted_bright()
        };
        let line = Line::from(vec![
            Span::raw("   "),
            Span::styled(save_label, save_style),
            Span::raw("    "),
            Span::styled(cancel_label, cancel_style),
        ]);
        f.render_widget(Paragraph::new(line).style(style::body()), area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let line = Line::from(vec![
            Span::styled("  ↑↓", style::id_code()),
            Span::styled(" row    ", style::muted()),
            Span::styled("←→", style::id_code()),
            Span::styled(" cycle enum    ", style::muted()),
            Span::styled("Enter", style::id_code()),
            Span::styled(" type / edit section / activate    ", style::muted()),
            Span::styled("Ctrl+S", style::id_code()),
            Span::styled(" save    ", style::muted()),
            Span::styled("Esc", style::id_code()),
            Span::styled(" cancel", style::muted()),
        ]);
        f.render_widget(Paragraph::new(line).style(style::body()), area);
    }
}

// ---------------------------------------------------------------------------
// Save: apply changes through the same mutator path the CLI verbs use
// ---------------------------------------------------------------------------

fn apply_change(
    db: &mut Database,
    pm_dir: &std::path::Path,
    leaf: LeafId,
    change: &FieldChange,
) -> Result<(), String> {
    let task = db
        .get_mut(leaf)
        .ok_or_else(|| format!("task {leaf} disappeared mid-save"))?;
    match change.clone() {
        FieldChange::Title(v) => task.title = v,
        FieldChange::Summary(v) => task.summary = v,
        FieldChange::Kind(v) => task.kind = v,
        FieldChange::Status(v) => task.status = v,
        FieldChange::Priority(v) => task.priority_level = v,
        FieldChange::Urgency(v) => task.urgency = v,
        FieldChange::ProcessStage(v) => task.process_stage = v,
        FieldChange::Tags(v) => task.tags = v,
        FieldChange::Due(v) => task.due = v,
        FieldChange::Parent(v) => task.parent = v,
        FieldChange::Milestone(v) => task.milestone = v,
        FieldChange::IssueLink(v) => task.issue_link = v,
        FieldChange::PrLink(v) => task.pr_link = v,
    }
    task.updated_at_utc = Utc::now().timestamp();

    db.save(pm_dir)
        .map_err(|e| format!("save {leaf} after {}: {e}", change.label()))?;

    let summary = format!("{} via editor", change.label());
    let _ = commit_workspace(
        pm_dir,
        &format!("pm: {} {} via editor", leaf, change.label()),
    );
    let event = Event {
        ts: Utc::now(),
        actor: actor(),
        verb: change.label().to_string(),
        id: Some(leaf),
        detail: Some(summary),
    };
    let _ = append_event(pm_dir, &event);
    Ok(())
}
