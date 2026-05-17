//! SpaceCell Thunder visual language.
//!
//! Single source of truth for palette, glyphs, and ratatui [`Style`]
//! presets. Every TUI surface composes from these tokens; ad-hoc colours
//! at call sites are a review block.
//!
//! Tokens mirror the SpaceCell brand book and PM_DESIGN section 8.7.

use ratatui::style::{Color, Modifier, Style};

pub mod glyphs;

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// Primary canvas. Near-black navy that anchors every panel background.
pub const NAVY_DEEP: Color = Color::Rgb(0, 14, 40);

/// Active pane fill, hovered row, secondary surface.
pub const NAVY: Color = Color::Rgb(0, 23, 65);

/// One step lighter than `NAVY` for nested fills.
pub const NAVY_MID: Color = Color::Rgb(0, 32, 91);

/// Subtle rule and divider colour. Use sparingly; full saturation reads
/// loud against navy_deep.
pub const NAVY_LIGHT: Color = Color::Rgb(1, 50, 140);

/// Primary text. Off-white from the brand paper colour.
pub const PAPER: Color = Color::Rgb(250, 250, 247);

/// Warm cream variant of paper used for hover and secondary text on
/// light surfaces (rare in the dark TUI).
pub const PAPER_WARM: Color = Color::Rgb(244, 241, 232);

/// **Mono/code text colour** per the brand book. This is the colour for
/// ids, eyebrows, code-like metadata, and command output throughout the
/// TUI. The brand book pins it explicitly.
pub const GOLD3: Color = Color::Rgb(209, 152, 11);

/// Bright accent. Active row, focused element, the active-card marker.
pub const GOLD: Color = Color::Rgb(240, 183, 38);

/// Hover and focus glow, lighter than `GOLD`.
pub const GOLD5: Color = Color::Rgb(251, 208, 101);

/// Deepest ochre, used for muted accents on cream surfaces.
pub const GOLD1: Color = Color::Rgb(92, 68, 5);

/// Semantic: done / good / shipped.
pub const GREEN: Color = Color::Rgb(20, 73, 44);

/// Semantic: in-progress healthy. Slightly brighter than `GREEN`.
pub const GREEN_MID: Color = Color::Rgb(26, 93, 57);

/// Semantic: blocked / stale / risk.
pub const CRIMSON: Color = Color::Rgb(107, 29, 36);

/// Semantic: blocked, brighter for foreground text.
pub const CRIMSON_MID: Color = Color::Rgb(138, 39, 48);

/// Muted metadata text. Dim greys lifted from the brand neutrals so they
/// read as related-but-quieter rather than as a different palette.
pub const MUTED: Color = Color::Rgb(107, 112, 128);

/// One step lighter than `MUTED`, used when `MUTED` looks too dark on
/// `NAVY_DEEP`.
pub const MUTED_2: Color = Color::Rgb(154, 160, 176);

// ---------------------------------------------------------------------------
// Composed Styles
// ---------------------------------------------------------------------------

/// Plain body text on the navy canvas.
pub fn body() -> Style {
    Style::default().fg(PAPER).bg(NAVY_DEEP)
}

/// Uppercase letter-spaced eyebrow labels (PROJECTS, ACTIVITY, etc.).
/// Mirrors `--type-eyebrow` in the brand book.
pub fn eyebrow() -> Style {
    Style::default()
        .fg(GOLD3)
        .bg(NAVY_DEEP)
        .add_modifier(Modifier::BOLD)
}

/// Ticket ids, code-like metadata.
pub fn id_code() -> Style {
    Style::default().fg(GOLD3).bg(NAVY_DEEP)
}

/// Active row or selected card.
pub fn active() -> Style {
    Style::default()
        .fg(GOLD)
        .bg(NAVY_DEEP)
        .add_modifier(Modifier::BOLD)
}

/// Focus glow accent used on the active column header in the board.
pub fn focus_glow() -> Style {
    Style::default()
        .fg(GOLD5)
        .bg(NAVY_DEEP)
        .add_modifier(Modifier::UNDERLINED)
}

/// Muted metadata (timestamps, helpers, dim secondary text).
pub fn muted() -> Style {
    Style::default().fg(MUTED).bg(NAVY_DEEP)
}

/// Slightly brighter muted for cases where `MUTED` is unreadable.
pub fn muted_bright() -> Style {
    Style::default().fg(MUTED_2).bg(NAVY_DEEP)
}

/// Semantic green for done/healthy.
pub fn status_done() -> Style {
    Style::default().fg(GREEN_MID).bg(NAVY_DEEP)
}

/// Semantic gold for in-progress.
pub fn status_in_progress() -> Style {
    Style::default().fg(GOLD).bg(NAVY_DEEP)
}

/// Semantic crimson for blocked/risk/stale.
pub fn status_blocked() -> Style {
    Style::default().fg(CRIMSON_MID).bg(NAVY_DEEP)
}

/// Semantic muted for to-do (unstarted).
pub fn status_todo() -> Style {
    Style::default().fg(MUTED_2).bg(NAVY_DEEP)
}

/// Border colour for panels (rounded corners come from
/// [`ratatui::widgets::BorderType::Rounded`]).
pub fn border() -> Style {
    Style::default().fg(NAVY_LIGHT).bg(NAVY_DEEP)
}

/// Brand wordmark style. Gold on navy, bold.
pub fn wordmark() -> Style {
    Style::default()
        .fg(GOLD)
        .bg(NAVY_DEEP)
        .add_modifier(Modifier::BOLD)
}

// ---------------------------------------------------------------------------
// Per-kind accent colours (legacy compatibility for the kanban board)
// ---------------------------------------------------------------------------

/// The board (`tui::workflow`) previously coloured rows by ticket kind.
/// These map the v0.9 per-kind palette onto the SpaceCell tokens so the
/// existing renderer can adopt them without restructuring.
pub fn kind_product() -> Color {
    NAVY_LIGHT
}

pub fn kind_epic() -> Color {
    GREEN_MID
}

pub fn kind_task() -> Color {
    GOLD
}

pub fn kind_subtask() -> Color {
    CRIMSON_MID
}

pub fn kind_milestone() -> Color {
    GOLD5
}

pub fn kind_project() -> Color {
    GOLD3
}
