//! Workbench: the central work surface of the shell.
//!
//! v0.3.1 implements the Board surface only; Documents and Activity
//! placeholders render a coming-soon paragraph. v0.3.3 fleshes out all
//! six surface types and adds vertical/horizontal splits.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::db::Database;
use crate::store::LeafId;

use crate::tui::input::{Disposition, Mode};

pub mod board;
mod placeholder;

/// Which surface the Workbench is currently showing. v0.3.1 has only the
/// Board surface populated; Documents and Activity are stubs that the
/// later phases replace.
#[derive(Clone, Copy, Debug)]
pub enum Surface {
    Board,
    Documents,
    Activity,
}

impl Surface {
    /// Map a [`Mode`] onto its default surface. Once splits land in
    /// v0.3.3 a Mode may host several surfaces simultaneously; for
    /// v0.3.1 the mapping is one-to-one.
    pub fn for_mode(mode: Mode) -> Surface {
        match mode {
            Mode::Board => Surface::Board,
            Mode::Documents => Surface::Documents,
            Mode::Activity => Surface::Activity,
        }
    }
}

/// Per-surface state retained across renders. The Board's column scroll
/// and card focus live here so they survive mode switches.
#[derive(Default)]
pub struct WorkbenchState {
    pub board: board::BoardState,
}

impl WorkbenchState {
    pub fn new() -> Self {
        WorkbenchState {
            board: board::BoardState::new(),
        }
    }

    /// Dispatch a keystroke into the surface that matches `mode`.
    /// Forwards the surface's [`Disposition`] up to the shell so
    /// arrow-key overflows can flip focus back to the LHP.
    pub fn handle_key(
        &mut self,
        mode: Mode,
        key: KeyCode,
        mods: KeyModifiers,
        db: &Database,
    ) -> Disposition {
        match Surface::for_mode(mode) {
            Surface::Board => self.board.handle_key(key, mods, db),
            Surface::Documents | Surface::Activity => Disposition::Consumed,
        }
    }

    /// Render the workbench for the active `mode` into `area`, filtered
    /// to `scope` (the LHP's currently selected subtree).
    pub fn render(
        &self,
        f: &mut Frame,
        area: Rect,
        db: &Database,
        scope: Option<LeafId>,
        mode: Mode,
        focused_zone: bool,
    ) {
        match Surface::for_mode(mode) {
            Surface::Board => board::render(f, area, db, scope, &self.board, focused_zone),
            Surface::Documents => placeholder::render_documents(f, area),
            Surface::Activity => placeholder::render_activity_mode(f, area),
        }
    }
}
