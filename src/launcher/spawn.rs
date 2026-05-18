//! Spawn a configured-launcher terminal.
//!
//! Glues [`config`](super::config) (templates + substitution) and
//! [`registry`](super::registry) (on-disk tracking) together. The
//! `spacecell run <id>` CLI verb is the only caller; callers must
//! resolve `pm_dir` and the scope leaf before invoking.

use std::path::Path;
use std::process::Command;

use crate::store::events::emit_event;
use crate::store::LeafId;

use super::config::{load_config, resolve_spawn_command, ScopeSubstitution};
use super::registry::{write_terminal, TerminalEntry};

/// Errors the spawn path can surface to the user. The user-facing CLI
/// turns these into a single-line message; no panic on bad config.
#[derive(Debug)]
pub enum SpawnError {
    EmptySpawnCommand,
    Io(std::io::Error),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnError::EmptySpawnCommand => {
                write!(f, "launcher spawn template resolved to an empty string")
            }
            SpawnError::Io(e) => write!(f, "spawn failed: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {}

/// Spawn a terminal scoped to `scope`. Generates a UUID, resolves the
/// launcher template, performs substitution, exec's the result, and
/// writes the registry entry so the cockpit can track the result.
///
/// `cwd` defaults to `pm_dir` when `None`.
/// `title` is used to build the `{label}` substitution.
///
/// Returns the registry UUID so the caller can print it or pipe it
/// to `spacecell focus`.
pub fn spawn_terminal(
    pm_dir: &Path,
    scope: LeafId,
    title: Option<&str>,
    cwd: Option<&Path>,
) -> Result<String, SpawnError> {
    let cfg = load_config(pm_dir);
    let spawn_template = resolve_spawn_command(&cfg);
    if spawn_template.trim().is_empty() {
        return Err(SpawnError::EmptySpawnCommand);
    }

    let uuid = generate_uuid();
    let label = super::config::label_for(scope, title);
    let cwd = cwd
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| pm_dir.display().to_string());

    let sub = ScopeSubstitution {
        cmd: format!("spacecell agent --window {uuid}"),
        uuid: uuid.clone(),
        scope: scope.to_string(),
        label: label.clone(),
        cwd: cwd.clone(),
    };
    let spawn_line = sub.apply(&spawn_template);

    // Run the spawn line through `$SHELL -c` so the template can be a
    // full command line with redirects, env tweaks, or `&` to detach.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let child = Command::new(&shell)
        .arg("-c")
        .arg(&spawn_line)
        .spawn()
        .map_err(SpawnError::Io)?;
    let pid = child.id();

    let entry = TerminalEntry::new(uuid.clone(), scope, label, spawn_line, pid);
    if let Err(e) = write_terminal(pm_dir, &entry) {
        return Err(SpawnError::Io(e));
    }

    // Emit an event so the cockpit's activity strip surfaces the
    // spawn without polling the registry. Detail string carries the
    // UUID + label for the human reader.
    let detail = format!("{uuid} ({})", entry.label);
    let _ = emit_event(pm_dir, "terminal-spawn", Some(scope), Some(&detail));

    Ok(uuid)
}

/// Generate a short, URL-safe UUID for a terminal. v0.3.5 uses a
/// time-based 12-char id rather than pulling in a uuid crate; the
/// cardinality is more than enough for a single-user workspace and
/// the format is easy to grep for in tmux/i3 window titles.
fn generate_uuid() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Base36 of the lower 60 bits gives us 12 chars max; pad with a
    // `tk-` prefix to make registry files easy to spot.
    let truncated = (nanos as u64) & 0x0FFF_FFFF_FFFF_FFFF;
    format!("tk-{}", base36(truncated))
}

fn base36(mut n: u64) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut buf: Vec<u8> = Vec::new();
    while n > 0 {
        buf.push(ALPHABET[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_uuids_are_prefixed_and_distinct() {
        let a = generate_uuid();
        let b = generate_uuid();
        assert!(a.starts_with("tk-"), "uuid prefix missing: {a}");
        assert!(b.starts_with("tk-"));
        assert_ne!(a, b);
    }

    #[test]
    fn base36_round_trips_a_few_values() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
    }
}
