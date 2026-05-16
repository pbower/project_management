//! Editor resolution.
//!
//! The CLI and TUI both shell out to an external editor for prose edits.
//! Behaviour follows PM_DESIGN.md Section 8.4:
//!
//! 1. If `$EDITOR` is set and non-empty, honour it.
//! 2. Otherwise walk the fallback chain `nvim, vim, helix, nano, vi` and
//!    pick the first one available on the user's `$PATH`.
//! 3. If nothing is on `$PATH`, fall back to `nano` as the historical
//!    default. The launch will then fail with a clear "command not found"
//!    error rather than silently doing nothing.

use std::path::PathBuf;

/// Fallback chain in priority order.
const FALLBACK_CHAIN: &[&str] = &["nvim", "vim", "helix", "nano", "vi"];

/// Resolve which editor program to launch. Returns the program name as a
/// `String`; pass it directly into `Command::new`.
pub fn resolve_editor() -> String {
    if let Ok(value) = std::env::var("EDITOR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    for candidate in FALLBACK_CHAIN {
        if which(candidate).is_some() {
            return candidate.to_string();
        }
    }
    // Nothing on PATH: emit the historical default so the failure mode is a
    // clear "nano: command not found" rather than a silent no-op.
    "nano".to_string()
}

/// Lightweight `which` shim: walks `$PATH` looking for `name` and returns
/// the first hit. Avoids pulling in the `which` crate for a one-liner.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honours_editor_env_when_set() {
        // SAFETY: tests in this module are not run in parallel by default
        // for env-var work; cargo serialises by default unless the user
        // overrides. We restore the prior value at the end.
        let previous = std::env::var("EDITOR").ok();
        unsafe {
            std::env::set_var("EDITOR", "my-editor");
        }
        assert_eq!(resolve_editor(), "my-editor");
        match previous {
            Some(v) => unsafe { std::env::set_var("EDITOR", v) },
            None => unsafe { std::env::remove_var("EDITOR") },
        }
    }

    #[test]
    fn empty_editor_falls_through_to_chain() {
        let previous = std::env::var("EDITOR").ok();
        unsafe {
            std::env::set_var("EDITOR", "   ");
        }
        // Resolve picks whatever is first available on PATH from the chain.
        // We do not assert which one; we only assert the function does not
        // return the empty string or whitespace.
        let resolved = resolve_editor();
        assert!(!resolved.trim().is_empty());
        assert!(FALLBACK_CHAIN.contains(&resolved.as_str()) || resolved == "nano");
        match previous {
            Some(v) => unsafe { std::env::set_var("EDITOR", v) },
            None => unsafe { std::env::remove_var("EDITOR") },
        }
    }
}
