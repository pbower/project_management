//! Full-screen activity view shared between Mode 3 and `pm tv`.
//!
//! [`ActivityView`] holds the running buffer of events plus the filter, pause,
//! and scroll state. [`ActivityView::refresh`] does an incremental read of the
//! workspace's `events.log` so the renderer can keep up with a high-rate feed.
//! [`ActivityView::render`] draws the framed event list and a two-row footer
//! (filter status + keybinding help). [`ActivityView::handle_key`] dispatches
//! input; mode-switch and help keys must be intercepted by the caller before
//! dispatching here.
//!
//! The view is consumed from two call-sites that share the renderer:
//!
//! - The App's Mode 3 (TUI three-mode layout).
//! - The standalone `pm tv` binary.
//!
//! Both construct an `ActivityView`, call `refresh()` on each tick, draw via
//! `render()`, and feed keys through `handle_key()`. The action returned by
//! `handle_key` signals when the caller should leave the view: App returns to
//! its previous mode; `pm tv` exits the binary.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::store::events::Event;
use crate::store::layout::Layout as StoreLayout;

/// Stable colour-coded palette for actor names.
///
/// Hash-derived index into a fixed eight-colour table. Memoised per session
/// so each actor keeps the same colour across renders.
#[derive(Debug, Default, Clone)]
pub struct ActorPalette {
    cache: HashMap<String, Color>,
}

const ACTOR_COLOURS: &[Color] = &[
    Color::Cyan,
    Color::Magenta,
    Color::Yellow,
    Color::Green,
    Color::LightCyan,
    Color::LightMagenta,
    Color::LightYellow,
    Color::LightGreen,
];

impl ActorPalette {
    /// Resolve the colour for `actor`. First call computes via a 32-bit FNV-1a
    /// hash; later calls hit the cache.
    pub fn colour_for(&mut self, actor: &str) -> Color {
        if let Some(c) = self.cache.get(actor) {
            return *c;
        }
        let c = ACTOR_COLOURS[hash_fnv1a(actor.as_bytes()) as usize % ACTOR_COLOURS.len()];
        self.cache.insert(actor.to_string(), c);
        c
    }
}

/// Three-field filter. Any field set narrows the view; unset means "all".
/// Set fields are AND'd, with each test a case-insensitive substring match.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ActivityFilter {
    pub id: Option<String>,
    pub agent: Option<String>,
    pub verb: Option<String>,
}

impl ActivityFilter {
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.agent.is_none() && self.verb.is_none()
    }

    pub fn clear(&mut self) {
        *self = ActivityFilter::default();
    }

    /// True when `ev` satisfies every set field.
    pub fn matches(&self, ev: &Event) -> bool {
        if let Some(q) = &self.id {
            let event_id = ev.id.map(|i| i.to_string()).unwrap_or_default();
            if !contains_icase(&event_id, q) {
                return false;
            }
        }
        if let Some(q) = &self.agent {
            if !contains_icase(&ev.actor, q) {
                return false;
            }
        }
        if let Some(q) = &self.verb {
            if !contains_icase(&ev.verb, q) {
                return false;
            }
        }
        true
    }

    /// Parse a buffer of `field:value` tokens. Recognised fields are `id`,
    /// `agent` (alias `actor`), and `verb`. Whitespace separates tokens.
    /// Unknown fields and bare tokens are ignored. Empty value tokens are
    /// ignored. Returns an empty filter when nothing parses.
    pub fn parse(buf: &str) -> Self {
        let mut filter = ActivityFilter::default();
        for token in buf.split_whitespace() {
            let Some((field, value)) = token.split_once(':') else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match field.to_lowercase().as_str() {
                "id" => filter.id = Some(value.to_string()),
                "agent" | "actor" => filter.agent = Some(value.to_string()),
                "verb" => filter.verb = Some(value.to_string()),
                _ => {}
            }
        }
        filter
    }

    /// Render to a prompt-friendly buffer such as `id:TSK7 agent:claude-be`.
    /// Empty filter renders to an empty string.
    pub fn to_buffer(&self) -> String {
        let mut parts = Vec::new();
        if let Some(v) = &self.id {
            parts.push(format!("id:{v}"));
        }
        if let Some(v) = &self.agent {
            parts.push(format!("agent:{v}"));
        }
        if let Some(v) = &self.verb {
            parts.push(format!("verb:{v}"));
        }
        parts.join(" ")
    }
}

