//! Color constants for the terminal user interface.

use ratatui::style::Color;

// These support branded views of the UI
// reflecting the current item hierarchy

// Native Color::Blue is used for Product

/// Used for Epics
pub const DARK_GREEN: Color = Color::Rgb(0, 80, 0);
/// Used for Tasks
pub const GOLD: Color = Color::Rgb(255, 215, 0);
/// Used for Subtasks
pub const DARK_RED: Color = Color::Rgb(114, 0, 0);
/// Used for Milestones
pub const DARK_PURPLE: Color = Color::Rgb(86, 60, 92);
