//! Left-hand panel: hierarchical navigation of the workspace.
//!
//! Renders five stacked sections (`PROJECTS`, `PRODUCTS`, `EPICS`,
//! `TASKS`, `SUBTASKS`) with cursor selection at each level. Selecting
//! an item in a level filters the children shown at the next level, and
//! filters the Workbench's Board content to that subtree.
//!
//! Implementation notes:
//! - Section bodies are recomputed from the database on every render
//!   (cheap given workspace sizes; v0.3.2 adds debounced caching when
//!   live refresh wires up).
//! - Selection state survives between renders via the per-level cursor
//!   indices on [`LhpState`].
//! - Out-of-range cursors are clamped on render, so a removed ticket
//!   does not panic the selection.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::db::{format_kind, format_status, Database};
use crate::fields::{Kind, Status};
use crate::store::LeafId;
use crate::style;
use crate::tui::nav::{ancestor_chain, tickets_at, Level};

/// Persistent LHP state. Cursor positions at each level, plus a count
/// summary that the footer can surface.
pub struct LhpState {
    /// Currently focused hierarchy level. Up/Down moves the cursor in
    /// this level's list; Left/Right shifts focus to the previous /
    /// next level.
    pub focused_level: Level,
    /// Cursor index within each level's currently visible list.
    cursors: HashMap<Level, usize>,
}

impl LhpState {
    pub fn new() -> Self {
        LhpState {
            focused_level: Level::Project,
            cursors: HashMap::new(),
        }
    }

    /// Items visible at `level` given the current parent-chain
    /// selections. `Project` is unfiltered; deeper levels filter on the
    /// selected ticket at the parent level.
    fn items_at(&self, db: &Database, level: Level) -> Vec<LeafId> {
        let parent = match level.parent() {
            None => None,
            Some(parent_level) => self.selected_at(db, parent_level),
        };
        let mut items = tickets_at(db, level, parent);
        // Stable visual ordering by id so the cursor does not jitter
        // when the database is reloaded between ticks.
        items.sort();
        items
    }

    /// The id selected at `level`, or `None` if the level has no items
    /// or the cursor is out of range.
    fn selected_at(&self, db: &Database, level: Level) -> Option<LeafId> {
        let items = self.items_at(db, level);
        if items.is_empty() {
            return None;
        }
        let cursor = self.cursors.get(&level).copied().unwrap_or(0);
        let clamped = cursor.min(items.len() - 1);
        Some(items[clamped])
    }

    /// The "current scope" the Workbench should filter to. This is the
    /// selected id at the focused level, falling back to the deepest
    /// selected ancestor if the focused level has no selection.
    pub fn scope(&self, db: &Database) -> Option<LeafId> {
        if let Some(id) = self.selected_at(db, self.focused_level) {
            return Some(id);
        }
        let mut current = self.focused_level.parent();
        while let Some(l) = current {
            if let Some(id) = self.selected_at(db, l) {
                return Some(id);
            }
            current = l.parent();
        }
        None
    }

    /// Handle a keystroke routed to the LHP. Returns `true` when the
    /// selection changed and the Workbench should re-render.
    pub fn handle_key(&mut self, key: KeyCode, _mods: KeyModifiers, db: &Database) -> bool {
        match key {
            KeyCode::Up => self.move_cursor(db, -1),
            KeyCode::Down => self.move_cursor(db, 1),
            KeyCode::Left => self.move_focus(-1),
            KeyCode::Right | KeyCode::Enter => self.move_focus(1),
            _ => false,
        }
    }

    fn move_cursor(&mut self, db: &Database, delta: i32) -> bool {
        let items = self.items_at(db, self.focused_level);
        if items.is_empty() {
            return false;
        }
        let cursor = self.cursors.get(&self.focused_level).copied().unwrap_or(0);
        let next = (cursor as i32 + delta).clamp(0, items.len() as i32 - 1) as usize;
        if next == cursor {
            return false;
        }
        self.cursors.insert(self.focused_level, next);
        // Moving the cursor at a parent level invalidates child cursors;
        // they'll clamp on next read but reset to 0 for a clean drill.
        let mut child = self.focused_level.child();
        while let Some(l) = child {
            self.cursors.insert(l, 0);
            child = l.child();
        }
        true
    }

    fn move_focus(&mut self, delta: i32) -> bool {
        let new = match (self.focused_level, delta.signum()) {
            (l, 1) => l.child(),
            (l, -1) => l.parent(),
            _ => None,
        };
        match new {
            Some(l) => {
                self.focused_level = l;
                true
            }
            None => false,
        }
    }

