//! Append-only activity feed at `.pm/events.log`.
//!
//! Every state-changing verb records one JSONL line. The feed is the shared
//! source for the TUI bottom-bar tail (Phase 7), the full-screen `pm tv` view
//! (Phase 9), and any external consumer that wants to `tail -f` the workspace.
//!
//! Writes go through `OpenOptions::append`, which maps to `O_APPEND` on Unix.
//! A single event line stays well under `PIPE_BUF`, so concurrent agents
//! writing to the same feed interleave at line boundaries without a lock.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::LeafId;
use super::layout::Layout;

/// One entry in the activity feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// When the event was recorded (UTC).
    pub ts: DateTime<Utc>,
    /// Who triggered it - `PM_AGENT_ID` or the `claude-<host>-<pid>` default.
    pub actor: String,
    /// The verb that ran (`checkout`, `edit`, `status`, ...).
    pub verb: String,
    /// The ticket the verb acted on. Absent for workspace-level verbs such as
    /// `init`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<LeafId>,
    /// Free-form context - an intent string for `checkout`, a summary for
    /// `checkin`/`edit`, and so on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Errors emitted by the events layer.
#[derive(Debug)]
pub enum EventError {
    Io(std::io::Error),
    Serialise(serde_json::Error),
}

impl std::fmt::Display for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventError::Io(e) => write!(f, "events.log io: {e}"),
            EventError::Serialise(e) => write!(f, "events.log serialise: {e}"),
        }
    }
}

impl std::error::Error for EventError {}

/// Result type used across the events layer.
pub type EventResult<T> = Result<T, EventError>;

/// Resolve the actor string. `PM_AGENT_ID` wins when set to a non-empty value;
/// otherwise the actor is `claude-<host>-<pid>`.
pub fn actor() -> String {
    let env = std::env::var("PM_AGENT_ID").ok();
    let host = system_hostname().unwrap_or_default();
    actor_from(env.as_deref(), &host, std::process::id())
}

