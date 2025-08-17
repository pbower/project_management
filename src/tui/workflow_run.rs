//! Workflow TUI entry point and setup.

use std::{io, path::Path};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}
};
use ratatui::{prelude::CrosstermBackend, Terminal};

use crate::tui::workflow::{WorkflowApp, WorkflowExit};

/// Initialise and run the workflow terminal user interface.
/// Returns the exit action requested by the user.
pub fn run_workflow_tui(db_path: &Path) -> io::Result<WorkflowExit> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = WorkflowApp::new(db_path)?;
    let result = app.run(&mut terminal);
    let exit_action = app.get_exit_action();

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result?;
    Ok(exit_action)
}
