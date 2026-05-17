//! Terminal user interface entry point and setup.

use std::{io, path::Path, time::Duration};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::CrosstermBackend, Terminal};

use crate::store::LeafId;
use crate::tui::app::App;
use crate::views::events_view::{ActivityAction, ActivityView};

/// Initialise and run the terminal user interface.
pub fn run_tui(db_path: &Path) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(db_path)?;
    let result = app.run(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Drive the full-screen activity view standalone, the way `pm tv` does. The
/// renderer is the same one Mode 3 uses inside the main TUI; only the host
/// loop differs.
///
/// The loop tick is 200ms: input is polled with that timeout, and each tick
/// also refreshes the activity view's buffer from `events.log` so a quiet
/// terminal still picks up new events within the same window. Ctrl+C exits
/// alongside `q` / `Esc`.
pub fn run_activity_view(pm_dir: &Path) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut view = ActivityView::new(pm_dir.to_path_buf());
    let result = drive_activity_view(&mut terminal, &mut view);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn drive_activity_view<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    view: &mut ActivityView,
) -> io::Result<()> {
    loop {
        if let Err(e) = view.refresh() {
            // Surface the read failure in the view's transient hint line.
            // The view keeps rendering whatever it parsed earlier; the next
            // refresh tries again.
            view.hint = Some(format!("events.log: {e}"));
        }
        terminal.draw(|f| view.render(f, f.area()))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                // Ctrl+C exits the standalone binary regardless of the view's
                // own bindings (which use bare `c` for clear-filter).
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
                if view.handle_key(key.code, key.modifiers) == ActivityAction::ExitView {
                    return Ok(());
                }
            }
        }
    }
}

/// Run the TUI with a specific task pre-selected for editing.
pub fn run_tui_with_edit(db_path: &Path, task_id: LeafId) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(db_path)?;
    app.open_task_for_edit(task_id);
    let result = app.run(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}