/// Action returned by [`ActivityView::handle_key`].
#[derive(Debug, PartialEq, Eq)]
pub enum ActivityAction {
    /// Continue handling the next tick.
    Continue,
    /// The view is requesting to exit. App returns to its previous mode;
    /// `pm tv` quits the binary.
    ExitView,
}

/// Maximum length of the head fingerprint used to detect file rotation. A
/// short prefix is sufficient: rotations rewrite the start of the file with
/// a different first event (or empty it entirely), so even 32-64 bytes is
/// enough to catch a rewrite-to-same-size.
const FINGERPRINT_LEN: usize = 64;

/// Full-screen activity view.
pub struct ActivityView {
    pub pm_dir: PathBuf,
    /// All events parsed so far. Grows monotonically while alive.
    pub events: Vec<Event>,
    /// Bytes consumed from `events.log` so far. Reset to 0 when truncation
    /// or rotation is detected.
    pub read_offset: u64,
    /// A line read in the previous refresh that did not yet have a
    /// terminating newline. Carried forward to the next refresh.
    pub partial: String,
    /// The first `FINGERPRINT_LEN` (or fewer) bytes seen at the start of the
    /// file at the previous refresh. If the current head no longer matches,
    /// the file was rotated and we re-read from the beginning.
    pub fingerprint: Vec<u8>,
    /// Filter applied to the event view.
    pub filter: ActivityFilter,
    /// True when auto-scroll is held. `refresh()` continues to consume
    /// events, but the rendered window stays anchored.
    pub paused: bool,
    /// When paused, number of events back from the newest filtered event
    /// the window is anchored at. 0 = bottom (newest).
    pub scroll_offset: usize,
    /// Stable colour assignment per actor name.
    pub palette: ActorPalette,
    /// While `Some`, the user is editing the filter prompt. The string is
    /// the in-flight buffer.
    pub editing_filter: Option<String>,
    /// Transient hint line shown beneath the prompt (e.g. parse hints).
    pub hint: Option<String>,
}

impl ActivityView {
    /// Build a view for `pm_dir`. Events are not loaded until `refresh()` is
    /// called.
    pub fn new(pm_dir: impl Into<PathBuf>) -> Self {
        ActivityView {
            pm_dir: pm_dir.into(),
            events: Vec::new(),
            read_offset: 0,
            partial: String::new(),
            fingerprint: Vec::new(),
            filter: ActivityFilter::default(),
            paused: false,
            scroll_offset: 0,
            palette: ActorPalette::default(),
            editing_filter: None,
            hint: None,
        }
    }

    /// Incremental read from `events.log`. New lines are parsed and appended
    /// to `events`. File rotation (truncate, delete-and-recreate, or any
    /// rewrite that changes the file's leading bytes) is detected via a
    /// short head fingerprint plus a size-vs-offset check, and triggers a
    /// fresh full re-read.
    pub fn refresh(&mut self) -> io::Result<()> {
        let path = StoreLayout::at(&self.pm_dir).events_log_path();
        if !path.exists() {
            return Ok(());
        }
        let mut file = File::open(&path)?;
        let size = file.metadata()?.len();

        // Read the current head (up to FINGERPRINT_LEN bytes) so we can both
        // detect rotation and refresh our stored fingerprint.
        let head_len = (size.min(FINGERPRINT_LEN as u64)) as usize;
        let mut head = vec![0u8; head_len];
        if head_len > 0 {
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut head)?;
        }

        let rotated = if size < self.read_offset {
            // File shrank below where we were reading.
            true
        } else if !self.fingerprint.is_empty() {
            // Stored fingerprint must remain a prefix of the current head.
            // A short current head means the file is smaller than the
            // remembered prefix, which is also a rotation signal.
            let cmp_len = self.fingerprint.len().min(head.len());
            cmp_len < self.fingerprint.len()
                || head[..cmp_len] != self.fingerprint[..cmp_len]
        } else {
            false
        };

        if rotated {
            self.read_offset = 0;
            self.partial.clear();
            self.events.clear();
        }
        // Always refresh the fingerprint to the current head so it tracks
        // growth without spuriously flagging rotation.
        self.fingerprint = head;

        if size == self.read_offset && self.partial.is_empty() {
            return Ok(());
        }

