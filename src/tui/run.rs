//! TUI entry-point setup.
//!
//! v0.3.0 demolition removed the per-project `App` and its launcher
//! (`MenuApp`). The remaining entry point is the standalone activity
//! view driven by `spacecell tv`; the workflow board has its own setup
//! in [`crate::tui::workflow_run`].

use std::{io, path::Path, time::Duration};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{prelude::CrosstermBackend, Terminal};

use crate::views::events_view::{ActivityAction, ActivityView};

/// Drive the full-screen activity view standalone, the way `spacecell tv`
/// does. The same renderer will back the v0.3.x cockpit's activity mode
/// once that lands; only the host loop differs.
///
/// The loop tick is 200ms: input is polled with that timeout, and each tick
/// also refreshes the activity view's buffer from `events.log` so a quiet
/// terminal still picks up new events within the same window. `Ctrl+C`
/// exits alongside `q` / `Esc`.
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
                // Ctrl+C exits the standalone binary regardless of the
                // view's own bindings (which use bare `c` for clear-filter).
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
