//! Workbench: the central work surface of the shell.
//!
//! v0.3.6 wires three live surfaces (Board, Memories, Terminals) and
//! a Templates browser. Documents / Activity placeholders are gone;
//! the modes now map onto the surfaces directly.

use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::db::Database;
use crate::store::LeafId;

use crate::tui::input::{Disposition, Mode};

pub mod board;
pub mod memories;
pub mod templates;
pub mod terminals;

/// Which surface the Workbench is currently showing. v0.3.6 populates
/// all four. Templates is not wired to a numeric mode yet; v0.3.7 may
/// add a dedicated mode or keep it as a key-triggered overlay.
#[derive(Clone, Copy, Debug)]
pub enum Surface {
    Board,
    Memories,
    Terminals,
    Templates,
}

impl Surface {
    /// Map a [`Mode`] onto its default surface.
    pub fn for_mode(mode: Mode) -> Surface {
        match mode {
            Mode::Board => Surface::Board,
            Mode::Memories => Surface::Memories,
            Mode::Terminals => Surface::Terminals,
        }
    }
}

/// Per-surface state retained across renders. Each surface owns the
/// cursor / scroll state it needs to survive mode switches.
#[derive(Default)]
pub struct WorkbenchState {
    pub board: board::BoardState,
    pub memories: memories::MemoriesState,
    pub terminals: terminals::TerminalsState,
    pub templates: templates::TemplatesState,
    /// `true` when the Workbench is temporarily showing the Templates
    /// surface (toggled with `t`). Reverts to the mode-default surface
    /// the next time the user presses a mode-switch or `t` again.
    pub templates_visible: bool,
}

impl WorkbenchState {
    pub fn new() -> Self {
        WorkbenchState {
            board: board::BoardState::new(),
            memories: memories::MemoriesState::new(),
            terminals: terminals::TerminalsState::new(),
            templates: templates::TemplatesState::new(),
            templates_visible: false,
        }
    }

    /// Toggle the Templates surface overlay.
    pub fn toggle_templates(&mut self) {
        self.templates_visible = !self.templates_visible;
    }

    /// Dispatch a keystroke into the surface that matches `mode` (or
    /// the Templates surface when overlay is active). Returns the
    /// surface's [`Disposition`] so the shell knows when to flip focus
    /// or open the ticket editor.
    pub fn handle_key(
        &mut self,
        mode: Mode,
        key: KeyCode,
        mods: KeyModifiers,
        db: &Database,
        pm_dir: &Path,
        scope: Option<LeafId>,
    ) -> Disposition {
        // 't' toggles the templates overlay from any surface. Routed
        // here rather than in the input router because it shadows
        // surface-local 't' bindings, and only the Workbench owns the
        // surface state to mutate.
        if matches!(key, KeyCode::Char('t')) && mods.is_empty() {
            self.toggle_templates();
            return Disposition::Consumed;
        }

        if self.templates_visible {
            return self.templates.handle_key(key, mods, pm_dir);
        }

        match Surface::for_mode(mode) {
            Surface::Board => self.board.handle_key(key, mods, db),
            Surface::Memories => self.memories.handle_key(key, mods, db, pm_dir, scope),
            Surface::Terminals => self.terminals.handle_key(key, mods, pm_dir),
            Surface::Templates => self.templates.handle_key(key, mods, pm_dir),
        }
    }

    /// Render the workbench for the active `mode` (or the Templates
    /// overlay) into `area`, filtered to `scope`.
    pub fn render(
        &self,
        f: &mut Frame,
        area: Rect,
        db: &Database,
        pm_dir: &Path,
        scope: Option<LeafId>,
        mode: Mode,
        focused_zone: bool,
    ) {
        let surface = if self.templates_visible {
            Surface::Templates
        } else {
            Surface::for_mode(mode)
        };

        match surface {
            Surface::Board => board::render(f, area, db, scope, &self.board, focused_zone),
            Surface::Memories => {
                memories::render(f, area, db, pm_dir, scope, &self.memories, focused_zone)
            }
            Surface::Terminals => terminals::render(f, area, pm_dir, &self.terminals, focused_zone),
            Surface::Templates => templates::render(f, area, pm_dir, &self.templates, focused_zone),
        }
    }
}