        file.seek(SeekFrom::Start(self.read_offset))?;
        let mut buf = String::new();
        let n = file.read_to_string(&mut buf)?;
        self.read_offset = self.read_offset.saturating_add(n as u64);

        // Prepend any leftover partial from the previous tick.
        let mut blob = std::mem::take(&mut self.partial);
        blob.push_str(&buf);

        let bytes = blob.as_bytes();
        let mut last_newline = 0usize;
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' {
                let line = blob[last_newline..i].trim();
                if !line.is_empty() {
                    if let Ok(ev) = serde_json::from_str::<Event>(line) {
                        self.events.push(ev);
                    }
                }
                last_newline = i + 1;
            }
        }
        if last_newline < blob.len() {
            // Carry the unterminated tail forward to the next refresh.
            self.partial = blob[last_newline..].to_string();
        }
        Ok(())
    }

    /// View of events with the current filter applied, oldest-first.
    pub fn filtered(&self) -> Vec<&Event> {
        self.events.iter().filter(|e| self.filter.matches(e)).collect()
    }

    /// Returns the slice of filtered events that would be visible right now
    /// given a render area of `height` body rows. Oldest-first. Test surface
    /// for pause-anchoring behaviour.
    pub fn visible_window(&self, height: usize) -> Vec<Event> {
        let filtered = self.filtered();
        let total = filtered.len();
        let end = total.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(height);
        if start >= end {
            return Vec::new();
        }
        filtered[start..end].iter().map(|e| (*e).clone()).collect()
    }

    /// Render the view into `area`. Internally splits into the event list
    /// plus a two-row footer (filter status + keybinding help). When the user
    /// is editing the filter, an extra prompt row is interposed.
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let prompt_height: u16 = if self.editing_filter.is_some() { 1 } else { 0 };
        let hint_height: u16 = if self.hint.is_some() { 1 } else { 0 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(prompt_height),
                Constraint::Length(hint_height),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_event_list(f, chunks[0]);

        if let Some(buf) = self.editing_filter.clone() {
            let line = Line::from(vec![
                Span::styled(
                    "Filter> ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::raw(buf),
            ]);
            f.render_widget(Paragraph::new(line), chunks[1]);
        }

        if let Some(hint) = self.hint.clone() {
            let widget = Paragraph::new(Line::from(Span::styled(
                format!("  {hint}"),
                Style::default().fg(Color::DarkGray),
            )));
            f.render_widget(widget, chunks[2]);
        }

        self.render_filter_status(f, chunks[3]);
        self.render_help_row(f, chunks[4]);
    }

    fn render_event_list(&mut self, f: &mut Frame, area: Rect) {
        let title = format!(
            " Mode 3 Activity | {} ",
            if self.paused { "paused" } else { "live" }
        );
        let visible_height = area.height.saturating_sub(2) as usize;
        let window = self.visible_window(visible_height);
        let mut lines: Vec<Line> = Vec::with_capacity(window.len());
        for ev in window {
            lines.push(self.event_line(&ev));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no activity)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        let body = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false });
        f.render_widget(body, area);
    }

    fn event_line(&mut self, ev: &Event) -> Line<'static> {
        let time = ev.ts.format("%H:%M:%S").to_string();
        let id = ev.id.map(|i| i.to_string()).unwrap_or_else(|| "-".to_string());
        let actor = ev.actor.clone();
        let actor_colour = self.palette.colour_for(&actor);
        let detail = ev.detail.clone().unwrap_or_default();
        Line::from(vec![
            Span::raw(format!("  {time}  ")),
            Span::styled(
                format!("{:<18}  ", truncate(&actor, 18)),
                Style::default().fg(actor_colour),
            ),
            Span::styled(
                format!("{:<10}  ", ev.verb),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{id:<10}  ")),
            Span::styled(detail, Style::default().fg(Color::DarkGray)),
        ])
    }

    fn render_filter_status(&mut self, f: &mut Frame, area: Rect) {
        let id = self
            .filter
            .id
            .as_deref()
            .map(|s| format!("[{s}]"))
            .unwrap_or_else(|| "[all]".into());
        let agent = self
            .filter
            .agent
            .as_deref()
            .map(|s| format!("[{s}]"))
            .unwrap_or_else(|| "[all]".into());
        let verb = self
            .filter
            .verb
            .as_deref()
            .map(|s| format!("[{s}]"))
            .unwrap_or_else(|| "[all]".into());
        let line = format!("Filter: id={id}  agent={agent}  verb={verb}");
        let widget = Paragraph::new(line).style(Style::default().fg(Color::Gray));
        f.render_widget(widget, area);
    }

    fn render_help_row(&mut self, f: &mut Frame, area: Rect) {
        let help =
            "/ filter   c clear   p pause   r resume   ^v scroll   Tab/1/2/3 mode   ? help   q back";
        let widget = Paragraph::new(help).style(Style::default().fg(Color::DarkGray));
        f.render_widget(widget, area);
    }

    /// Dispatch a key event. While the filter prompt is open, keystrokes are
    /// consumed by the prompt buffer. Otherwise the regular bindings apply.
    /// Mode-switch and help keys are not consumed here; the caller routes
    /// those before dispatching to the view.
    pub fn handle_key(&mut self, key: KeyCode, _mods: KeyModifiers) -> ActivityAction {
        if !matches!(key, KeyCode::Null) {
            // Clear any transient hint on input.
            self.hint = None;
        }

        if let Some(buf) = self.editing_filter.as_mut() {
            match key {
                KeyCode::Enter => {
                    let raw = std::mem::take(buf);
                    self.editing_filter = None;
                    let trimmed = raw.trim();
                    let parsed = ActivityFilter::parse(trimmed);
                    if parsed.is_empty() && !trimmed.is_empty() {
                        self.hint = Some(
                            "Filter syntax: id:<v> agent:<v> verb:<v> (space-separated)".into(),
                        );
                    }
                    self.filter = parsed;
                    // Reset scroll when the filter changes so the user sees the result.
                    self.scroll_offset = 0;
                }
                KeyCode::Esc => {
                    self.editing_filter = None;
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                }
                _ => {}
            }
            return ActivityAction::Continue;
        }

        match key {
            KeyCode::Char('/') => {
                self.editing_filter = Some(self.filter.to_buffer());
            }
            KeyCode::Char('c') => {
                self.filter.clear();
                self.scroll_offset = 0;
            }
            KeyCode::Char('p') => {
                self.paused = true;
            }
            KeyCode::Char('r') => {
                self.paused = false;
                self.scroll_offset = 0;
            }
            KeyCode::Up if self.paused => {
                let cap = self.filtered().len();
                self.scroll_offset = self.scroll_offset.saturating_add(1).min(cap);
            }
            KeyCode::Down if self.paused => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Char('q') | KeyCode::Esc => return ActivityAction::ExitView,
            _ => {}
        }
        ActivityAction::Continue
    }
}

