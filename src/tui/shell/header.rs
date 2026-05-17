//! Top header: brand wordmark on the left, mode badge in the centre,
//! breadcrumb of the LHP's current selection chain on the right.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::db::{format_kind, Database};
use crate::style;
use crate::tui::input::Mode;
use crate::tui::lhp::LhpState;
use crate::tui::nav::{ancestor_chain, Level};

pub fn render(f: &mut Frame, area: Rect, mode: Mode, lhp: &LhpState, db: &Database) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(style::border())
        .style(style::body());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22),
            Constraint::Min(20),
            Constraint::Length(18),
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  ⚡ SPACECELL THUNDER",
            style::wordmark(),
        )]))
        .style(style::body()),
        split[0],
    );

    f.render_widget(
        Paragraph::new(breadcrumb(lhp, db)).style(style::body()),
        split[1],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("MODE  ", style::eyebrow()),
            Span::styled(mode.label(), style::active()),
        ]))
        .style(style::body()),
        split[2],
    );
}

fn breadcrumb<'a>(lhp: &LhpState, db: &'a Database) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled("  ", style::body()));

    // Walk from the LHP scope up to root and render levels left-to-right.
    let scope = lhp.scope(db);
    if let Some(leaf) = scope {
        let chain = ancestor_chain(db, leaf);
        for (idx, id) in chain.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::styled(" › ", style::muted()));
            }
            let task = match db.get(*id) {
                Some(t) => t,
                None => continue,
            };
            let kind_label = format_kind(task.kind).to_uppercase();
            spans.push(Span::styled(kind_label, style::eyebrow()));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(id.to_string(), style::id_code()));
        }
    } else {
        spans.push(Span::styled(Level::Project.label_upper(), style::eyebrow()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled("(empty workspace)", style::muted()));
    }
    Line::from(spans)
}
