//! Agents surface: embedded PTY view for the focused ticket's agent.
//!
//! When the LHP scope points at a ticket that has a hosted agent
//! (spawned via `r` from the board or `spacecell run <id>`), this
//! surface renders the agent's vt100 screen inside its rect.
//! Keystrokes typed while the surface owns focus go straight to the
//! PTY master via [`crate::agents::Agent::write`]; mode-switch keys
//! stay reserved at the shell router.

use std::sync::Arc;
use std::sync::Mutex;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::agents::{Agent, AgentManager, AgentStatus};
use crate::store::LeafId;
use crate::style as theme;
use crate::tui::input::Disposition;
use shpool_vt100::{Color as VtColor, Parser};

pub struct AgentsState {
    /// True when the next keystrokes should be forwarded to the PTY
    /// rather than handled locally. Auto-set when the surface is
    /// focused on a live agent; `Esc` while focused exits input mode
    /// so the user can move the cursor without sending characters.
    pub input_mode: bool,
    /// The ticket whose agent the surface should render and route
    /// keys to. Set by `Shell::spawn_agent_and_switch` so the
    /// surface lookup does not depend on the LHP scope matching the
    /// spawn target (the user can be focused on PRJ1 in the LHP but
    /// press `r` on TSK7 in the board; both should work).
    pub active: Option<LeafId>,
}

impl AgentsState {
    pub fn new() -> Self {
        AgentsState {
            input_mode: true,
            active: None,
        }
    }

    /// Pin the surface to `leaf`. Called by the shell when spawning
    /// or switching focus to a different ticket's agent.
    pub fn set_active(&mut self, leaf: LeafId) {
        self.active = Some(leaf);
        self.input_mode = true;
    }

    /// Resolve which leaf the surface should display. Pinned active
    /// agent wins; otherwise the LHP scope (so navigating to a
    /// ticket with a live agent surfaces it without pressing keys);
    /// otherwise `None` (empty state).
    fn resolved(&self, scope: Option<LeafId>, agents: &AgentManager) -> Option<LeafId> {
        if let Some(a) = self.active {
            if agents.get(a).is_some() {
                return Some(a);
            }
        }
        if let Some(s) = scope {
            if agents.get(s).is_some() {
                return Some(s);
            }
        }
        None
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        agents: &mut AgentManager,
        scope: Option<LeafId>,
    ) -> Disposition {
        // Esc always exits input mode so the user can take focus back
        // without killing the PTY.
        if key.code == KeyCode::Esc {
            self.input_mode = false;
            return Disposition::Consumed;
        }

        // Tap `i` to re-enter input mode when out of it.
        if !self.input_mode && key.code == KeyCode::Char('i') && key.modifiers.is_empty() {
            self.input_mode = true;
            return Disposition::Consumed;
        }

        let Some(leaf) = self.resolved(scope, agents) else {
            return Disposition::Consumed;
        };

        // Out-of-input mode: arrow keys overflow back to the LHP /
        // sibling surfaces so the user can still navigate without the
        // PTY swallowing the arrows.
        if !self.input_mode {
            return match key.code {
                KeyCode::Left => Disposition::OverflowLeft,
                KeyCode::Right => Disposition::OverflowRight,
                _ => Disposition::Consumed,
            };
        }

        let Some(agent) = agents.get_mut(leaf) else {
            return Disposition::Consumed;
        };

        // Encode the keystroke and write to the PTY. Mode-switch keys
        // (Tab, 1/2/3, Shift-arrows) were already intercepted by the
        // shell router before this handler runs, so plain `Tab` here
        // never reaches the PTY; an explicit `Ctrl-i` is needed to
        // send a tab character (the same convention vim uses).
        let bytes = encode_key(key);
        if !bytes.is_empty() {
            agent.write(&bytes);
        }
        Disposition::Consumed
    }
}

impl Default for AgentsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the surface into `area`. Shows an empty state when the
/// focused scope has no agent; otherwise renders the PTY's current
/// screen contents and resizes the PTY to match the visible rect.
pub fn render(
    f: &mut Frame,
    area: Rect,
    agents: &mut AgentManager,
    scope: Option<LeafId>,
    state: &AgentsState,
    focused_zone: bool,
) {
    let resolved = state.resolved(scope, agents);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(if focused_zone {
            theme::border_focused()
        } else {
            theme::border()
        })
        .title(Line::styled(
            agent_title(agents, resolved),
            theme::eyebrow(),
        ))
        .style(theme::body());
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let Some(leaf) = resolved else {
        render_empty_state(
            f,
            inner,
            "No agent attached. Press r on a board card to spawn one.",
        );
        return;
    };

    let Some(agent) = agents.get_mut(leaf) else {
        render_empty_state(
            f,
            inner,
            "Agent went away. Press r on a board card to spawn a new one.",
        );
        return;
    };

    // Resize the PTY to match the visible inner rect on every render
    // so a window resize, mode switch, or split change keeps the
    // child's COLUMNS / LINES in sync.
    agent.resize(inner.height, inner.width);
    let parser = agent.parser();
    render_pty(f, inner, parser);

    // Status / input-mode hint at the bottom (over the rendered PTY
    // when the area is tight, otherwise as a thin overlay).
    render_status_line(f, area, agent, state.input_mode);
}