    /// Render the LHP into `area`. The whole rail gets bordered with the
    /// SpaceCell palette; each section is a labelled paragraph block.
    pub fn render(&self, f: &mut Frame, area: Rect, db: &Database, focused_zone: bool) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(style::border())
            .title(Line::styled(" THUNDER ", style::wordmark_accent()))
            .style(style::body());
        let inner = outer.inner(area);
        f.render_widget(outer, area);

        // Five stacked sections plus a counts summary at the bottom.
        let constraints = [
            Constraint::Min(3),
            Constraint::Min(3),
            Constraint::Min(3),
            Constraint::Min(3),
            Constraint::Min(3),
            Constraint::Length(4),
        ];
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        for (idx, level) in [
            Level::Project,
            Level::Product,
            Level::Epic,
            Level::Task,
            Level::Subtask,
        ]
        .into_iter()
        .enumerate()
        {
            self.render_section(f, rows[idx], db, level, focused_zone);
        }

        self.render_counts(f, rows[5], db);
    }

    fn render_section(
        &self,
        f: &mut Frame,
        area: Rect,
        db: &Database,
        level: Level,
        focused_zone: bool,
    ) {
        let items = self.items_at(db, level);
        let cursor = self.cursors.get(&level).copied().unwrap_or(0);
        let is_focused = focused_zone && level == self.focused_level;

        let label = format!(" {} ", level.label_upper());
        let title_style = if is_focused {
            style::active()
        } else {
            style::eyebrow()
        };

        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(style::border())
            .title(Line::styled(label, title_style))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        if items.is_empty() {
            lines.push(Line::styled("  (none)", style::muted()));
        } else {
            let clamped_cursor = cursor.min(items.len() - 1);
            // Window the visible list around the cursor so long lists
            // do not overflow the section height.
            let height = inner.height as usize;
            let start = clamped_cursor.saturating_sub(height.saturating_sub(2));
            let end = (start + height).min(items.len());
            for (offset, id) in items[start..end].iter().enumerate() {
                let abs = start + offset;
                let task = match db.get(*id) {
                    Some(t) => t,
                    None => continue,
                };
                let pointer = if abs == clamped_cursor && is_focused {
                    "▸"
                } else {
                    " "
                };
                let id_str = id.to_string();
                let style_for_row = if abs == clamped_cursor && is_focused {
                    style::active()
                } else if abs == clamped_cursor {
                    style::id_code()
                } else {
                    style::body()
                };
                let dot = status_glyph(task.status);
                lines.push(Line::from(vec![
                    Span::styled(format!(" {pointer} "), style_for_row),
                    Span::styled(dot, status_style(task.status)),
                    Span::raw(" "),
                    Span::styled(id_str, style::id_code()),
                    Span::raw(" "),
                    Span::styled(task.title.clone(), style_for_row),
                ]));
            }
        }

        let para = Paragraph::new(lines).style(style::body());
        f.render_widget(para, inner);
    }

    fn render_counts(&self, f: &mut Frame, area: Rect, db: &Database) {
        // Fixed 3-element bucket: [Open, InProgress, Done]. Status lacks
        // Hash so a HashMap is not directly usable; the small variant set
        // makes the manual tally cheaper anyway.
        let mut counts: [usize; 3] = [0, 0, 0];
        for t in db.tasks.iter() {
            // Only count things on the linear hierarchy; milestones are a
            // cross-cut and would skew the totals.
            if !matches!(
                t.kind,
                Kind::Project | Kind::Product | Kind::Epic | Kind::Task | Kind::Subtask
            ) {
                continue;
            }
            let bucket = match t.status {
                Status::Open => 0,
                Status::InProgress => 1,
                Status::Done => 2,
            };
            counts[bucket] += 1;
        }
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(style::border())
            .title(Line::styled(" TOTALS ", style::eyebrow()))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        for (status, count) in [
            (Status::Open, counts[0]),
            (Status::InProgress, counts[1]),
            (Status::Done, counts[2]),
        ] {
            lines.push(Line::from(vec![
                Span::styled(format!(" {count:>3} "), style::id_code()),
                Span::styled(format_status(status).to_uppercase(), status_style(status)),
            ]));
        }
        let para = Paragraph::new(lines).style(style::body());
        f.render_widget(para, inner);

        // Suppress unused-imports warning while keeping the function call
        // available for v0.3.2 sorting work.
        let _ = ancestor_chain;
        let _ = format_kind;
    }
}

impl Default for LhpState {
    fn default() -> Self {
        Self::new()
    }
}

fn status_glyph(status: Status) -> &'static str {
    match status {
        Status::Done => "✓",
        Status::InProgress => "●",
        Status::Open => "○",
    }
}

fn status_style(status: Status) -> ratatui::style::Style {
    match status {
        Status::Done => style::status_done(),
        Status::InProgress => style::status_in_progress(),
        Status::Open => style::status_todo(),
    }
}
