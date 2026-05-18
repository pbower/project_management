//! Board surface: the kanban grouped by [`ProcessStage`] for tickets
//! within the LHP's currently selected scope.
//!
//! v0.3.1 ships a read-only renderer with column scrolling and card
//! focus. Card movement, card-detail drill-down, and the live re-render
//! on events.log changes land in v0.3.2. The full 9-stage drag-and-drop
//! board with edit dispatch stays at `spacecell wf` until v0.3.3 retires
//! it.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::db::Database;
use crate::fields::{Kind, ProcessStage, Status};
use crate::store::LeafId;
use crate::style;
use crate::tui::input::Disposition;

/// Persistent board state. Per-column scroll plus the focused column /
/// card cursor pair.
pub struct BoardState {
    pub focused_column: usize,
    pub focused_card: usize,
    pub scroll: [usize; 9],
}

impl BoardState {
    pub fn new() -> Self {
        BoardState {
            focused_column: 0,
            focused_card: 0,
            scroll: [0; 9],
        }
    }

    pub fn handle_key(&mut self, key: KeyCode, _mods: KeyModifiers, db: &Database) -> Disposition {
        let columns = columns_in_scope(db, None);
        match key {
            KeyCode::Left => {
                if self.focused_column == 0 {
                    // Left from the leftmost column hands focus back to
                    // the LHP so arrow-key navigation flows seamlessly
                    // across the cockpit.
                    return Disposition::OverflowLeft;
                }
                self.focused_column -= 1;
                self.focused_card = 0;
                Disposition::Consumed
            }
            KeyCode::Right => {
                if self.focused_column + 1 >= columns.len() {
                    return Disposition::OverflowRight;
                }
                self.focused_column += 1;
                self.focused_card = 0;
                Disposition::Consumed
            }
            KeyCode::Down => {
                let col_len = columns
                    .get(self.focused_column)
                    .map(|c| c.cards.len())
                    .unwrap_or(0);
                if col_len > 0 && self.focused_card + 1 < col_len {
                    self.focused_card += 1;
                }
                Disposition::Consumed
            }
            KeyCode::Up => {
                if self.focused_card > 0 {
                    self.focused_card -= 1;
                }
                Disposition::Consumed
            }
            KeyCode::Enter => {
                // Enter opens the focused card in the full-screen
                // ticket editor (form + section list). For raw
                // CLAUDE.md access bypassing the form, `e` is the
                // power-user escape hatch (handled below).
                if let Some(col) = columns.get(self.focused_column) {
                    if let Some(id) = col.cards.get(self.focused_card) {
                        return Disposition::OpenTicketEditor(*id);
                    }
                }
                Disposition::Consumed
            }
            KeyCode::Char('e') => {
                // Escape hatch: open the focused card's raw CLAUDE.md
                // in $EDITOR. Skips the form so power users can edit
                // front-matter, sections, and the @-import line all
                // at once.
                if let Some(col) = columns.get(self.focused_column) {
                    if let Some(id) = col.cards.get(self.focused_card) {
                        return Disposition::EditRaw(*id);
                    }
                }
                Disposition::Consumed
            }
            _ => Disposition::Consumed,
        }
    }
}

impl Default for BoardState {
    fn default() -> Self {
        Self::new()
    }
}

/// One column of the kanban. Title plus the cards (in ticket-id order).
struct Column {
    title: &'static str,
    cards: Vec<LeafId>,
}

const STAGE_TITLES: [&str; 9] = [
    "NONE", "IDEATION", "DESIGN", "PROTO", "READY", "IMPL", "TEST", "REFINE", "RELEASE",
];

fn stage_index(stage: Option<ProcessStage>) -> usize {
    match stage {
        None => 0,
        Some(ProcessStage::Ideation) => 1,
        Some(ProcessStage::Design) => 2,
        Some(ProcessStage::Prototyping) => 3,
        Some(ProcessStage::ReadyToImplement) => 4,
        Some(ProcessStage::Implementation) => 5,
        Some(ProcessStage::Testing) => 6,
        Some(ProcessStage::Refinement) => 7,
        Some(ProcessStage::Release) => 8,
    }
}

/// Compute the column layout for tickets at or below `scope`. When
/// `scope` is `None`, the whole workspace is in view.
fn columns_in_scope(db: &Database, scope: Option<LeafId>) -> Vec<Column> {
    let mut cols: Vec<Column> = STAGE_TITLES
        .iter()
        .map(|title| Column {
            title,
            cards: Vec::new(),
        })
        .collect();

    for task in db.tasks.iter() {
        // Board only shows worker-level tickets, not the wrapping nodes
        // that act as containers.
        if !matches!(task.kind, Kind::Task | Kind::Subtask | Kind::Epic) {
            continue;
        }
        if let Some(scope_id) = scope {
            if !is_descendant_of(db, task.id, scope_id) {
                continue;
            }
        }
        cols[stage_index(task.process_stage)].cards.push(task.id);
    }
    for col in cols.iter_mut() {
        col.cards.sort();
    }
    cols
}