/// Pure actor-string builder, split out from [`actor`] so the resolution rule
/// can be tested without touching process environment or the host.
fn actor_from(env_value: Option<&str>, host: &str, pid: u32) -> String {
    if let Some(v) = env_value {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let host = if host.trim().is_empty() {
        "host"
    } else {
        host.trim()
    };
    format!("claude-{host}-{pid}")
}

/// The machine hostname, or `None` if it cannot be resolved or is empty.
///
/// On Unix this is the POSIX `gethostname(3)` call. The hostname is cosmetic -
/// it only flavours the default `claude-<host>-<pid>` actor string, which
/// `PM_AGENT_ID` overrides entirely - so a `None` here is harmless.
#[cfg(unix)]
pub(crate) fn system_hostname() -> Option<String> {
    use core::ffi::{c_char, c_int};
    extern "C" {
        fn gethostname(name: *mut c_char, len: usize) -> c_int;
    }
    let mut buf = [0u8; 256];
    // SAFETY: `gethostname` writes at most `buf.len()` bytes into `buf`. On a
    // name that fits it null-terminates; on truncation POSIX may leave it
    // unterminated, so the NUL search below is capped at the buffer length.
    let rc = unsafe { gethostname(buf.as_mut_ptr() as *mut c_char, buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).into_owned();
    if name.trim().is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Non-Unix fallback: best-effort via the environment. Windows sets
/// `COMPUTERNAME`. A proper Windows `GetComputerNameExW` path is left for the
/// cross-platform pass (PM_BUILD_PLAN.md Phase 12).
#[cfg(not(unix))]
pub(crate) fn system_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Record one event in `pm_dir`'s activity feed. The actor and timestamp are
/// filled in here so callers only supply the verb and its context.
pub fn emit_event(
    pm_dir: &Path,
    verb: &str,
    id: Option<LeafId>,
    detail: Option<&str>,
) -> EventResult<()> {
    let event = Event {
        ts: Utc::now(),
        actor: actor(),
        verb: verb.to_string(),
        id,
        detail: detail.map(|s| s.to_string()),
    };
    append_event(pm_dir, &event)
}

/// Append a pre-built event. Exposed mainly for tests that need a fixed
/// timestamp; ordinary callers use [`emit_event`].
pub fn append_event(pm_dir: &Path, event: &Event) -> EventResult<()> {
    let path = Layout::at(pm_dir).events_log_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(EventError::Io)?;
    }
    let mut line = serde_json::to_string(event).map_err(EventError::Serialise)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(EventError::Io)?;
    file.write_all(line.as_bytes()).map_err(EventError::Io)
}

/// Read the whole activity feed in write order. A missing feed reads as empty.
/// Blank lines are skipped, and a malformed line is dropped rather than
/// aborting the read - the feed is append-only and one bad line should not
/// hide the rest of the history.
pub fn read_events(pm_dir: &Path) -> EventResult<Vec<Event>> {
    let path = Layout::at(pm_dir).events_log_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path).map_err(EventError::Io)?;
    let mut events = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Event>(line) {
            events.push(event);
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-events-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // events.log lives at the `.pm/` root; the layout's events_log_path
        // joins `events.log` onto `pm_dir`, so the dir itself is the root.
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn actor_prefers_pm_agent_id() {
        assert_eq!(
            actor_from(Some("claude-be"), "workstation", 42),
            "claude-be"
        );
        // Whitespace is trimmed.
        assert_eq!(
            actor_from(Some("  claude-fe  "), "workstation", 42),
            "claude-fe"
        );
    }

    #[test]
    fn system_hostname_is_none_or_non_empty() {
        // The value is environment-specific, so the contract is only: it does
        // not panic, and a Some is never empty.
        if let Some(name) = system_hostname() {
            assert!(
                !name.trim().is_empty(),
                "a resolved hostname must be non-empty"
            );
        }
    }

    #[test]
    fn actor_falls_back_to_host_pid() {
        assert_eq!(
            actor_from(None, "workstation", 12482),
            "claude-workstation-12482"
        );
        // Empty env value is ignored.
        assert_eq!(
            actor_from(Some("   "), "workstation", 7),
            "claude-workstation-7"
        );
        // Empty host degrades to a literal.
        assert_eq!(actor_from(None, "", 7), "claude-host-7");
    }

    #[test]
    fn emit_then_read_round_trip() {
        let dir = tmp_dir();
        let tsk = LeafId::new(TypePrefix::Task, 7);
        emit_event(&dir, "checkout", Some(tsk), Some("implement TTL")).unwrap();
        emit_event(&dir, "edit", Some(tsk), None).unwrap();
        emit_event(&dir, "init", None, None).unwrap();

        let events = read_events(&dir).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].verb, "checkout");
        assert_eq!(events[0].id, Some(tsk));
        assert_eq!(events[0].detail.as_deref(), Some("implement TTL"));
        assert_eq!(events[1].verb, "edit");
        assert_eq!(events[1].detail, None);
        assert_eq!(events[2].verb, "init");
        assert_eq!(events[2].id, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn each_line_is_standalone_json() {
        let dir = tmp_dir();
        let tsk = LeafId::new(TypePrefix::Task, 1);
        emit_event(&dir, "add", Some(tsk), Some("Lock protocol")).unwrap();
        emit_event(&dir, "status", Some(tsk), None).unwrap();

        let raw = std::fs::read_to_string(Layout::at(&dir).events_log_path()).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            serde_json::from_str::<Event>(line).expect("each line parses on its own");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_tolerates_blank_and_malformed_lines() {
        let dir = tmp_dir();
        let tsk = LeafId::new(TypePrefix::Task, 9);
        emit_event(&dir, "add", Some(tsk), None).unwrap();
        // Hand-append a blank line and a junk line.
        let path = Layout::at(&dir).events_log_path();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"\n{ not json\n").unwrap();
        emit_event(&dir, "status", Some(tsk), None).unwrap();

        let events = read_events(&dir).unwrap();
        // The two real events survive; blank and malformed lines are dropped.
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].verb, "add");
        assert_eq!(events[1].verb, "status");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_missing_feed_is_empty() {
        let dir = tmp_dir();
        let events = read_events(&dir).unwrap();
        assert!(events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn concurrent_appends_keep_every_line_intact() {
        use std::sync::Arc;
        use std::thread;
        let dir = Arc::new(tmp_dir());
        let mut handles = Vec::new();
        for t in 0..4u32 {
            let dir = Arc::clone(&dir);
            handles.push(thread::spawn(move || {
                for n in 0..25u32 {
                    let leaf = LeafId::new(TypePrefix::Task, (t * 100 + n) as u64);
                    emit_event(&dir, "edit", Some(leaf), Some("concurrent")).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // 4 threads x 25 events, every line still parses.
        let events = read_events(&dir).unwrap();
        assert_eq!(events.len(), 100, "no line was torn by concurrent appends");
        std::fs::remove_dir_all(&*dir).ok();
    }
}
