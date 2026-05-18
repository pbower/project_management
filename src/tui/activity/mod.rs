//! Activity strip: always-visible bottom band of the shell.
//!
//! Left side: rolling tail of `events.log` (verb-coloured rows).
//! Right side: state-change ticker showing the last few transitions.
//!
//! v0.3.4 replaces the v0.3.1 500ms poll with a `notify-debouncer-mini`
//! watcher on `events.log`. The strip becomes event-driven: the shell
//! drives [`ActivityStrip::poll`] every tick and that method drains the
//! watcher channel, refreshes the buffer when it fires, and returns
//! whether anything changed so the shell can reload `Database` for
//! out-of-band CLI mutations. A short fallback poll covers terminals
//! where the file-watcher driver is missing or quietly fails.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::style;
use crate::views::events_view::ActivityView;

pub mod ticker;
use ticker::StateChangeTicker;

/// Debounce window for `events.log` file events. Tight enough that the
/// strip reads "instant" to a human; loose enough that a burst of
/// appends from a script does not produce a re-read per line.
const WATCHER_DEBOUNCE: Duration = Duration::from_millis(120);

/// Fallback poll interval when the file-watcher could not be started
/// (rare; usually a sandboxed filesystem). Same cadence the v0.3.1
/// strip used.
const FALLBACK_POLL: Duration = Duration::from_millis(500);

/// One row of the activity strip. Owns the buffered [`ActivityView`],
/// the right-side [`StateChangeTicker`], and the watcher that drives
/// both.
pub struct ActivityStrip {
    view: ActivityView,
    ticker: StateChangeTicker,
    rx: Option<Receiver<()>>,
    // Holding the debouncer keeps the OS watcher thread alive; dropping
    // us drops it. Field is named with a leading underscore because the
    // value is never read directly; its lifetime is the contract.
    _debouncer: Option<Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>>,
    last_refresh: Instant,
}

impl ActivityStrip {
    pub fn new(pm_dir: PathBuf) -> Self {
        let mut view = ActivityView::new(pm_dir.clone());
        // Best-effort initial load. Any error surfaces in the hint
        // line on the next refresh.
        let _ = view.refresh();
        let mut ticker = StateChangeTicker::new();
        ticker.ingest(&view.events);

        let (rx, debouncer) = match start_watcher(&pm_dir) {
            Some((rx, dbnc)) => (Some(rx), Some(dbnc)),
            None => (None, None),
        };

        ActivityStrip {
            view,
            ticker,
            rx,
            _debouncer: debouncer,
            last_refresh: Instant::now(),
        }
    }

    /// Called by the shell on every tick. Drains the watcher channel,
    /// reads `events.log` again if anything fired or the fallback
    /// throttle elapsed, and returns `true` when the buffer actually
    /// changed since the last call so the shell knows to reload the
    /// `Database`.
    pub fn poll(&mut self) -> bool {
        let mut should_refresh = false;

        if let Some(rx) = self.rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(()) => should_refresh = true,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        // Watcher thread died; fall back to polling
                        // from here on so the strip keeps updating.
                        self.rx = None;
                        break;
                    }
                }
            }
        }

        if !should_refresh && self.last_refresh.elapsed() >= FALLBACK_POLL {
            should_refresh = true;
        }
        if !should_refresh {
            return false;
        }

        let prev_len = self.view.events.len();
        if let Err(e) = self.view.refresh() {
            self.view.hint = Some(format!("events.log: {e}"));
        }
        self.last_refresh = Instant::now();
        let len_changed = self.view.events.len() != prev_len;
        if len_changed {
            self.ticker.ingest(&self.view.events);
        }
        len_changed
    }

    /// Render the strip into `area`. Splits horizontally: events on
    /// the left (about two-thirds), state-change ticker on the right.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(style::border())
            .title(Line::styled(" ACTIVITY ", style::eyebrow()))
            .style(style::body());
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Carve out a right-hand strip for the ticker; events take the
        // rest. The ticker needs ~26 cells for `HH:MM:SS TSKnnn  ▸ VERB`
        // plus its own border; events use whatever remains.
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(34)])
            .split(inner);

        self.render_events(f, columns[0]);
        self.ticker.render(f, columns[1]);
    }

    fn render_events(&self, f: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1)])
            .split(area);

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

/// Start a debounced watcher on `events.log`. Returns `None` (and the
/// strip falls back to polling) when the watcher cannot be set up;
/// missing-file is one of those cases because the file is only created
/// the first time something writes to it.
fn start_watcher(
    pm_dir: &std::path::Path,
) -> Option<(
    Receiver<()>,
    Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
)> {
    let events_log = pm_dir.join("events.log");
    if !events_log.exists() {
        return None;
    }

    let (tx, rx) = channel::<()>();
    let mut debouncer = new_debouncer(WATCHER_DEBOUNCE, move |res: DebounceEventResult| {
        if res.is_ok() {
            // Coalesce: callers only care that something fired, not
            // how many events landed. A blocked send (receiver dropped)
            // is fine to ignore; the strip falls back to polling.
            let _ = tx.send(());
        }
    })
    .ok()?;

    debouncer
        .watcher()
        .watch(&events_log, RecursiveMode::NonRecursive)
        .ok()?;

    Some((rx, debouncer))
}
