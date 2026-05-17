//! SpaceCell Thunder visual language.
//!
//! Single source of truth for palette, glyphs, and ratatui [`Style`]
//! presets. Every TUI surface composes from these tokens; ad-hoc colours
//! at call sites are a review block.
//!
//! v0.3.2 palette refresh: pure-black canvas, Lightning cyan-blue for
//! eyebrows / ids / section labels, gold retained for active and
//! focused selections plus the SpaceCell Thunder wordmark accent. The
//! Lightning blue is lifted from the SpaceCell Lightning logo and gives
//! the cockpit a high-contrast neon read; gold marks "you are here".

use ratatui::style::{Color, Modifier, Style};

pub mod glyphs;

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

/// Primary canvas. True black. Reads as the absolute base behind every
/// pane, eyebrow, and glyph; the Lightning blue and gold accents do the
/// branded contrast work above it.
pub const BLACK: Color = Color::Rgb(0, 0, 0);

/// Subtle raised surface for active pane fills. One step lighter than
/// black so panes can differentiate without the eye registering navy.
pub const SURFACE_RAISED: Color = Color::Rgb(10, 12, 18);

/// One step lighter again, for deeply nested cards.
pub const SURFACE_RAISED_2: Color = Color::Rgb(18, 22, 32);

// ---------------------------------------------------------------------------
// Lightning palette
// ---------------------------------------------------------------------------

/// Lightning blue (primary accent). Eyebrows, ticket ids, section
/// labels, breadcrumb separators. Bright enough to read cleanly on the
/// black canvas.
pub const LIGHTNING_BLUE: Color = Color::Rgb(41, 182, 246);

/// Lightning blue, brighter. Hover / focus glow accent.
pub const LIGHTNING_BLUE_BRIGHT: Color = Color::Rgb(79, 195, 247);

/// Lightning blue, deeper. Subtle accents and borders that should sit
/// behind the brighter content without disappearing.
pub const LIGHTNING_BLUE_DEEP: Color = Color::Rgb(2, 136, 209);

// ---------------------------------------------------------------------------
// Paper (text)
// ---------------------------------------------------------------------------

/// Body text. Off-white from the brand paper colour.
pub const PAPER: Color = Color::Rgb(250, 250, 247);

/// Warm cream variant used for emphasis text on light surfaces (rare in
/// the black-canvas cockpit; retained for the materialised composed
/// view rendering).
pub const PAPER_WARM: Color = Color::Rgb(244, 241, 232);

// ---------------------------------------------------------------------------
// Gold accents
// ---------------------------------------------------------------------------

/// Bright gold. Active row, focused element, the active-card marker.
/// Reserved for "you are here / selected".
pub const GOLD: Color = Color::Rgb(240, 183, 38);

/// Gold, lighter. Hover and focus glow.
pub const GOLD5: Color = Color::Rgb(251, 208, 101);

/// Gold, lightest. Soft highlight band.
pub const GOLD7: Color = Color::Rgb(255, 234, 181);

/// Deep ochre. Reserved for inset accents on cream surfaces.
pub const GOLD3: Color = Color::Rgb(209, 152, 11);

/// Deepest ochre.
pub const GOLD1: Color = Color::Rgb(92, 68, 5);

// ---------------------------------------------------------------------------
// Semantic
// ---------------------------------------------------------------------------

/// Done / good / shipped. Brighter than the original SpaceCell green
/// so it reads on the black canvas.
pub const GREEN: Color = Color::Rgb(67, 173, 110);

/// In-progress healthy.
pub const GREEN_MID: Color = Color::Rgb(102, 187, 106);

/// Blocked / stale / risk. Brighter than the original crimson so it
/// pops on black.
pub const CRIMSON: Color = Color::Rgb(229, 57, 53);

/// Blocked, brighter for foreground text.
pub const CRIMSON_MID: Color = Color::Rgb(239, 83, 80);

// ---------------------------------------------------------------------------
// Muted neutrals
// ---------------------------------------------------------------------------

/// Muted metadata text. Reads as related-but-quieter rather than as a
/// different palette.
pub const MUTED: Color = Color::Rgb(120, 130, 145);

