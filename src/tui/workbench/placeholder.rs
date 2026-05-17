//! Coming-soon placeholders for surfaces that v0.3.1 does not yet
//! populate. Replaced by real renderers in v0.3.3.

use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::style;

fn placeholder(f: &mut Frame, area: Rect, title: &str, blurb: &str) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(style::border())
        .title(Line::styled(format!(" {title} "), style::eyebrow()))
        .style(style::body());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::styled(blurb, style::muted_bright()).alignment(Alignment::Center),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tab", style::id_code()),
            Span::styled("  cycle modes      ", style::muted()),
            Span::styled("1 2 3", style::id_code()),
            Span::styled("  jump directly    ", style::muted()),
            Span::styled("?", style::id_code()),
            Span::styled("  help", style::muted()),
        ])
        .alignment(Alignment::Center),
    ];
    f.render_widget(Paragraph::new(lines).style(style::body()), inner);
}

pub fn render_documents(f: &mut Frame, area: Rect) {
    placeholder(
        f,
        area,
        "DOCUMENTS",
        "The Documents workspace lands in v0.3.3 (Workbench surfaces + splits).",
    );
}

pub fn render_activity_mode(f: &mut Frame, area: Rect) {
    placeholder(
        f,
        area,
        "ACTIVITY",
        "The full-screen Activity view lands in v0.3.3. The bottom strip is live now.",
    );
}
