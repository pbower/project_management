//! Glyph constants used across the SpaceCell Thunder TUI.
//!
//! Per PM_DESIGN section 8.7.4. Unicode geometric symbols only; no Nerd
//! Font dependency. Pasted glyphs render in any modern terminal with a
//! decent Unicode font.

/// Brand mark. Used in the wordmark and as a generic "this is a
/// SpaceCell context" indicator.
pub const BOLT: char = '⚡';

/// Project marker (empty). Hexagons read as "structural container" without
/// implying status.
pub const HEX_EMPTY: char = '⬡';

/// Project marker (active/filled).
pub const HEX_FILLED: char = '⬢';

/// Status: done.
pub const DONE: char = '✓';

/// Status: in-progress.
pub const IN_PROGRESS: char = '●';

/// Status: blocked.
pub const BLOCKED: char = '⊘';

/// Status: to-do (open, not started).
pub const TODO: char = '○';

/// Status: stale (lock past TTL, etc.).
pub const STALE: char = '⚠';

/// Active row / focused selection marker.
pub const POINTER: char = '▸';

/// Left-edge accent on a card with an attached terminal.
pub const TERMINAL_ACCENT: char = '▌';

/// Breadcrumb separator (between hierarchy levels).
pub const BREADCRUMB: char = '›';

/// Branch indicator (git).
pub const BRANCH: char = '⎇';

/// Event verb glyphs.
pub const EVENT_STARTED: char = '▷';
pub const EVENT_DONE: char = '✓';
pub const EVENT_INFO: char = 'ⓘ';
pub const EVENT_WARN: char = '⚠';
