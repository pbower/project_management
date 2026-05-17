//! The SpaceCell Thunder main shell.
//!
//! Composes three zones described in PM_DESIGN section 8.3.1:
//!
//! - `Header` (top): brand wordmark and breadcrumb.
//! - `LHP` (left): hierarchical navigation, salvaged from the v0.9
//!   `App` and rewritten against [`crate::tui::nav`].
//! - `Workbench` (centre): the active surface for the current mode
//!   ([`Mode::Board`] in v0.3.1; [`Mode::Documents`] and
//!   [`Mode::Activity`] are placeholders until v0.3.3).
//! - `Activity` (bottom strip): always-visible tail of `events.log`.
//! - `Footer` (bottom row): context-sensitive key hints.
//!
//! v0.3.1 wires the structure end-to-end with a poll-based event loop.
//! v0.3.2 replaces the polling with a `notify`-driven channel so the
//! board re-renders on every state-mutating commit.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::cmd::cmd_edit;
use crate::db::Database;
use crate::store::LeafId;
use crate::style;
use crate::tui::activity::ActivityStrip;
use crate::tui::input::{route, Disposition, Focus, Mode, ShellAction};
use crate::tui::lhp::LhpState;
use crate::tui::workbench::WorkbenchState;

mod footer;
mod header;
mod help;

/// What `Shell::apply` reports back to the run loop after each
/// keystroke. `Continue` keeps the loop running; `Quit` exits; `Edit`
/// suspends the TUI so `$EDITOR` can take over the terminal for the
/// duration of the edit and then resumes.
enum ShellOutcome {
    Continue,
    Quit,
    Edit(LeafId),
}

/// The main shell state. Re-loads the database on every refresh tick so
/// out-of-band mutations from concurrent CLI calls become visible without
/// needing inter-process notification (notify-debouncer-mini is added in
/// v0.3.2).
pub struct Shell {
    pm_dir: PathBuf,
    db: Database,
    mode: Mode,
    focus: Focus,
    lhp: LhpState,
    workbench: WorkbenchState,
    activity: ActivityStrip,
    show_help: bool,
}

impl Shell {
    pub fn new(pm_dir: &Path) -> Self {
        let db = Database::load(pm_dir);
        let activity = ActivityStrip::new(pm_dir.to_path_buf());
        Shell {
            pm_dir: pm_dir.to_path_buf(),
            db,
            mode: Mode::Board,
            focus: Focus::Workbench,
            lhp: LhpState::new(),
            workbench: WorkbenchState::new(),
            activity,
            show_help: false,
        }
    }

