//! State-change ticker.
//!
//! Sits on the right side of the bottom activity strip. Surfaces the
//! last handful of ticket state transitions (status / priority /
//! complete / move) so the human at the cockpit sees the workflow
//! moving even when the focused mode is on something else.
//!
//! Reads from the same parsed event buffer the LHS uses, so the
//! ticker stays byte-for-byte consistent with the main feed and
//! costs nothing extra to keep current.

use std::collections::VecDeque;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::store::events::Event;
use crate::style;

/// How many transitions the ticker keeps in its rolling window. Sized
/// to fill the strip's right column at the default 5-row strip height
/// without scroll overflow.
const TICKER_CAPACITY: usize = 8;

/// One transition rendered as a single row. We keep the original
/// event around for the verb glyph and any future detail-string
/// rendering; v0.3.4 only uses ts + id + verb.
#[derive(Clone)]
struct Transition {
    event: Event,
}

/// State-change ticker. Owns its own rolling window of transitions
/// derived from the upstream event buffer. Re-built from scratch on
/// every ingest so out-of-order events from concurrent agents land
/// in the right place.
pub struct StateChangeTicker {
    window: VecDeque<Transition>,
}

impl StateChangeTicker {
    pub fn new() -> Self {
        StateChangeTicker {
            window: VecDeque::with_capacity(TICKER_CAPACITY),
        }
    }

    /// Rebuild the window from the full event buffer. Cheap because
    /// the buffer is already in memory and we cap at the last few
    /// matching events.
    pub fn ingest(&mut self, events: &[Event]) {
        // Walk the buffer newest-first so the most recent state-change
        // ends up at `front()` after the push_back sequence finishes;
        // push_front would invert the order on every insert and put
        // the oldest kept event at the front.
        self.window.clear();
        for ev in events.iter().rev() {
            if !is_state_change(&ev.verb) {
                continue;
            }
            self.window.push_back(Transition { event: ev.clone() });
            if self.window.len() >= TICKER_CAPACITY {
                break;
            }
        }
    }

    /// Render the ticker into `area`. Wraps itself in a labelled
    /// border so the shell can hand over the rect without further
    /// chrome.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(style::border())
            .title(Line::styled(" STATE CHANGES ", style::eyebrow()))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let row = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)])
            .split(inner);

        let height = row[0].height as usize;
        let lines: Vec<Line> = self
            .window
            .iter()
            .take(height)
            .map(|t| {
                let ts_short = t.event.ts.format("%H:%M:%S").to_string();
                let id_label = t
                    .event
                    .id
                    .as_ref()
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "—".to_string());
                let verb_upper = t.event.verb.to_uppercase();
                Line::from(vec![
                    Span::styled(format!("  {ts_short} "), style::muted()),
                    Span::styled(format!("{id_label:<8}"), style::id_code()),
                    Span::raw(" "),
                    Span::styled("▸ ", style::muted_bright()),
                    Span::styled(verb_upper, verb_style(&t.event.verb)),
                ])
            })
            .collect();

        f.render_widget(Paragraph::new(lines).style(style::body()), row[0]);
    }
}

impl Default for StateChangeTicker {
    fn default() -> Self {
        Self::new()
    }
}

/// Verbs that count as "ticket moved between states". The activity
/// feed shows everything; the ticker shows only the moves.
fn is_state_change(verb: &str) -> bool {
    matches!(
        verb,
        "status" | "priority" | "complete" | "move" | "reopen" | "checkin"
    )
}

fn verb_style(verb: &str) -> ratatui::style::Style {
    match verb {
        "complete" | "checkin" => style::status_done(),
        "status" | "move" | "priority" => style::status_in_progress(),
        "reopen" => style::status_todo(),
        _ => style::muted_bright(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::{LeafId, TypePrefix};
    use chrono::Utc;

    fn ev(verb: &str, leaf: Option<LeafId>) -> Event {
        Event {
            ts: Utc::now(),
            actor: "test".into(),
            verb: verb.into(),
            id: leaf,
            detail: None,
            scope: None,
        }
    }

    #[test]
    fn ingest_keeps_only_state_changes_and_caps_capacity() {
        let leaf = LeafId::new(TypePrefix::Task, 1);
        let mut events: Vec<Event> = Vec::new();
        // Mix of state-change and non-state-change verbs; ten of each.
        for i in 0..10 {
            let leaf_i = LeafId::new(TypePrefix::Task, i);
            events.push(ev("status", Some(leaf_i)));
            events.push(ev("edit", Some(leaf_i)));
        }
        events.push(ev("complete", Some(leaf)));

        let mut t = StateChangeTicker::new();
        t.ingest(&events);

        assert_eq!(t.window.len(), TICKER_CAPACITY);
        // Every retained event is a state-change verb.
        for tr in t.window.iter() {
            assert!(
                is_state_change(&tr.event.verb),
                "non-state-change leaked: {}",
                tr.event.verb
            );
        }
        // Most recent appears first.
        assert_eq!(
            t.window.front().map(|t| t.event.verb.as_str()),
            Some("complete")
        );
    }

    #[test]
    fn is_state_change_recognises_the_six_verbs() {
        for verb in [
            "status", "priority", "complete", "move", "reopen", "checkin",
        ] {
            assert!(is_state_change(verb), "{verb} should count");
        }
        for verb in [
            "edit",
            "init",
            "checkout",
            "warning",
            "stale",
            "artifact-add",
        ] {
            assert!(!is_state_change(verb), "{verb} should not count");
        }
    }
}
