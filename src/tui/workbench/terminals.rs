//! Terminals surface: live view of the launcher registry.
//!
//! Reads `.pm/terminals/*.json` on every render so the table reflects
//! freshly spawned terminals without needing an explicit refresh key.
//! Per-row actions cover the daily-driver operations: focus an
//! existing terminal (delegates to the configured focus command),
//! kill a terminal (sends SIGINT to the recorded pid), or open the
//! ticket the terminal is scoped to.

use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::launcher::{list_terminals, TerminalEntry, TerminalStatus};
use crate::style;
use crate::tui::input::Disposition;

/// Persistent state for the Terminals surface. Just the cursor for
/// now; v0.3.7 may add a filter (Active / Closed / Dead) here.
pub struct TerminalsState {
    pub cursor: usize,
}

impl TerminalsState {
    pub fn new() -> Self {
        TerminalsState { cursor: 0 }
    }

    /// Forward keystrokes to the surface. Returns a [`Disposition`]
    /// so the shell knows when the user has asked to leave the zone
    /// or open the ticket editor for the focused terminal's scope.
    pub fn handle_key(&mut self, key: KeyCode, _mods: KeyModifiers, pm_dir: &Path) -> Disposition {
        let entries = list_terminals(pm_dir).unwrap_or_default();

        match key {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                Disposition::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < entries.len() {
                    self.cursor += 1;
                }
                Disposition::Consumed
            }
            KeyCode::Char('o') => {
                if let Some(entry) = entries.get(self.cursor).cloned() {
                    invoke_focus(pm_dir, &entry);
                }
                Disposition::Consumed
            }
            KeyCode::Char('K') => {
                // Kill is uppercase-only so a tap of lowercase k for
                // cursor-up never sends a signal by accident.
                if let Some(entry) = entries.get(self.cursor).cloned() {
                    kill_terminal(&entry);
                }
                Disposition::Consumed
            }
            KeyCode::Enter => {
                // Drop into the ticket editor for the terminal's scope
                // so the user can review the work the agent did before
                // closing the terminal.
                if let Some(entry) = entries.get(self.cursor).cloned() {
                    return Disposition::OpenTicketEditor(entry.scope);
                }
                Disposition::Consumed
            }
            KeyCode::Left => Disposition::OverflowLeft,
            KeyCode::Right => Disposition::OverflowRight,
            _ => Disposition::Consumed,
        }
    }
}

impl Default for TerminalsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the surface into `area`. Reads the registry on every call;
/// fresh spawn / close events surface within one tick.
pub fn render(
    f: &mut Frame,
    area: Rect,
    pm_dir: &Path,
    state: &TerminalsState,
    focused_zone: bool,
) {
    let entries = list_terminals(pm_dir).unwrap_or_default();
    let total = entries.len();
    let active = entries
        .iter()
        .filter(|e| e.status == TerminalStatus::Active)
        .count();

    let title = format!(" TERMINALS · {active} active / {total} total ");
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

    if entries.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::styled(
                "  No terminals yet. Spawn one with `spacecell run <id>`.",
                style::muted_bright(),
            ),
            Line::from(""),
            Line::styled(
                "  Configure the launcher in .pm/.thunder.toml.",
                style::muted(),
            ),
        ];
        f.render_widget(Paragraph::new(lines).style(style::body()), inner);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    render_header(f, rows[0]);
    render_rows(f, rows[1], &entries, state.cursor, focused_zone);
    render_keys(f, rows[2]);
}

fn render_header(f: &mut Frame, area: Rect) {
    let header = Line::from(vec![
        Span::styled(format!("  {:<14}", "UUID"), style::eyebrow()),
        Span::styled(format!("{:<8}", "SCOPE"), style::eyebrow()),
        Span::styled(format!("{:<22}", "AGENT"), style::eyebrow()),
        Span::styled(format!("{:<8}", "PID"), style::eyebrow()),
        Span::styled(format!("{:<10}", "HEARTBEAT"), style::eyebrow()),
        Span::styled("STATUS", style::eyebrow()),
    ]);
    f.render_widget(Paragraph::new(header).style(style::body()), area);
}