/// One step lighter than `MUTED`, used when `MUTED` looks too dark on
/// black.
pub const MUTED_2: Color = Color::Rgb(170, 180, 195);

// ---------------------------------------------------------------------------
// Composed styles
// ---------------------------------------------------------------------------

/// Plain body text on the canvas.
pub fn body() -> Style {
    Style::default().fg(PAPER).bg(BLACK)
}

/// Uppercase letter-spaced eyebrow labels (PROJECTS, ACTIVITY, etc.).
/// Lightning blue does the heavy accent work.
pub fn eyebrow() -> Style {
    Style::default()
        .fg(LIGHTNING_BLUE)
        .bg(BLACK)
        .add_modifier(Modifier::BOLD)
}

/// Ticket ids, code-like metadata. Lightning blue at body weight so it
/// reads as data rather than as a section label.
pub fn id_code() -> Style {
    Style::default().fg(LIGHTNING_BLUE).bg(BLACK)
}

/// Active row or selected card. Gold marks the current cursor focus.
pub fn active() -> Style {
    Style::default()
        .fg(GOLD)
        .bg(BLACK)
        .add_modifier(Modifier::BOLD)
}

/// Focus glow accent used on the active column header in the board.
pub fn focus_glow() -> Style {
    Style::default()
        .fg(GOLD5)
        .bg(BLACK)
        .add_modifier(Modifier::UNDERLINED)
}

/// Muted metadata (timestamps, helpers, dim secondary text).
pub fn muted() -> Style {
    Style::default().fg(MUTED).bg(BLACK)
}

/// Slightly brighter muted for cases where `MUTED` is unreadable.
pub fn muted_bright() -> Style {
    Style::default().fg(MUTED_2).bg(BLACK)
}

/// Semantic green for done/healthy.
pub fn status_done() -> Style {
    Style::default().fg(GREEN_MID).bg(BLACK)
}

/// Semantic gold for in-progress.
pub fn status_in_progress() -> Style {
    Style::default().fg(GOLD).bg(BLACK)
}

/// Semantic crimson for blocked/risk/stale.
pub fn status_blocked() -> Style {
    Style::default().fg(CRIMSON_MID).bg(BLACK)
}

/// Semantic muted for to-do (unstarted).
pub fn status_todo() -> Style {
    Style::default().fg(MUTED_2).bg(BLACK)
}

/// Border colour for panels. Deep Lightning blue so the border reads as
/// a structural rule rather than competing with content for attention.
/// Rounded corners come from
/// [`ratatui::widgets::BorderType::Rounded`].
pub fn border() -> Style {
    Style::default().fg(LIGHTNING_BLUE_DEEP).bg(BLACK)
}

/// SpaceCell wordmark accent. Used on its own for "SPACECELL" in the
/// header; "THUNDER" gets [`wordmark_accent`] to mirror the SpaceCell
/// Lightning logo treatment (white wordmark + accent product name).
pub fn wordmark() -> Style {
    Style::default()
        .fg(PAPER)
        .bg(BLACK)
        .add_modifier(Modifier::BOLD)
}

/// THUNDER half of the wordmark, plus other "this is the brand" lifts.
/// Gold retains the brand-book connection while Lightning blue carries
/// the rest of the cockpit accents.
pub fn wordmark_accent() -> Style {
    Style::default()
        .fg(GOLD)
        .bg(BLACK)
        .add_modifier(Modifier::BOLD)
}

/// The bolt glyph in the header, mirroring the Lightning logo bolt.
pub fn bolt() -> Style {
    Style::default()
        .fg(LIGHTNING_BLUE_BRIGHT)
        .bg(BLACK)
        .add_modifier(Modifier::BOLD)
}

// ---------------------------------------------------------------------------
// Per-kind accent colours (legacy compatibility for the kanban board)
// ---------------------------------------------------------------------------

/// The board (`tui::workflow`) previously coloured rows by ticket kind.
/// These map the v0.9 per-kind palette onto the v0.3.2 tokens so the
/// existing renderer can adopt them without restructuring.
pub fn kind_product() -> Color {
    LIGHTNING_BLUE
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
    LIGHTNING_BLUE_BRIGHT
}
