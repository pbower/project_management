//! Modal help overlay - mode-independent, scrollable, layered over the
//! current mode. PM_DESIGN.md Section 8.3.5.

use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::enums::{Mode, Overlay};
use crate::tui::utils::centered_rect;

use super::App;

impl App {
    /// Handle a keystroke while the help overlay is open. `?`, `Esc`, `h`,
    /// and `F1` close it; `Up`/`Down` scroll. Mode-switch keys are handled
    /// before this is reached, so they close help and switch in one stroke.
    pub(super) fn handle_help_overlay_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => {
                self.overlay = Overlay::None;
            }
            KeyCode::Up => {
                if let Overlay::Help { scroll } = &mut self.overlay {
                    *scroll = scroll.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Overlay::Help { scroll } = &mut self.overlay {
                    *scroll = scroll.saturating_add(1);
                }
            }
            _ => {}
        }
    }

    /// Render the modal help overlay. Mode-independent: drawn over whatever
    /// the current mode produced. Scrollable with Up/Down; closed with `?`,
    /// `Esc`, `h`, or `F1`. Layout follows PM_DESIGN.md Section 8.3.5 -
    /// current-mode keybindings first, then a concepts panel, then workflows.
    pub(super) fn render_help(&mut self, f: &mut Frame, area: Rect) {
        let heading = |text: &str| {
            Line::from(vec![Span::styled(
                text.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )])
        };

        let mut lines: Vec<Line> = Vec::new();
        lines.push(heading(&format!("{} - keybindings", self.mode.label())));
        match self.mode {
            Mode::Tickets => {
                lines.push(Line::from("  <- ->        Traverse hierarchy levels"));
                lines.push(Line::from("  ^ v          Move within the list"));
                lines.push(Line::from("  Enter        Drill into the selected ticket"));
                lines.push(Line::from("  e            Open the ticket's CLAUDE.md in $EDITOR"));
                lines.push(Line::from("  f            Open the quick-entry form"));
                lines.push(Line::from("  n            Add a child ticket"));
                lines.push(Line::from("  c / i        Checkout / checkin the selected ticket"));
                lines.push(Line::from("  a            Add an artifact"));
                lines.push(Line::from("  m            Toggle the memory side-panel"));
                lines.push(Line::from("  d            Delete the selected ticket"));
                lines.push(Line::from("  s            Cycle status   p   cycle process stage"));
                lines.push(Line::from("  t            Toggle show/hide completed   r refresh"));
                lines.push(Line::from("  /            Filter by title / tags / project"));
            }
            Mode::Documents => {
                lines.push(Line::from("  Document Workspace arrives in Phase 8."));
                lines.push(Line::from("  q            Exit to the launcher"));
            }
            Mode::Activity => {
                lines.push(Line::from("  Activity View arrives in Phase 9."));
                lines.push(Line::from("  q            Exit to the launcher"));
            }
        }
        lines.push(Line::from("  Tab / S-Tab  Cycle modes      1 / 2 / 3  jump to a mode"));
        lines.push(Line::from("  ? / F1       Toggle this help   q  back to launcher"));
        lines.push(Line::from(""));

        lines.push(heading("Concepts"));
        lines.push(Line::from("  Hierarchy    PRJ Project > PRD Product > EPC Epic > TSK Task > SBT Subtask"));
        lines.push(Line::from("  MLS          Milestone - a cross-cutting marker, project-scoped by default"));
        lines.push(Line::from("  Locks        A checkout claims a ticket; the Lock column shows the holder,"));
        lines.push(Line::from("               or STALE once the heartbeat TTL has passed"));
        lines.push(Line::from("  Memories     Three tiers - user, project, ticket. M:n counts linked refs"));
        lines.push(Line::from("  Composition  A ticket's CLAUDE.md carries front-matter plus prose sections"));
        lines.push(Line::from("  Git          Every state change commits; checkin squashes the checkout span"));
        lines.push(Line::from(""));

        lines.push(heading("Workflows"));
        lines.push(Line::from("  File a task            n, fill the quick-entry form, save"));
        lines.push(Line::from("  Hand off to an agent   c to checkout, share the ticket id, i to checkin"));
        lines.push(Line::from("  Monitor parallel work  Mode 3 (Phase 9) or `pm tv` on a second screen"));
        lines.push(Line::from("  Write a project memory `pm memory write --scope project ...` (Phase 10)"));

        let overlay = centered_rect(80, 80, area);
        f.render_widget(Clear, overlay);
        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Help - ^v scroll, ? or Esc to close"),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.help_scroll(), 0));
        f.render_widget(paragraph, overlay);
    }

    /// Current help-overlay scroll offset. Returns 0 when the overlay is not
    /// the help variant; callers should only invoke `render_help` while it is.
    pub(super) fn help_scroll(&self) -> u16 {
        match self.overlay {
            Overlay::Help { scroll } => scroll,
            _ => 0,
        }
    }
}