    /// Drive the shell against `terminal`. Returns on quit. Edit
    /// requests from the workbench short-circuit the inner loop, exit
    /// the alternate screen so `$EDITOR` can take over the tty, then
    /// re-enter and continue.
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        loop {
            self.activity.refresh_if_due();
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        continue;
                    }
                    let action = route(key.code, key.modifiers, self.focus, self.show_help);
                    match self.apply(key, action) {
                        ShellOutcome::Continue => {}
                        ShellOutcome::Quit => return Ok(()),
                        ShellOutcome::Edit(id) => {
                            self.run_editor(terminal, id)?;
                        }
                    }
                }
            }
        }
    }

    /// Suspend the alternate screen, hand the terminal back to `$EDITOR`
    /// for the given ticket, then resume and re-load the database so
    /// any front-matter changes the user made show up immediately.
    fn run_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        id: LeafId,
    ) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        cmd_edit(&self.pm_dir, &id.to_string(), None);

        enable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;
        terminal.clear()?;

        // Reload state so any front-matter or section changes the user
        // made in $EDITOR are reflected in the next render.
        self.db = Database::load(&self.pm_dir);
        Ok(())
    }

    /// Apply a routed action and report what the run loop should do
    /// next ([`ShellOutcome::Continue`], `Quit`, or `Edit`).
    fn apply(&mut self, key: crossterm::event::KeyEvent, action: ShellAction) -> ShellOutcome {
        match action {
            ShellAction::Quit => ShellOutcome::Quit,
            ShellAction::ToggleHelp => {
                self.show_help = !self.show_help;
                ShellOutcome::Continue
            }
            ShellAction::SwitchMode(_) => {
                // Tab / Shift-Tab resolve to a relative cycle; jump
                // keys use the absolute mode. The router returns
                // `SwitchMode(Mode::Board)` for both cycle and key 1,
                // so disambiguate by looking at the raw key here.
                match key.code {
                    crossterm::event::KeyCode::Tab => self.mode = self.mode.next(),
                    crossterm::event::KeyCode::BackTab => self.mode = self.mode.prev(),
                    crossterm::event::KeyCode::Char('1') => self.mode = Mode::Board,
                    crossterm::event::KeyCode::Char('2') => self.mode = Mode::Documents,
                    crossterm::event::KeyCode::Char('3') => self.mode = Mode::Activity,
                    _ => {}
                }
                self.show_help = false;
                ShellOutcome::Continue
            }
            ShellAction::FocusZone(focus) => {
                self.focus = focus;
                ShellOutcome::Continue
            }
            ShellAction::LhpKey(k, m) => {
                match self.lhp.handle_key(k, m, &self.db) {
                    Disposition::Consumed => {
                        // Re-load the DB so the board sees fresh task
                        // state when the user has navigated to a new
                        // scope.
                        self.db = Database::load(&self.pm_dir);
                    }
                    Disposition::OverflowRight => {
                        // Right from the deepest LHP level hands focus
                        // to the Workbench so the user can keep arrowing
                        // into the board's cards.
                        self.focus = Focus::Workbench;
                    }
                    Disposition::OverflowLeft => {
                        // Already at the leftmost LHP level; nothing to
                        // do until v0.3.4 adds a left-of-LHP surface.
                    }
                    Disposition::Edit(id) => return ShellOutcome::Edit(id),
                }
                ShellOutcome::Continue
            }
            ShellAction::WorkbenchKey(k, m) => {
                match self.workbench.handle_key(self.mode, k, m, &self.db) {
                    Disposition::Consumed => {}
                    Disposition::OverflowLeft => {
                        // Left from the leftmost board column hands
                        // focus back to the LHP.
                        self.focus = Focus::Lhp;
                    }
                    Disposition::OverflowRight => {
                        // Right from the rightmost column: stay put.
                        // v0.3.4 adds a right-of-Workbench surface.
                    }
                    Disposition::Edit(id) => return ShellOutcome::Edit(id),
                }
                ShellOutcome::Continue
            }
            ShellAction::None => ShellOutcome::Continue,
        }
    }

    fn render(&self, f: &mut ratatui::Frame) {
        let area = f.area();

        // Hard responsive minimum: refuse to draw if the terminal is
        // narrower than 80 cols, per PM_DESIGN section 8.7.5.
        if area.width < 80 || area.height < 20 {
            let msg = format!(
                "SpaceCell Thunder requires at least 80x20 (current: {}x{}). \
                 Please resize and try again.",
                area.width, area.height
            );
            let para = ratatui::widgets::Paragraph::new(msg).style(style::body());
            f.render_widget(para, area);
            return;
        }

        // Top header (3 rows) + body + bottom activity strip (5 rows)
        // + footer (1 row).
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(5),
                Constraint::Length(1),
            ])
            .split(area);

        header::render(f, rows[0], self.mode, &self.lhp, &self.db);

        // LHP + Workbench split horizontally inside the body row.
        let lhp_width = (area.width / 5).clamp(22, 36);
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(lhp_width), Constraint::Min(40)])
            .split(rows[1]);

        self.lhp
            .render(f, body[0], &self.db, self.focus == Focus::Lhp);
        self.workbench.render(
            f,
            body[1],
            &self.db,
            self.lhp.scope(&self.db),
            self.mode,
            self.focus == Focus::Workbench,
        );

        self.activity.render(f, rows[2]);
        footer::render(f, rows[3], self.mode, self.focus);

        if self.show_help {
            help::render(f, area, self.mode);
        }
    }
}

/// Entry point used by `spacecell` (no args). Sets up the alternate
/// screen, drives [`Shell::run`], and tears down on exit.
pub fn run_shell(pm_dir: &Path) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut shell = Shell::new(pm_dir);
    let result = shell.run(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}