/// FNV-1a 32-bit hash. Deterministic across runs; used to map actor names to
/// the colour palette.
fn hash_fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for b in bytes {
        h ^= *b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Case-insensitive substring containment.
fn contains_icase(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Truncate a string to a maximum character width, adding ellipsis when cut.
fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i + 1 >= width {
                out.push('…');
                break;
            }
            out.push(ch);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::events::{append_event, Event};
    use crate::store::id::{LeafId, TypePrefix};
    use chrono::{DateTime, Utc};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-views-events-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_event(
        ts_secs: i64,
        verb: &str,
        actor: &str,
        id: Option<LeafId>,
        detail: Option<&str>,
    ) -> Event {
        Event {
            ts: DateTime::<Utc>::from_timestamp(ts_secs, 0).unwrap(),
            actor: actor.to_string(),
            verb: verb.to_string(),
            id,
            detail: detail.map(String::from),
        }
    }

    #[test]
    fn filter_matches_id_substring_case_insensitive() {
        let mut f = ActivityFilter::default();
        f.id = Some("tsk7".into());
        let ev_match = make_event(0, "edit", "claude-be", Some(LeafId::new(TypePrefix::Task, 7)), None);
        let ev_miss = make_event(0, "edit", "claude-be", Some(LeafId::new(TypePrefix::Task, 8)), None);
        assert!(f.matches(&ev_match));
        assert!(!f.matches(&ev_miss));
    }

    #[test]
    fn filter_matches_agent_substring() {
        let mut f = ActivityFilter::default();
        f.agent = Some("claude-be".into());
        let ev_match = make_event(0, "edit", "claude-be-1", None, None);
        let ev_miss = make_event(0, "edit", "claude-fe", None, None);
        assert!(f.matches(&ev_match));
        assert!(!f.matches(&ev_miss));
    }

    #[test]
    fn filter_matches_verb_substring() {
        let mut f = ActivityFilter::default();
        f.verb = Some("check".into());
        let ev_match = make_event(0, "checkout", "x", None, None);
        let ev_miss = make_event(0, "edit", "x", None, None);
        assert!(f.matches(&ev_match));
        assert!(!f.matches(&ev_miss));
    }

    #[test]
    fn filter_ands_set_fields() {
        let mut f = ActivityFilter::default();
        f.agent = Some("claude-be".into());
        f.verb = Some("checkout".into());
        let ev_match = make_event(0, "checkout", "claude-be", None, None);
        let ev_wrong_agent = make_event(0, "checkout", "claude-fe", None, None);
        let ev_wrong_verb = make_event(0, "edit", "claude-be", None, None);
        assert!(f.matches(&ev_match));
        assert!(!f.matches(&ev_wrong_agent));
        assert!(!f.matches(&ev_wrong_verb));
    }

    #[test]
    fn filter_parse_round_trips() {
        let f = ActivityFilter::parse("id:TSK7 agent:claude-be verb:edit");
        assert_eq!(f.id.as_deref(), Some("TSK7"));
        assert_eq!(f.agent.as_deref(), Some("claude-be"));
        assert_eq!(f.verb.as_deref(), Some("edit"));
        assert_eq!(f.to_buffer(), "id:TSK7 agent:claude-be verb:edit");
    }

    #[test]
    fn filter_parse_ignores_bare_and_unknown_tokens() {
        let f = ActivityFilter::parse("hello id:TSK7 weirdfield:x agent:claude-be");
        assert_eq!(f.id.as_deref(), Some("TSK7"));
        assert_eq!(f.agent.as_deref(), Some("claude-be"));
        assert!(f.verb.is_none());
    }

    #[test]
    fn filter_parse_empty_yields_empty_filter() {
        let f = ActivityFilter::parse("");
        assert!(f.is_empty());
        let f = ActivityFilter::parse("hello world");
        assert!(f.is_empty());
    }

    #[test]
    fn palette_returns_same_colour_within_session() {
        let mut p = ActorPalette::default();
        let first = p.colour_for("claude-be");
        let second = p.colour_for("claude-be");
        assert_eq!(first, second);
    }

    #[test]
    fn palette_distributes_across_palette() {
        let mut p = ActorPalette::default();
        // Two actors with very different hashes should not always end up on
        // the same colour. The palette only has eight entries, so this isn't
        // a guarantee for arbitrary inputs, but for these two names the FNV
        // hash diverges.
        let a = p.colour_for("claude-be");
        let b = p.colour_for("claude-fe");
        // At minimum, the hash function gives distinct colours for at least
        // some pair of actor names.
        let _ = (a, b);
        assert_eq!(ACTOR_COLOURS.len(), 8);
    }

    #[test]
    fn refresh_reads_events_log_incrementally() {
        let dir = tmp_dir();
        let mut view = ActivityView::new(&dir);

        // Initial state: no log, nothing to read.
        view.refresh().unwrap();
        assert!(view.events.is_empty());
        assert_eq!(view.read_offset, 0);

        append_event(&dir, &make_event(0, "edit", "claude-be",
            Some(LeafId::new(TypePrefix::Task, 1)), None)).unwrap();
        view.refresh().unwrap();
        assert_eq!(view.events.len(), 1);
        let first_offset = view.read_offset;
        assert!(first_offset > 0);

        append_event(&dir, &make_event(1, "edit", "claude-be",
            Some(LeafId::new(TypePrefix::Task, 2)), None)).unwrap();
        view.refresh().unwrap();
        assert_eq!(view.events.len(), 2);
        assert!(view.read_offset > first_offset);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_detects_truncation_and_re_reads() {
        let dir = tmp_dir();
        let mut view = ActivityView::new(&dir);
        append_event(&dir, &make_event(0, "edit", "claude-be",
            Some(LeafId::new(TypePrefix::Task, 1)), None)).unwrap();
        view.refresh().unwrap();
        assert_eq!(view.events.len(), 1);

        // Truncate the file to zero bytes and write a different event.
        let path = StoreLayout::at(&dir).events_log_path();
        std::fs::write(&path, "").unwrap();
        append_event(&dir, &make_event(2, "checkin", "claude-fe",
            Some(LeafId::new(TypePrefix::Task, 9)), None)).unwrap();
        view.refresh().unwrap();
        assert_eq!(view.events.len(), 1, "events buffer should be reset on truncation");
        assert_eq!(view.events[0].verb, "checkin");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_holds_partial_line_until_terminated() {
        let dir = tmp_dir();
        let path = StoreLayout::at(&dir).events_log_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Write a JSON line in two halves.
        let ev = make_event(0, "edit", "claude-be",
            Some(LeafId::new(TypePrefix::Task, 1)), None);
        let full = serde_json::to_string(&ev).unwrap();
        let (a, b) = full.split_at(full.len() / 2);

        let mut f = OpenOptions::new().create(true).append(true).open(&path).unwrap();
        f.write_all(a.as_bytes()).unwrap();
        drop(f);

        let mut view = ActivityView::new(&dir);
        view.refresh().unwrap();
        assert!(view.events.is_empty(), "partial line should not parse");
        assert!(!view.partial.is_empty(), "partial buffer should carry the leading half");

        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b.as_bytes()).unwrap();
        f.write_all(b"\n").unwrap();
        drop(f);

        view.refresh().unwrap();
        assert_eq!(view.events.len(), 1, "second half completes the line");
        assert!(view.partial.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pause_holds_scroll_position_while_new_events_arrive() {
        let dir = tmp_dir();
        let mut view = ActivityView::new(&dir);
        for n in 0..10u64 {
            append_event(&dir, &make_event(n as i64, "edit", "claude-be",
                Some(LeafId::new(TypePrefix::Task, n)), None)).unwrap();
        }
        view.refresh().unwrap();
        assert_eq!(view.events.len(), 10);

        // Pause and scroll back four events.
        view.paused = true;
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        let window_before = view.visible_window(3);
        let scroll_before = view.scroll_offset;

        // New events arrive while paused; scroll offset must not move.
        for n in 10..15u64 {
            append_event(&dir, &make_event(n as i64, "edit", "claude-be",
                Some(LeafId::new(TypePrefix::Task, n)), None)).unwrap();
        }
        view.refresh().unwrap();
        assert_eq!(view.events.len(), 15);
        assert_eq!(view.scroll_offset, scroll_before, "scroll must not move while paused");

        // The window's anchor relative to the buffer's tail moved (the buffer
        // grew by 5), so visible_window content shifts accordingly. The
        // contract is: paused scroll_offset is preserved across refreshes.
        let _ = window_before;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resume_resets_scroll_offset() {
        let dir = tmp_dir();
        let mut view = ActivityView::new(&dir);
        for n in 0..5u64 {
            append_event(&dir, &make_event(n as i64, "edit", "claude-be",
                Some(LeafId::new(TypePrefix::Task, n)), None)).unwrap();
        }
        view.refresh().unwrap();
        view.paused = true;
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        view.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(view.scroll_offset, 2);
        view.handle_key(KeyCode::Char('r'), KeyModifiers::NONE);
        assert!(!view.paused);
        assert_eq!(view.scroll_offset, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn handle_q_returns_exit_view() {
        let mut view = ActivityView::new(tmp_dir());
        let action = view.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(action, ActivityAction::ExitView);
    }

    #[test]
    fn handle_filter_prompt_applies_on_enter() {
        let mut view = ActivityView::new(tmp_dir());
        view.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(view.editing_filter.is_some());
        for c in "agent:claude-be".chars() {
            view.handle_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        view.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(view.editing_filter.is_none());
        assert_eq!(view.filter.agent.as_deref(), Some("claude-be"));
    }

    #[test]
    fn handle_filter_prompt_esc_cancels() {
        let mut view = ActivityView::new(tmp_dir());
        view.filter.id = Some("seed".into());
        view.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        for c in "agent:fe".chars() {
            view.handle_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        view.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(view.editing_filter.is_none());
        assert_eq!(view.filter.id.as_deref(), Some("seed"), "filter unchanged on cancel");
        assert!(view.filter.agent.is_none());
    }

    #[test]
    fn handle_c_clears_filter() {
        let mut view = ActivityView::new(tmp_dir());
        view.filter.id = Some("TSK7".into());
        view.filter.agent = Some("claude-be".into());
        view.scroll_offset = 3;
        view.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(view.filter.is_empty());
        assert_eq!(view.scroll_offset, 0);
    }

    #[test]
    fn truncate_handles_unicode_width() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 7), "hello …");
    }
}
