//! Modal help overlay reachable from every mode via `?` or `F1`.
//!
//! v0.3.1 ships a single-page reference. v0.3.x phases extend it with
//! per-mode sections, concept refresher, and workflow recipes as the
//! cockpit grows.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::style;
use crate::tui::input::Mode;
use crate::tui::utils::centered_rect;

pub fn render(f: &mut Frame, area: Rect, mode: Mode) {
    let rect = centered_rect(70, 70, area);
    f.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(style::border())
        .title(Line::styled(" HELP ", style::wordmark()))
        .style(style::body());
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("  SPACECELL ", style::wordmark()),
        Span::styled("THUNDER", style::wordmark_accent()),
        Span::styled(" · v0.3.2", style::muted()),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::styled("  GLOBAL", style::eyebrow()));
    lines.push(row("Tab / Shift-Tab", "Cycle modes"));
    lines.push(row("1 / 2 / 3", "Jump to Board / Memories / Agents"));
    lines.push(row(
        "Shift + ← / →",
        "Jump focus between LHP and Workbench from anywhere",
    ));
    lines.push(row("t", "Toggle the Templates overlay (per-kind configs)"));
    lines.push(row("? / F1", "Open / close this help"));
    lines.push(row("q / Ctrl-C", "Quit"));
    lines.push(Line::raw(""));

    lines.push(Line::styled("  NAVIGATION · LHP", style::eyebrow()));
    lines.push(row("↑ / ↓", "Move within the focused level (no skipping)"));
    lines.push(row("→ / Enter", "Drill into the selected item's children"));
    lines.push(row("←", "Drill back to the parent level"));
    lines.push(row("→  at Subtask", "Hand focus to the board"));
    lines.push(Line::raw(""));

    lines.push(Line::styled("  NAVIGATION · BOARD", style::eyebrow()));
    lines.push(row("← / →", "Move between columns"));
    lines.push(row("↑ / ↓", "Move between cards in the focused column"));
    lines.push(row("Enter", "Open the focused card in $EDITOR"));
    lines.push(row("←  at column 1", "Hand focus back to the LHP"));
    lines.push(Line::raw(""));

    lines.push(Line::styled(
        "  NAVIGATION · MEMORIES (mode 2)",
        style::eyebrow(),
    ));
    lines.push(row("↑ / ↓", "Move between memory files"));
    lines.push(row("Enter", "Open the focused memory in $EDITOR"));
    lines.push(Line::raw(""));

    lines.push(Line::styled(
        "  NAVIGATION · AGENTS (mode 3)",
        style::eyebrow(),
    ));
    lines.push(row(
        "r (on board)",
        "Spawn an embedded agent on the focused card",
    ));
    lines.push(row("(typing)", "Keystrokes flow to the agent's PTY"));
    lines.push(row("Esc", "Leave input mode (PTY stops receiving keys)"));
    lines.push(row("i", "Re-enter input mode"));
    lines.push(blurb(
        "Embedded agents run in-cockpit; their stdout/stderr renders inside the Agents surface and their stdin is the keystrokes you type while focused.",
    ));
    lines.push(blurb(
        "External-window agents (launcher-spawned) are still available via `spacecell terminals` on the CLI.",
    ));
    lines.push(Line::raw(""));

    lines.push(Line::styled("  TEMPLATES OVERLAY", style::eyebrow()));
    lines.push(row("t", "Open / close the templates overlay"));
    lines.push(row(
        "Enter",
        "Open the selected template (or launcher) in $EDITOR",
    ));
    lines.push(Line::raw(""));

    lines.push(Line::styled("  CONCEPTS", style::eyebrow()));
    lines.push(blurb(
        "Hierarchy: PROJECT > PRODUCT > EPIC > TASK > SUBTASK, plus MILESTONE as a cross-cutting marker.",
    ));
    lines.push(blurb(
        "Workspace storage lives in .pm/ next to your code. Every state mutation is a git commit.",
    ));
    lines.push(blurb(
        "Memories: ~/.claude/projects/*/memory/ (user, never committed), .pm/projects/<PRJ>/memories/ (project, committed), and per-ticket memories alongside CLAUDE.md.",
    ));
    lines.push(Line::raw(""));

    lines.push(Line::styled(
        format!("  Current mode: {}", mode.label()),
        style::muted(),
    ));

    f.render_widget(Paragraph::new(lines).style(style::body()), inner);
}

fn row(keys: &'static str, label: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("    {keys:<18}"), style::id_code()),
        Span::styled(label, style::body()),
    ])
}

fn blurb(text: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(text, style::muted_bright()),
    ])
}
