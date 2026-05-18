//! Templates surface: the six per-kind template files plus the
//! workspace-scoped launcher config. Enter opens the file in
//! `$EDITOR`; v0.3.6 ships read-only Browse + Edit, no schema
//! validation. Saved changes get picked up by the next process that
//! reads the template (the launcher reads `.thunder.toml` on every
//! spawn; the form template reader will pick changes up on the next
//! ticket open once v0.3.7's per-kind form work lands).

use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::style;
use crate::tui::input::Disposition;

pub struct TemplatesState {
    pub cursor: usize,
}

impl TemplatesState {
    pub fn new() -> Self {
        TemplatesState { cursor: 0 }
    }

    pub fn handle_key(&mut self, key: KeyCode, _mods: KeyModifiers, pm_dir: &Path) -> Disposition {
        let rows = enumerate_rows(pm_dir);
        match key {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                Disposition::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < rows.len() {
                    self.cursor += 1;
                }
                Disposition::Consumed
            }
            KeyCode::Enter => {
                if let Some(row) = rows.get(self.cursor) {
                    // Make sure the parent dir exists so the editor
                    // can create a fresh file when the user wants to
                    // start a template from scratch.
                    if let Some(parent) = row.path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    return Disposition::EditPath(row.path.clone());
                }
                Disposition::Consumed
            }
            KeyCode::Left => Disposition::OverflowLeft,
            KeyCode::Right => Disposition::OverflowRight,
            _ => Disposition::Consumed,
        }
    }
}

impl Default for TemplatesState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct Row {
    label: &'static str,
    path: std::path::PathBuf,
    exists: bool,
}

const KINDS: [&str; 6] = ["project", "product", "epic", "task", "subtask", "milestone"];

fn enumerate_rows(pm_dir: &Path) -> Vec<Row> {
    let templates_dir = pm_dir.join("templates");
    let mut rows: Vec<Row> = KINDS
        .iter()
        .map(|kind| {
            let path = templates_dir.join(format!("{kind}.toml"));
            let exists = path.is_file();
            Row {
                label: kind,
                path,
                exists,
            }
        })
        .collect();

    // Plus the workspace launcher config so this surface is the
    // single home for "config files I might want to tweak".
    let launcher_path = pm_dir.join(".thunder.toml");
    let launcher_exists = launcher_path.is_file();
    rows.push(Row {
        label: "launcher",
        path: launcher_path,
        exists: launcher_exists,
    });

    rows
}

pub fn render(
    f: &mut Frame,
    area: Rect,
    pm_dir: &Path,
    state: &TemplatesState,
    focused_zone: bool,
) {
    let rows = enumerate_rows(pm_dir);
    let title = format!(" TEMPLATES · {} files ", rows.len());
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(if focused_zone {
            style::border_focused()
        } else {
            style::border()
        })
        .title(Line::styled(title, style::eyebrow()))
        .style(style::body());
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let cursor = state.cursor.min(rows.len().saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for (idx, row) in rows.iter().enumerate() {
        let focused = focused_zone && idx == cursor;
        let pointer = if focused { "▸" } else { " " };
        let row_style = if focused {
            style::active()
        } else {
            style::body()
        };
        let presence = if row.exists { "exists  " } else { "missing " };
        let presence_style = if row.exists {
            style::status_done()
        } else {
            style::muted()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {pointer} "), row_style),
            Span::styled(format!("{:<10}", row.label), style::id_code()),
            Span::styled(presence, presence_style),
            Span::raw(" "),
            Span::styled(row.path.display().to_string(), style::muted()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Enter", style::id_code()),
        Span::styled(" open in $EDITOR   ", style::muted()),
        Span::styled("↑↓", style::id_code()),
        Span::styled(" cursor", style::muted()),
    ]));

    f.render_widget(Paragraph::new(lines).style(style::body()), inner);
}
