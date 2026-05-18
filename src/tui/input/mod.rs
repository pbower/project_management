//! Mode router for the main shell.
//!
//! Arrow keys flow seamlessly across LHP and Workbench: zones return a
//! [`Disposition`] from their key handlers, and a `OverflowLeft` /
//! `OverflowRight` value tells the shell to hand focus over to the
//! adjacent zone. `Tab` / `Shift-Tab` cycle modes; `1` / `2` / `3`
//! jump directly. `?` and `F1` open the help overlay. `q` quits when
//! no overlay is active.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::store::LeafId;

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

    // Shift + arrow jumps focus between zones from anywhere, so the
    // user does not have to drill to a hierarchy edge before crossing
    // into the board (or back). The unmodified arrows still drive
    // within-zone navigation; only the chord crosses zones.
    if mods.contains(KeyModifiers::SHIFT) {
        match key {
            KeyCode::Right => return ShellAction::FocusZone(Focus::Workbench),
            KeyCode::Left => return ShellAction::FocusZone(Focus::Lhp),
            _ => {}
        }
    }

    // Everything else flows to the focused zone. Focus moves across
    // zones via arrow-key overflow inside the zone handlers, plus the
    // Shift+arrow shortcut above.
    match focus {
        Focus::Lhp => ShellAction::LhpKey(key, mods),
        Focus::Workbench => ShellAction::WorkbenchKey(key, mods),
    }
}

/// What a zone's key handler tells the shell after processing a
/// keystroke. `Consumed` covers both "handled with state change" and
/// "handled with no change" (the shell re-renders on every tick
/// regardless). The overflow variants ask the shell to flip focus to
/// the adjacent zone so arrow-key navigation flows across the whole
/// cockpit. `OpenTicketEditor` swaps the layout for the full-screen
/// ticket editor; `EditRaw` suspends the TUI and hands the terminal
/// to `$EDITOR` on the ticket's raw CLAUDE.md file (escape hatch).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Disposition {
    Consumed,
    OverflowLeft,
    OverflowRight,
    OpenTicketEditor(LeafId),
    EditRaw(LeafId),
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
    fn brackets_are_no_longer_focus_keys() {
        // Bracket keys used to flip focus in v0.3.1. The arrow-key
        // overflow model dropped them; they should now flow through to
        // the focused zone like any other unmapped key.
        let a = route(
            KeyCode::Char('['),
            KeyModifiers::NONE,
            Focus::Workbench,
            false,
        );
        assert_eq!(
            a,
            ShellAction::WorkbenchKey(KeyCode::Char('['), KeyModifiers::NONE)
        );
    }

    #[test]
    fn shift_arrows_jump_zones_from_anywhere() {
        let a = route(KeyCode::Right, KeyModifiers::SHIFT, Focus::Lhp, false);
        assert_eq!(a, ShellAction::FocusZone(Focus::Workbench));
        let b = route(KeyCode::Left, KeyModifiers::SHIFT, Focus::Workbench, false);
        assert_eq!(b, ShellAction::FocusZone(Focus::Lhp));
        // Plain arrows are still within-zone navigation.
        let c = route(KeyCode::Right, KeyModifiers::NONE, Focus::Lhp, false);
        assert_eq!(c, ShellAction::LhpKey(KeyCode::Right, KeyModifiers::NONE));
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
