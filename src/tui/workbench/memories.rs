//! Memories surface: three-tier browser for the LHP-focused scope.
//!
//! Sections, top to bottom:
//!
//! 1. User memories under `~/.claude/projects/<encoded-cwd>/memory/`.
//! 2. Project memories under `.pm/projects/<PRJ>/memories/`.
//! 3. Ticket memories under `<ticket-dir>/memories/` for the focused
//!    leaf (when one exists).
//!
//! v0.3.6 is read-only: the user picks a memory and presses Enter to
//! drop into `$EDITOR` on the file. Promotion / link / unlink stay in
//! the `spacecell memory` CLI for now.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::db::Database;
use crate::memory::{scope as mem_scope, MemoryFile};
use crate::store::LeafId;
use crate::style;
use crate::tui::input::Disposition;

/// Per-row metadata: where the memory lives on disk, plus the tier
/// label rendered next to its name.
#[derive(Clone)]
struct Row {
    tier: &'static str,
    name: String,
    description: Option<String>,
    path: PathBuf,
}

pub struct MemoriesState {
    pub cursor: usize,
}

impl MemoriesState {
    pub fn new() -> Self {
        MemoriesState { cursor: 0 }
    }

    pub fn handle_key(
        &mut self,
        key: KeyCode,
        _mods: KeyModifiers,
        db: &Database,
        pm_dir: &Path,
        scope: Option<LeafId>,
    ) -> Disposition {
        let rows = enumerate_rows(db, pm_dir, scope);
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
                if let Some(row) = rows.get(self.cursor).cloned() {
                    return Disposition::EditPath(row.path);
                }
                Disposition::Consumed
            }
            KeyCode::Left => Disposition::OverflowLeft,
            KeyCode::Right => Disposition::OverflowRight,
            _ => Disposition::Consumed,
        }
    }
}

impl Default for MemoriesState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render(
    f: &mut Frame,
    area: Rect,
    db: &Database,
    pm_dir: &Path,
    scope: Option<LeafId>,
    state: &MemoriesState,
    focused_zone: bool,
) {
    let rows = enumerate_rows(db, pm_dir, scope);
    let title = format!(" MEMORIES · {} files ", rows.len());
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

    if rows.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::styled(
                "  No memories yet. Use `spacecell memory write` to add one.",
                style::muted_bright(),
            ),
            Line::from(""),
            Line::styled(
                "  User memories live in ~/.claude/projects/<cwd>/memory/",
                style::muted(),
            ),
            Line::styled(
                "  Project memories live in .pm/projects/<PRJ>/memories/",
                style::muted(),
            ),
        ];
        f.render_widget(Paragraph::new(lines).style(style::body()), inner);
        return;
    }

    let cursor = state.cursor.min(rows.len() - 1);
    let mut lines: Vec<Line> = Vec::new();
    for (idx, row) in rows.iter().enumerate() {
        let focused = focused_zone && idx == cursor;
        let pointer = if focused { "▸" } else { " " };
        let row_style = if focused {
            style::active()
        } else {
            style::body()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {pointer} "), row_style),
            Span::styled(format!("[{:<7}]", row.tier), style::id_code()),
            Span::raw(" "),
            Span::styled(row.name.clone(), row_style),
        ]));
        if let Some(desc) = row.description.as_deref() {
            lines.push(Line::from(vec![
                Span::raw("           "),
                Span::styled(desc.to_string(), style::muted()),
            ]));
        }
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

/// Walk the three tiers and return one [`Row`] per memory file.
fn enumerate_rows(db: &Database, pm_dir: &Path, scope: Option<LeafId>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();

    // User tier: ~/.claude/projects/<encoded-cwd>/memory/. cwd is
    // the workspace root from the perspective of Claude Code's
    // memory loader.
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = home {
        let user_dir = mem_scope::user_dir(&home, pm_dir);
        for path in list_md_files(&user_dir) {
            rows.push(row_from_file("user", path));
        }
    }

    // Project tier: every PRJ that exists in the database. Lists
    // across all projects so the user can see what is shared at the
    // organisational level even when their focus is inside one.
    for task in db.tasks.iter() {
        if !matches!(task.kind, crate::fields::Kind::Project) {
            continue;
        }
        let dir = mem_scope::project_dir(pm_dir, task.id);
        for path in list_md_files(&dir) {
            rows.push(row_from_file("project", path));
        }
    }

    // Ticket tier: the focused leaf's adjacent `memories/` directory,
    // if any.
    if let Some(leaf) = scope {
        if let Some(entry) = db.state.items.get(&leaf) {
            let ticket_dir = pm_dir.join(&entry.path);
            let dir = mem_scope::ticket_dir(&ticket_dir);
            for path in list_md_files(&dir) {
                rows.push(row_from_file("ticket", path));
            }
        }
    }

    rows
}

fn list_md_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn row_from_file(tier: &'static str, path: PathBuf) -> Row {
    let (name, description) = match MemoryFile::read(&path) {
        Ok(mf) => (mf.front_matter.name, mf.front_matter.description),
        Err(_) => (
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("(unparsed)")
                .to_string(),
            None,
        ),
    };
    Row {
        tier,
        name,
        description,
        path,
    }
}