fn render_rows(
    f: &mut Frame,
    area: Rect,
    entries: &[TerminalEntry],
    cursor: usize,
    focused_zone: bool,
) {
    let now = chrono::Utc::now();
    let visible_cursor = cursor.min(entries.len().saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        let row_focused = focused_zone && idx == visible_cursor;
        let pointer = if row_focused { "▸" } else { " " };
        let row_style = if row_focused {
            style::active()
        } else {
            style::body()
        };
        let status_text = status_label(entry, now);
        let status_style = status_style_for(entry, now);

        let heartbeat = relative_seconds(now, entry.last_heartbeat);

        lines.push(Line::from(vec![
            Span::styled(format!(" {pointer} "), row_style),
            Span::styled(format!("{:<14}", entry.uuid), style::id_code()),
            Span::styled(format!("{:<8}", entry.scope.to_string()), style::id_code()),
            Span::styled(format!("{:<22}", entry.agent_id), row_style),
            Span::styled(format!("{:<8}", entry.pid), style::muted()),
            Span::styled(format!("{:<10}", heartbeat), style::muted()),
            Span::styled(status_text, status_style),
        ]));
    }
    f.render_widget(Paragraph::new(lines).style(style::body()), area);
}

fn render_keys(f: &mut Frame, area: Rect) {
    let footer = Line::from(vec![
        Span::raw("  "),
        Span::styled("↑↓", style::id_code()),
        Span::styled(" cursor   ", style::muted()),
        Span::styled("Enter", style::id_code()),
        Span::styled(" open ticket   ", style::muted()),
        Span::styled("o", style::id_code()),
        Span::styled(" focus   ", style::muted()),
        Span::styled("K", style::id_code()),
        Span::styled(" kill (SIGINT)", style::muted()),
    ]);
    f.render_widget(Paragraph::new(footer).style(style::body()), area);
}

fn status_label(entry: &TerminalEntry, now: chrono::DateTime<chrono::Utc>) -> String {
    match (entry.status, entry.is_stale(now)) {
        (TerminalStatus::Active, true) => "STALE".to_string(),
        (s, _) => format!("{s:?}").to_uppercase(),
    }
}

fn status_style_for(
    entry: &TerminalEntry,
    now: chrono::DateTime<chrono::Utc>,
) -> ratatui::style::Style {
    match (entry.status, entry.is_stale(now)) {
        (TerminalStatus::Active, false) => style::status_in_progress(),
        (TerminalStatus::Active, true) => style::status_blocked(),
        (TerminalStatus::Closed, _) => style::status_done(),
        (TerminalStatus::Dead, _) => style::status_blocked(),
    }
}

fn relative_seconds(
    now: chrono::DateTime<chrono::Utc>,
    when: chrono::DateTime<chrono::Utc>,
) -> String {
    let elapsed = now.signed_duration_since(when).num_seconds().max(0);
    if elapsed < 60 {
        format!("{elapsed}s ago")
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else {
        format!("{}h ago", elapsed / 3600)
    }
}

fn invoke_focus(pm_dir: &Path, entry: &TerminalEntry) {
    let cfg = crate::launcher::load_config(pm_dir);
    let Some(focus) = crate::launcher::resolve_focus_command(&cfg) else {
        return;
    };
    let sub = crate::launcher::ScopeSubstitution {
        cmd: String::new(),
        uuid: entry.uuid.clone(),
        scope: entry.scope.to_string(),
        label: entry.label.clone(),
        cwd: pm_dir.display().to_string(),
    };
    let line = sub.apply(&focus);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let _ = std::process::Command::new(&shell)
        .arg("-c")
        .arg(&line)
        .status();
}

fn kill_terminal(entry: &TerminalEntry) {
    // Best effort: send SIGINT via libc on Unix; on platforms without
    // this glue we silently no-op. The user can still close the
    // terminal from their WM.
    #[cfg(unix)]
    {
        // SAFETY: `kill(2)` with a valid pid and signal is sound; the
        // pid was captured at spawn so the worst case is a no-op when
        // the process has already exited.
        unsafe {
            libc::kill(entry.pid as i32, libc::SIGINT);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = entry; // suppress unused-variable warning on Windows
    }
}
