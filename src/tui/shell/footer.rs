//! Bottom row: context-sensitive key hints. Updates when the focused
//! zone or active mode changes.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::style;
use crate::tui::input::{Focus, Mode};

pub fn render(f: &mut Frame, area: Rect, mode: Mode, focus: Focus) {
    let key = |k: &'static str, label: &'static str| -> Vec<Span<'static>> {
        vec![
            Span::styled(k, style::id_code()),
            Span::raw(" "),
            Span::styled(label, style::muted()),
            Span::raw("   "),
        ]
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled("  ", style::body()));
    spans.extend(key("Tab", "mode"));
    spans.extend(key("1 2 3", "jump"));
    spans.extend(key("[ ]", "focus"));

    match (focus, mode) {
        (Focus::Lhp, _) => {
            spans.extend(key("← →", "level"));
            spans.extend(key("↑ ↓", "item"));
        }
        (Focus::Workbench, Mode::Board) => {
            spans.extend(key("H L", "column"));
            spans.extend(key("J K", "card"));
        }
        (Focus::Workbench, _) => {}
    }

    spans.extend(key("?", "help"));
    spans.extend(key("q", "quit"));

    f.render_widget(Paragraph::new(Line::from(spans)).style(style::body()), area);
}
