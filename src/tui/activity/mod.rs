//! Activity strip: always-visible 3-line tail of `events.log` at the
//! bottom of the shell. Re-uses the buffer-parsing logic from
//! [`crate::views::events_view`] so the bottom strip and the full-screen
//! Activity view stay byte-identical.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::style;
use crate::views::events_view::ActivityView;

/// How often the strip will go back to `events.log`. Live refresh in
/// v0.3.2 wires a `notify` watcher and shortens this; v0.3.1 polls.
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

/// The bottom strip. Owns its own buffered [`ActivityView`] so the main
/// shell can render it without re-parsing events.log on every tick.
pub struct ActivityStrip {
    view: ActivityView,
    last_refresh: Instant,
}

impl ActivityStrip {
    pub fn new(pm_dir: PathBuf) -> Self {
        let mut view = ActivityView::new(pm_dir);
        // Best-effort initial load. Any error surfaces in the hint
        // line on the next refresh.
        let _ = view.refresh();
        ActivityStrip {
            view,
            last_refresh: Instant::now(),
        }
    }

    /// Re-read `events.log` if the throttle window has elapsed.
    pub fn refresh_if_due(&mut self) {
        if self.last_refresh.elapsed() < REFRESH_INTERVAL {
            return;
        }
        if let Err(e) = self.view.refresh() {
            self.view.hint = Some(format!("events.log: {e}"));
        }
        self.last_refresh = Instant::now();
    }

    /// Render the strip into `area`. Reserves the top row for an
    /// eyebrow label and renders the last (height - 1) events below.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(style::border())
            .title(Line::styled(" ACTIVITY ", style::eyebrow()))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)])
            .split(inner);

        let height = rows[0].height as usize;
        let events = self.view.visible_window(height);
        let lines: Vec<Line> = events
            .iter()
            .rev()
            .take(height)
            .rev()
            .map(|ev| {
                let ts_short = ev.ts.format("%H:%M:%S").to_string();
                let actor = ev.actor.as_str();
                let id_label = ev.id.as_ref().map(|i| i.to_string()).unwrap_or_default();
                let detail = ev.detail.as_deref().unwrap_or("");
                Line::from(vec![
                    Span::styled(format!("  {ts_short} "), style::muted()),
                    Span::styled(format!("{actor:<14} "), style::id_code()),
                    Span::styled(
                        format!("{:<10} ", ev.verb.to_uppercase()),
                        verb_style(&ev.verb),
                    ),
                    Span::styled(format!("{id_label:<10}"), style::id_code()),
                    Span::raw(" "),
                    Span::styled(detail.to_string(), style::body()),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines).style(style::body()), rows[0]);
    }
}

fn verb_style(verb: &str) -> ratatui::style::Style {
    match verb {
        "checkin" | "complete" | "edit" => style::status_done(),
        "checkout" | "status" | "move" => style::status_in_progress(),
        "warning" | "stale" => style::status_blocked(),
        _ => style::muted_bright(),
    }
}
