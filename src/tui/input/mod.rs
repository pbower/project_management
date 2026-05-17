//! Mode router for the main shell.
//!
//! v0.3.1 wires `Tab` / `Shift-Tab` to cycle modes and `1` / `2` / `3` to
//! jump directly. `?` and `F1` open the help overlay. `q` quits when no
//! overlay is active. Everything else flows through to the focused zone
//! (LHP or Workbench).

use crossterm::event::{KeyCode, KeyModifiers};

/// The three Workbench presets described in PM_DESIGN section 8.3.2.
/// Mode 1 hosts the kanban board; Modes 2 and 3 are placeholders in
/// v0.3.1 and gain content in later sub-phases.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Board,
    Documents,
    Activity,
}

impl Mode {
    /// Next mode in the `Tab` cycle.
    pub fn next(self) -> Mode {
        match self {
            Mode::Board => Mode::Documents,
            Mode::Documents => Mode::Activity,
            Mode::Activity => Mode::Board,
        }
    }

    /// Previous mode in the `Shift+Tab` cycle.
    pub fn prev(self) -> Mode {
        match self {
            Mode::Board => Mode::Activity,
            Mode::Documents => Mode::Board,
            Mode::Activity => Mode::Documents,
        }
    }

    /// Short label rendered in the header for the active mode.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Board => "BOARD",
            Mode::Documents => "DOCUMENTS",
            Mode::Activity => "ACTIVITY",
        }
    }
}

/// Which zone of the shell currently owns the input focus. Mode-switch
/// and quit keys are processed before any focused dispatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Lhp,
    Workbench,
}

/// What the shell should do after a keystroke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShellAction {
    /// Nothing happened that needs the shell to react.
    None,
    /// User requested a clean shutdown.
    Quit,
    /// Switch to the named mode.
    SwitchMode(Mode),
    /// Toggle the help overlay.
    ToggleHelp,
    /// Move keyboard focus to the named zone.
    FocusZone(Focus),
    /// Forward the keystroke to the LHP zone for hierarchy nav.
    LhpKey(KeyCode, KeyModifiers),
    /// Forward the keystroke to the Workbench zone.
    WorkbenchKey(KeyCode, KeyModifiers),
}

/// Translate a raw keystroke into a [`ShellAction`].
///
/// Global keys (mode switch, quit, help, focus switch) are recognised
/// first. Anything left over routes to the focused zone.
pub fn route(key: KeyCode, mods: KeyModifiers, focus: Focus, help_open: bool) -> ShellAction {
    // Help overlay swallows everything except its own dismiss key and
    // mode-switch keys (which also close it before switching).
    if help_open {
        match key {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::F(1) => return ShellAction::ToggleHelp,
            KeyCode::Tab => return ShellAction::SwitchMode(Mode::Board.next()),
            KeyCode::BackTab => return ShellAction::SwitchMode(Mode::Board.prev()),
            KeyCode::Char('1') => return ShellAction::SwitchMode(Mode::Board),
            KeyCode::Char('2') => return ShellAction::SwitchMode(Mode::Documents),
            KeyCode::Char('3') => return ShellAction::SwitchMode(Mode::Activity),
            _ => return ShellAction::None,
        }
    }

    // Quit
    if matches!(key, KeyCode::Char('q')) && mods.is_empty() {
        return ShellAction::Quit;
    }
    if matches!(key, KeyCode::Char('c')) && mods.contains(KeyModifiers::CONTROL) {
        return ShellAction::Quit;
    }

    // Help overlay toggle
    if matches!(key, KeyCode::Char('?')) || matches!(key, KeyCode::F(1)) {
        return ShellAction::ToggleHelp;
    }

    // Mode switching
    match key {
        KeyCode::Tab => return ShellAction::SwitchMode(Mode::Board), // resolved by caller
        KeyCode::BackTab => return ShellAction::SwitchMode(Mode::Board),
        KeyCode::Char('1') => return ShellAction::SwitchMode(Mode::Board),
        KeyCode::Char('2') => return ShellAction::SwitchMode(Mode::Documents),
        KeyCode::Char('3') => return ShellAction::SwitchMode(Mode::Activity),
        _ => {}
    }

    // Focus switching: square brackets jump focus between LHP and
    // Workbench so two-handed keyboards do not need a chord.
    if matches!(key, KeyCode::Char('[')) {
        return ShellAction::FocusZone(Focus::Lhp);
    }
    if matches!(key, KeyCode::Char(']')) {
        return ShellAction::FocusZone(Focus::Workbench);
    }

    // Everything else flows to the focused zone.
    match focus {
        Focus::Lhp => ShellAction::LhpKey(key, mods),
        Focus::Workbench => ShellAction::WorkbenchKey(key, mods),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_cycles_in_both_directions() {
        let m = Mode::Board;
        assert_eq!(m.next().next().next(), Mode::Board);
        assert_eq!(m.prev().prev().prev(), Mode::Board);
        assert_eq!(m.next().prev(), Mode::Board);
    }

    #[test]
    fn quit_keys_are_recognised() {
        let a = route(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            Focus::Workbench,
            false,
        );
        assert_eq!(a, ShellAction::Quit);
        let b = route(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            Focus::Workbench,
            false,
        );
        assert_eq!(b, ShellAction::Quit);
    }

    #[test]
    fn help_swallows_unrelated_keys() {
        let a = route(KeyCode::Char('x'), KeyModifiers::NONE, Focus::Lhp, true);
        assert_eq!(a, ShellAction::None);
    }

    #[test]
    fn focus_brackets_switch_zones() {
        let a = route(
            KeyCode::Char('['),
            KeyModifiers::NONE,
            Focus::Workbench,
            false,
        );
        assert_eq!(a, ShellAction::FocusZone(Focus::Lhp));
        let b = route(KeyCode::Char(']'), KeyModifiers::NONE, Focus::Lhp, false);
        assert_eq!(b, ShellAction::FocusZone(Focus::Workbench));
    }

    #[test]
    fn unmapped_key_forwards_to_focused_zone() {
        let a = route(KeyCode::Up, KeyModifiers::NONE, Focus::Lhp, false);
        assert_eq!(a, ShellAction::LhpKey(KeyCode::Up, KeyModifiers::NONE));
        let b = route(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
            Focus::Workbench,
            false,
        );
        assert_eq!(
            b,
            ShellAction::WorkbenchKey(KeyCode::Char('e'), KeyModifiers::NONE)
        );
    }
}