fn render_empty_state(f: &mut Frame, area: Rect, msg: &str) {
    let lines = vec![
        Line::from(""),
        Line::styled(format!("  {msg}"), theme::muted_bright()),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("r", theme::id_code()),
            Span::styled(" spawn agent   ", theme::muted()),
            Span::styled("Esc", theme::id_code()),
            Span::styled(" leave input mode   ", theme::muted()),
            Span::styled("i", theme::id_code()),
            Span::styled(" re-enter input mode", theme::muted()),
        ]),
    ];
    f.render_widget(Paragraph::new(lines).style(theme::body()), area);
}

fn render_pty(f: &mut Frame, area: Rect, parser: Arc<Mutex<Parser>>) {
    let parser = match parser.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    let screen = parser.screen();
    let (rows, cols) = screen.size();

    let usable_rows = std::cmp::min(rows as usize, area.height as usize);
    let usable_cols = std::cmp::min(cols as usize, area.width as usize);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(usable_rows);
    for row in 0..usable_rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_text = String::new();
        let mut current_style = Style::default();
        let mut first = true;
        for col in 0..usable_cols {
            let cell = match screen.cell(row as u16, col as u16) {
                Some(c) => c,
                None => continue,
            };
            let contents = cell.contents();
            let text = if contents.is_empty() {
                " ".to_string()
            } else {
                contents
            };
            let style = style_for_cell(cell);
            if first {
                current_style = style;
                first = false;
            }
            if style == current_style {
                current_text.push_str(&text);
            } else {
                spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    current_style,
                ));
                current_text = text;
                current_style = style;
            }
        }
        if !current_text.is_empty() {
            spans.push(Span::styled(current_text, current_style));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines).style(theme::body()), area);
}

fn render_status_line(f: &mut Frame, area: Rect, agent: &Agent, input_mode: bool) {
    // Anchor at the title row so it never overdraws PTY content.
    let strip = Rect {
        x: area.x + 2,
        y: area.y,
        width: area.width.saturating_sub(4),
        height: 1,
    };
    let mode_label = if input_mode { "INPUT" } else { "NAV" };
    let status_label = match agent.status {
        AgentStatus::Running => "RUNNING",
        AgentStatus::Exited => "EXITED",
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" [{status_label}]"),
            match agent.status {
                AgentStatus::Running => theme::status_in_progress(),
                AgentStatus::Exited => theme::status_done(),
            },
        ),
        Span::styled(format!(" [{mode_label}]"), theme::id_code()),
    ]);
    f.render_widget(Paragraph::new(line).style(theme::body()), strip);
}

fn agent_title(agents: &AgentManager, scope: Option<LeafId>) -> String {
    match scope.and_then(|l| agents.get(l).map(|a| (l, a))) {
        Some((leaf, agent)) => format!(" AGENT · {leaf} · {} ", agent.command),
        None => " AGENTS ".to_string(),
    }
}

fn style_for_cell(cell: &shpool_vt100::Cell) -> Style {
    let mut s = Style::default()
        .fg(vt_color_to_ratatui(cell.fgcolor(), theme::PAPER))
        .bg(vt_color_to_ratatui(cell.bgcolor(), theme::BLACK));
    let mut modifiers = Modifier::empty();
    if cell.bold() {
        modifiers |= Modifier::BOLD;
    }
    if cell.underline() {
        modifiers |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        modifiers |= Modifier::REVERSED;
    }
    if !modifiers.is_empty() {
        s = s.add_modifier(modifiers);
    }
    s
}

fn vt_color_to_ratatui(color: VtColor, fallback: Color) -> Color {
    match color {
        VtColor::Default => fallback,
        VtColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        VtColor::Idx(i) => indexed_color(i),
    }
}

fn indexed_color(i: u8) -> Color {
    // Standard 16-colour ANSI palette; >15 maps to the xterm 256-colour
    // table via ratatui's `Color::Indexed` so true-colour terminals
    // render the right entries.
    match i {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        15 => Color::White,
        n => Color::Indexed(n),
    }
}

/// Encode a crossterm key event into the bytes a PTY child expects.
/// Plain printable keys are sent verbatim; control keys map to the
/// usual ASCII control codes; arrow keys send the standard CSI
/// sequences.
fn encode_key(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            let mut bytes: Vec<u8> = Vec::new();
            if alt {
                bytes.push(0x1b);
            }
            if ctrl {
                // Map Ctrl-<letter> to the conventional ASCII control
                // byte. Falls back to the raw character for keys that
                // are not on the A-Z range.
                if c.is_ascii_alphabetic() {
                    let upper = c.to_ascii_uppercase() as u8;
                    bytes.push(upper - b'A' + 1);
                } else {
                    let mut buf = [0u8; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            } else {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            bytes
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => Vec::new(),
    }
}
