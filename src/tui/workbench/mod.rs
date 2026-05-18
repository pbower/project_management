//! Workbench: the central work surface of the shell.
//!
//! v0.3.7 hosts agent PTYs in-cockpit. The Workbench owns one
//! [`crate::agents::AgentManager`] so the main shell can drive every
//! live agent's reader thread from a single tick. The Agents surface
//! renders the focused ticket's PTY; the legacy "list of external
//! terminals" view from v0.3.6 retired because the embedded model
//! covers the same role with less indirection (the registry-style
//! list is still reachable via `spacecell terminals` on the CLI).

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::agents::AgentManager;
use crate::db::Database;
use crate::store::LeafId;

use crate::tui::input::{Disposition, Mode};

pub mod agents;
pub mod board;
pub mod memories;
pub mod templates;

/// Which surface the Workbench is currently showing.
#[derive(Clone, Copy, Debug)]
pub enum Surface {
    Board,
    Memories,
    Agents,
    Templates,
}

impl Surface {
    pub fn for_mode(mode: Mode) -> Surface {
        match mode {
            Mode::Board => Surface::Board,
            Mode::Memories => Surface::Memories,
            Mode::Agents => Surface::Agents,
        }
    }
}

/// Per-surface state retained across renders, plus the manager that
/// owns every hosted agent's PTY for the life of the shell.
pub struct WorkbenchState {
    pub board: board::BoardState,
    pub memories: memories::MemoriesState,
    pub agents_state: agents::AgentsState,
    pub templates: templates::TemplatesState,
    pub manager: AgentManager,
    /// `true` when the Templates overlay is visible. Toggled with `t`.
    pub templates_visible: bool,
}

impl WorkbenchState {
    pub fn new() -> Self {
        WorkbenchState {
            board: board::BoardState::new(),
            memories: memories::MemoriesState::new(),
            agents_state: agents::AgentsState::new(),
            templates: templates::TemplatesState::new(),
            manager: AgentManager::new(),
            templates_visible: false,
        }
    }

    pub fn toggle_templates(&mut self) {
        self.templates_visible = !self.templates_visible;
    }

    /// Drain every hosted agent's reader thread. Called by the shell
    /// on every tick. Returns `true` when at least one agent's
    /// screen changed so the shell can decide to re-render.
    pub fn poll_agents(&mut self) -> bool {
        !self.manager.poll_all().is_empty()
    }

    /// Spawn an embedded agent for `leaf`. The label is used in the
    /// surface title and the spawn event detail.
    pub fn spawn_agent(
        &mut self,
        leaf: LeafId,
        command: &str,
        cwd: Option<&Path>,
        label: &str,
    ) -> Result<(), crate::agents::AgentError> {
        let env = crate::agents::scope_env(leaf, label);
        self.manager.spawn(leaf, command, cwd, &env)
    }

    /// Dispatch a keystroke into the surface that matches `mode` (or
    /// the Templates overlay when active).
    pub fn handle_key(
        &mut self,
        mode: Mode,
        key: KeyEvent,
        db: &Database,
        pm_dir: &Path,
        scope: Option<LeafId>,
    ) -> Disposition {
        // 't' (no modifiers) toggles the templates overlay, but only
        // from non-Agents surfaces - the Agents surface forwards 't'
        // to the PTY because the user almost certainly wants to type
        // it. The overlay is still reachable from Board / Memories.
        if !self.templates_visible
            && matches!(key.code, KeyCode::Char('t'))
            && key.modifiers.is_empty()
            && !matches!(Surface::for_mode(mode), Surface::Agents)
        {
            self.toggle_templates();
            return Disposition::Consumed;
        }

        // Once the overlay is up, 't' (or Esc) closes it from any
        // surface so the user is never trapped.
        if self.templates_visible
            && (matches!(key.code, KeyCode::Char('t')) || matches!(key.code, KeyCode::Esc))
        {
            self.toggle_templates();
            return Disposition::Consumed;
        }

        if self.templates_visible {
            return self.templates.handle_key(key.code, key.modifiers, pm_dir);
        }

        match Surface::for_mode(mode) {
            Surface::Board => self.board.handle_key(key.code, key.modifiers, db),
            Surface::Memories => {
                self.memories
                    .handle_key(key.code, key.modifiers, db, pm_dir, scope)
            }
            Surface::Agents => self.agents_state.handle_key(key, &mut self.manager, scope),
            Surface::Templates => self.templates.handle_key(key.code, key.modifiers, pm_dir),
        }
    }

    pub fn render(
        &mut self,
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
            Surface::Agents => agents::render(
                f,
                area,
                &mut self.manager,
                scope,
                &self.agents_state,
                focused_zone,
            ),
            Surface::Templates => templates::render(f, area, pm_dir, &self.templates, focused_zone),
        }
    }
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self::new()
    }
}
