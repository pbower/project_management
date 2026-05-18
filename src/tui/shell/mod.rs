//! The SpaceCell Thunder main shell.
//!
//! Composes three zones described in PM_DESIGN section 8.3.1:
//!
//! - `Header` (top): brand wordmark and breadcrumb.
//! - `LHP` (left): hierarchical navigation, salvaged from the v0.9
//!   `App` and rewritten against [`crate::tui::nav`].
//! - `Workbench` (centre): the active surface for the current mode
//!   ([`Mode::Board`] in v0.3.1; [`Mode::Memories`] and
//!   [`Mode::Terminals`] are placeholders until v0.3.3).
//! - `Activity` (bottom strip): always-visible tail of `events.log`.
//! - `Footer` (bottom row): context-sensitive key hints.
//!
//! v0.3.1 wires the structure end-to-end with a poll-based event loop.
//! v0.3.2 replaces the polling with a `notify`-driven channel so the
//! board re-renders on every state-mutating commit.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
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
use crate::tui::ticket_editor::{
    sections::{cleanup_temp, extract_body, read_temp, splice_back, write_temp_for_section},
    EditorOutcome, TicketEditor,
};
use crate::tui::workbench::WorkbenchState;

mod footer;
mod header;
mod help;

/// What `Shell::apply` reports back to the run loop after each
/// keystroke. `Continue` keeps the loop running; `Quit` exits;
/// `EditRaw` suspends the TUI for raw `$EDITOR` on the ticket's
/// CLAUDE.md; `OpenEditor` swaps in the full-screen ticket editor.
enum ShellOutcome {
    Continue,
    Quit,
    EditRaw(LeafId),
    OpenEditor(LeafId),
    /// Suspend the alternate screen and hand `$EDITOR` an arbitrary
    /// file path (memory file, template, etc.).
    EditPath(std::path::PathBuf),
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
    /// Full-screen ticket editor. When `Some`, the editor owns the
    /// screen and the LHP / Workbench / Activity layout is hidden.
    editor: Option<TicketEditor>,
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
            editor: None,
        }
    }

    /// Drive the shell against `terminal`. Returns on quit. Edit
    /// requests from the workbench short-circuit the inner loop, exit
    /// the alternate screen so `$EDITOR` can take over the tty, then
    /// re-enter and continue.
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        loop {
            // Drain the activity strip's watcher. When events.log has
            // grown since the last tick the shell reloads Database so
            // out-of-band CLI mutations propagate into the LHP counts
            // and the board's content without the user needing to
            // press anything.
            if self.activity.poll() {
                self.db = Database::load(&self.pm_dir);
            }
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        continue;
                    }
                    // If the full-screen ticket editor is open it
                    // owns every keystroke until it returns Save or
                    // Cancel; the shell never sees mode-switch keys
                    // until then.
                    if self.editor.is_some() {
                        self.tick_editor(terminal, key.code, key.modifiers)?;
                        continue;
                    }
                    let action = route(key.code, key.modifiers, self.focus, self.show_help);
                    match self.apply(key, action) {
                        ShellOutcome::Continue => {}
                        ShellOutcome::Quit => return Ok(()),
                        ShellOutcome::EditRaw(id) => {
                            self.run_raw_editor(terminal, id)?;
                        }
                        ShellOutcome::OpenEditor(id) => {
                            self.open_ticket_editor(id);
                        }
                        ShellOutcome::EditPath(path) => {
                            self.run_external_editor(terminal, &path)?;
                        }
                    }
                }
            }
        }
    }

    fn open_ticket_editor(&mut self, id: LeafId) {
        match TicketEditor::open(self.pm_dir.clone(), &self.db, id) {
            Ok(editor) => self.editor = Some(editor),
            Err(e) => eprintln!("editor open: {e}"),
        }
    }

    /// Hand a keystroke to the open editor and react to its outcome.
    fn tick_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key: KeyCode,
        mods: KeyModifiers,
    ) -> io::Result<()> {
        let outcome = match self.editor.as_mut() {
            Some(ed) => ed.handle_key(key, mods),
            None => return Ok(()),
        };
        match outcome {
            EditorOutcome::Continue => Ok(()),
            EditorOutcome::Cancel => {
                self.editor = None;
                self.db = Database::load(&self.pm_dir);
                Ok(())
            }
            EditorOutcome::Save => {
                if let Some(ed) = self.editor.as_ref() {
                    if let Err(e) = ed.save(&mut self.db) {
                        eprintln!("editor save: {e}");
                    }
                }
                self.editor = None;
                self.db = Database::load(&self.pm_dir);
                Ok(())
            }
            EditorOutcome::EditSection { ticket, section } => {
                self.edit_section_externally(terminal, ticket, section)?;
                Ok(())
            }
        }
    }

    /// Drop out of the alternate screen, hand the temp section file
    /// to `$EDITOR`, splice the body back into CLAUDE.md, then resume.
    fn edit_section_externally(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        ticket: LeafId,
        section: crate::tui::ticket_editor::sections::Section,
    ) -> io::Result<()> {
        // Resolve the CLAUDE.md path for this ticket via the editor's
        // own knowledge if it is still open; otherwise fall back to
        // the state-json lookup.
        let claude_path = match self.editor.as_ref() {
            Some(ed) if ed.ticket_id() == ticket => ed.claude_path().clone(),
            _ => match self.db.state.items.get(&ticket) {
                Some(entry) => self
                    .pm_dir
                    .join(&entry.path)
                    .join(crate::store::claude_md::CLAUDE_MD),
                None => return Ok(()),
            },
        };

        // Extract the section body to a temp file with no headers,
        // front-matter, or import line. The user sees only their
        // prose.
        let body = match extract_body(&claude_path, &section) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("section extract: {e}");
                return Ok(());
            }
        };
        let temp_path =
            match write_temp_for_section(&self.pm_dir, &ticket.to_string(), &section, &body) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("section temp: {e}");
                    return Ok(());
                }
            };

        // Suspend, edit, resume.
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        let editor_bin = std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
        let _ = std::process::Command::new(&editor_bin)
            .arg(&temp_path)
            .status();

        enable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;
        terminal.clear()?;

        // Splice the edited body back into CLAUDE.md.
        match read_temp(&temp_path) {
            Ok(new_body) => {
                if let Err(e) = splice_back(&claude_path, &section, &new_body) {
                    eprintln!("section splice: {e}");
                }
            }
            Err(e) => eprintln!("section read-back: {e}"),
        }
        cleanup_temp(&temp_path);

        // Refresh the editor's section list and the database so the
        // form reflects whatever the user changed.
        if let Some(ed) = self.editor.as_mut() {
            let _ = ed.refresh_sections();
        }
        self.db = Database::load(&self.pm_dir);
        Ok(())
    }

    /// Raw `$EDITOR` on the ticket's full CLAUDE.md. Power-user
    /// escape hatch wired to `e` from the board.
    fn run_raw_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        id: LeafId,
    ) -> io::Result<()> {
        self.run_editor(terminal, id)
    }

    /// Suspend the alternate screen and hand `$EDITOR` an arbitrary
    /// file path. Used by the Memories and Templates surfaces; the
    /// resume + Database reload path mirrors the ticket editor flow
    /// so any front-matter changes the user made surface on the next
    /// render.
    fn run_external_editor(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        path: &std::path::Path,
    ) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
        let _ = std::process::Command::new(&editor).arg(path).status();

        enable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;
        terminal.clear()?;

        self.db = Database::load(&self.pm_dir);
        Ok(())
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
                    crossterm::event::KeyCode::Char('2') => self.mode = Mode::Memories,
                    crossterm::event::KeyCode::Char('3') => self.mode = Mode::Terminals,
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
                    Disposition::OpenTicketEditor(id) => return ShellOutcome::OpenEditor(id),
                    Disposition::EditRaw(id) => return ShellOutcome::EditRaw(id),
                    Disposition::EditPath(path) => return ShellOutcome::EditPath(path),
                }
                ShellOutcome::Continue
            }
            ShellAction::WorkbenchKey(k, m) => {
                let scope = self.lhp.scope(&self.db);
                match self
                    .workbench
                    .handle_key(self.mode, k, m, &self.db, &self.pm_dir, scope)
                {
                    Disposition::Consumed => {}
                    Disposition::OverflowLeft => {
                        // Left from the leftmost board column hands
                        // focus back to the LHP.
                        self.focus = Focus::Lhp;
                    }
                    Disposition::OverflowRight => {
                        // Right from the rightmost column: stay put.
                    }
                    Disposition::OpenTicketEditor(id) => return ShellOutcome::OpenEditor(id),
                    Disposition::EditRaw(id) => return ShellOutcome::EditRaw(id),
                    Disposition::EditPath(path) => return ShellOutcome::EditPath(path),
                }
                ShellOutcome::Continue
            }
            ShellAction::None => ShellOutcome::Continue,
        }
    }

    fn render(&self, f: &mut ratatui::Frame) {
        let area = f.area();

        // The full-screen ticket editor owns the whole canvas when
        // open; the normal LHP + Workbench + Activity layout is
        // hidden until the user saves or cancels.
        if let Some(editor) = &self.editor {
            editor.render(f, area);
            return;
        }

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
            &self.pm_dir,
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