/// `true` if `id` is `ancestor` itself or a descendant via parent links.
/// Linear walk capped at 16 to guard against pathological data.
fn is_descendant_of(db: &Database, id: LeafId, ancestor: LeafId) -> bool {
    let mut cursor = Some(id);
    let mut guard = 0;
    while let Some(c) = cursor {
        if c == ancestor {
            return true;
        }
        if guard > 16 {
            return false;
        }
        guard += 1;
        cursor = db.get(c).and_then(|t| t.parent);
    }
    false
}

/// Render the board into `area`. Reads `state` for cursor position;
/// `scope` filters the visible cards.
pub fn render(
    f: &mut Frame,
    area: Rect,
    db: &Database,
    scope: Option<LeafId>,
    state: &BoardState,
    focused_zone: bool,
) {
    let cols = columns_in_scope(db, scope);
    let total: usize = cols.iter().map(|c| c.cards.len()).sum();
    let scope_label = scope
        .map(|id| format!("scope {id}"))
        .unwrap_or_else(|| "workspace".to_string());

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(if focused_zone {
            style::border_focused()
        } else {
            style::border()
        })
        .title(Line::styled(
            format!(" BOARD · {scope_label} · {total} tickets "),
            style::eyebrow(),
        ))
        .style(style::body());
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    if inner.width < 30 || inner.height < 6 {
        return;
    }

    let constraints: Vec<Constraint> = (0..cols.len())
        .map(|_| Constraint::Ratio(1, cols.len() as u32))
        .collect();
    let column_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(inner);

    for (idx, col) in cols.iter().enumerate() {
        render_column(
            f,
            column_rects[idx],
            db,
            col,
            idx == state.focused_column && focused_zone,
            if idx == state.focused_column {
                Some(state.focused_card)
            } else {
                None
            },
        );
    }
}

fn render_column(
    f: &mut Frame,
    area: Rect,
    db: &Database,
    col: &Column,
    focused_column: bool,
    focused_card: Option<usize>,
) {
    let title_style = if focused_column {
        style::active()
    } else {
        style::eyebrow()
    };

    let title = format!(" {} · {} ", col.title, col.cards.len());
    let block = Block::default()
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(style::border())
        .title(Line::styled(title, title_style))
        .style(style::body());
    let inner = block.inner(area);
    f.render_widget(block, area);

    if col.cards.is_empty() {
        let para = Paragraph::new(Line::styled("  (empty)", style::muted())).style(style::body());
        f.render_widget(para, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (idx, id) in col.cards.iter().enumerate() {
        let Some(task) = db.get(*id) else { continue };
        let row_focused = focused_card == Some(idx);
        let row_style: Style = if row_focused {
            style::active()
        } else {
            style::body()
        };
        let pointer = if row_focused { "▸" } else { " " };
        let id_style = if row_focused {
            style::active()
        } else {
            style::id_code()
        };
        let dot_style = match task.status {
            Status::Done => style::status_done(),
            Status::InProgress => style::status_in_progress(),
            Status::Open => style::status_todo(),
        };
        let dot = match task.status {
            Status::Done => "✓",
            Status::InProgress => "●",
            Status::Open => "○",
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {pointer} "), row_style),
            Span::styled(dot, dot_style),
            Span::raw(" "),
            Span::styled(id.to_string(), id_style),
        ]));
        // Title on the next line so even narrow columns can show it.
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(truncate(&task.title, area.width as usize - 4), row_style),
        ]));
        lines.push(Line::raw(""));
    }

    f.render_widget(Paragraph::new(lines).style(style::body()), inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max < 2 {
        s.chars().take(max).collect()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_index_covers_every_variant() {
        assert_eq!(stage_index(None), 0);
        assert_eq!(stage_index(Some(ProcessStage::Ideation)), 1);
        assert_eq!(stage_index(Some(ProcessStage::Release)), 8);
        for stage in [
            ProcessStage::Ideation,
            ProcessStage::Design,
            ProcessStage::Prototyping,
            ProcessStage::ReadyToImplement,
            ProcessStage::Implementation,
            ProcessStage::Testing,
            ProcessStage::Refinement,
            ProcessStage::Release,
        ] {
            let idx = stage_index(Some(stage));
            assert!(idx < STAGE_TITLES.len(), "{stage:?} -> {idx} out of range");
        }
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
    }
}
