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
use crate::tui::input::Disposition;
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
    /// selected ticket at the parent level. When the parent level has
    /// no selection (empty list), child levels return empty rather than
    /// falling back to "all items at this kind" - otherwise picking a
    /// project with no products would still show every epic in the
    /// workspace at the Epic level.
    fn items_at(&self, db: &Database, level: Level) -> Vec<LeafId> {
        let mut items = match level.parent() {
            None => tickets_at(db, level, None),
            Some(parent_level) => match self.selected_at(db, parent_level) {
                Some(parent_id) => tickets_at(db, level, Some(parent_id)),
                None => Vec::new(),
            },
        };
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

    /// Handle a keystroke routed to the LHP.
    ///
    /// File-manager column model. `Up` / `Down` move the cursor within
    /// the focused level only - one keystroke, one item, no skipping
    /// across sections. To change level the user drills in or out:
    ///
    /// - `Right` / `Enter`: drill into the selected item's children
    ///   (focused level becomes the child level, cursor resets to 0).
    ///   At the deepest level (Subtask), or when the selected item has
    ///   no children, Right overflows to the Workbench so the cursor
    ///   keeps flowing rightward across the cockpit.
    /// - `Left`: drill back to the parent level, keeping the parent's
    ///   cursor where it was. At the top level (Project), Left is a
    ///   no-op until v0.3.4 adds a left-of-LHP surface.
    pub fn handle_key(&mut self, key: KeyCode, _mods: KeyModifiers, db: &Database) -> Disposition {
        match key {
            KeyCode::Up => {
                self.move_cursor_within(db, -1);
                Disposition::Consumed
            }
            KeyCode::Down => {
                self.move_cursor_within(db, 1);
                Disposition::Consumed
            }
            KeyCode::Right | KeyCode::Enter => {
                if let Some(child) = self.focused_level.child() {
                    let child_items = self.items_at(db, child);
                    if !child_items.is_empty() {
                        self.focused_level = child;
                        // Drill in starts at the first child; the
                        // descendant chain below stays at 0 so the
                        // cascade filter re-anchors cleanly.
                        self.cursors.insert(child, 0);
                        self.reset_descendant_cursors(child);
                        return Disposition::Consumed;
                    }
                }
                // No child level, or the selected item has no children:
                // hand focus to the Workbench.
                Disposition::OverflowRight
            }
            KeyCode::Left => {
                if let Some(parent) = self.focused_level.parent() {
                    self.focused_level = parent;
                    // Parent's cursor is preserved so the user lands
                    // back where they were before drilling in.
                    Disposition::Consumed
                } else {
                    // Already at the top level; nothing to drill out
                    // into until v0.3.4 adds a left-of-LHP surface.
                    Disposition::OverflowLeft
                }
            }
            _ => Disposition::Consumed,
        }
    }

    /// Move the cursor within the focused level by `delta`. Clamps to
    /// the level's item bounds; a `Down` at the end of a list is a
    /// no-op rather than rolling to the next level (the user uses
    /// `Right` / `Enter` to drill explicitly).
    fn move_cursor_within(&mut self, db: &Database, delta: i32) {
        let items = self.items_at(db, self.focused_level);
        if items.is_empty() {
            return;
        }
        let cursor = self.cursors.get(&self.focused_level).copied().unwrap_or(0);
        let next = (cursor as i32 + delta).clamp(0, items.len() as i32 - 1) as usize;
        if next == cursor {
            return;
        }
        self.cursors.insert(self.focused_level, next);
        // Changing the selection at this level invalidates the cascade
        // for descendant levels; reset their cursors so the next drill
        // starts at item 0 of the new child list.
        self.reset_descendant_cursors(self.focused_level);
    }

    /// Reset cursors below `level` to 0. Called whenever the cursor at
    /// `level` changes so the cascade filter re-anchors on the new
    /// selection's first child rather than a stale index.
    fn reset_descendant_cursors(&mut self, level: Level) {
        let mut child = level.child();
        while let Some(l) = child {
            self.cursors.insert(l, 0);
            child = l.child();
        }
    }

    /// Render the LHP into `area`. The whole rail gets bordered with the
    /// SpaceCell palette; each section is a labelled paragraph block.
    pub fn render(&self, f: &mut Frame, area: Rect, db: &Database, focused_zone: bool) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(if focused_zone {
                style::border_focused()
            } else {
                style::border()
            })
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
