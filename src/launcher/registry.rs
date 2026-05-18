//! Terminal registry under `.pm/terminals/<uuid>.json`.
//!
//! Each spawned terminal gets one JSON file written here. Atomic
//! writes (temp-file + rename) so a concurrent reader never sees a
//! torn snapshot. A short-lived background thread refreshes
//! `last_heartbeat` every 30s while the terminal lives; on exit the
//! `spacecell agent --window` driver flips `status` to `Closed`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::id::LeafId;
use crate::store::state::atomic_write;

/// How often `last_heartbeat` is refreshed by the in-process
/// heartbeat thread. Matches the lock-heartbeat cadence so a single
/// missed tick does not look like a dead terminal.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// A terminal whose `last_heartbeat` is older than this is considered
/// dead by `spacecell doctor --purge-terminals` and the cockpit's
/// Terminals surface.
pub const STALE_TERMINAL_TTL: Duration = Duration::from_secs(120);

/// Lifecycle status of a spawned terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalStatus {
    /// Heartbeat-refreshed within the last `STALE_TERMINAL_TTL`.
    Active,
    /// `spacecell agent --window` exited cleanly and flipped status.
    Closed,
    /// Heartbeat past TTL; the agent likely crashed without flipping.
    /// Set by `doctor --purge-terminals` or the Terminals surface
    /// when it surfaces the entry.
    Dead,
}

/// One entry in the terminal registry. The on-disk file maps 1:1.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TerminalEntry {
    pub uuid: String,
    pub scope: LeafId,
    pub agent_id: String,
    pub pid: u32,
    pub spawned_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
    pub label: String,
    pub spawn_command: String,
    pub status: TerminalStatus,
}

impl TerminalEntry {
    /// Build a fresh active entry. The caller fills in `spawn_command`
    /// after substitution; everything else defaults to "now".
    pub fn new(
        uuid: String,
        scope: LeafId,
        label: String,
        spawn_command: String,
        pid: u32,
    ) -> Self {
        let now = Utc::now();
        TerminalEntry {
            uuid,
            scope,
            agent_id: default_agent_id(pid),
            pid,
            spawned_at: now,
            last_heartbeat: now,
            label,
            spawn_command,
            status: TerminalStatus::Active,
        }
    }

    /// Treat the entry as dead when the heartbeat is past TTL.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        let elapsed = now.signed_duration_since(self.last_heartbeat);
        elapsed
            .to_std()
            .map(|d| d > STALE_TERMINAL_TTL)
            .unwrap_or(false)
    }
}

fn default_agent_id(pid: u32) -> String {
    std::env::var("PM_AGENT_ID").unwrap_or_else(|_| {
        let host = std::env::var("HOSTNAME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "host".to_string());
        format!("claude-{host}-{pid}")
    })
}

fn terminals_dir(pm_dir: &Path) -> PathBuf {
    pm_dir.join("terminals")
}

fn entry_path(pm_dir: &Path, uuid: &str) -> PathBuf {
    terminals_dir(pm_dir).join(format!("{uuid}.json"))
}

/// Write or replace `entry` on disk via atomic rename.
pub fn write_terminal(pm_dir: &Path, entry: &TerminalEntry) -> io::Result<()> {
    fs::create_dir_all(terminals_dir(pm_dir))?;
    let path = entry_path(pm_dir, &entry.uuid);
    let json =
        serde_json::to_vec_pretty(entry).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    atomic_write(&path, &json)
}

/// Load one entry by UUID. Returns `None` when the file is missing
/// or unreadable; callers treat that as "not found" rather than as a
/// hard failure.
pub fn load_terminal(pm_dir: &Path, uuid: &str) -> Option<TerminalEntry> {
    let raw = fs::read_to_string(entry_path(pm_dir, uuid)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Every terminal entry in the registry, sorted by `spawned_at`
/// oldest-first so callers can format chronological tables.
pub fn list_terminals(pm_dir: &Path) -> io::Result<Vec<TerminalEntry>> {
    let dir = terminals_dir(pm_dir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<TerminalEntry> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(entry.path()) {
            if let Ok(parsed) = serde_json::from_str::<TerminalEntry>(&raw) {
                out.push(parsed);
            }
        }
    }
    out.sort_by_key(|e| e.spawned_at);
    Ok(out)
}

/// Mark `uuid` as closed. Idempotent: a missing entry is a no-op
/// because the user may have already purged it.
pub fn mark_terminal_closed(pm_dir: &Path, uuid: &str) -> io::Result<()> {
    let mut entry = match load_terminal(pm_dir, uuid) {
        Some(e) => e,
        None => return Ok(()),
    };
    entry.status = TerminalStatus::Closed;
    entry.last_heartbeat = Utc::now();
    write_terminal(pm_dir, &entry)
}

/// Sweep the registry and either delete or mark-dead any entry whose
/// last heartbeat is older than [`STALE_TERMINAL_TTL`]. Returns the
/// uuids that were acted on so the doctor verb can report them.
pub fn purge_dead_terminals(pm_dir: &Path, delete: bool) -> io::Result<Vec<String>> {
    let now = Utc::now();
    let mut acted: Vec<String> = Vec::new();
    for entry in list_terminals(pm_dir)? {
        if !entry.is_stale(now) {
            continue;
        }
        if entry.status == TerminalStatus::Closed {
            continue;
        }
        if delete {
            let _ = fs::remove_file(entry_path(pm_dir, &entry.uuid));
        } else {
            let mut updated = entry.clone();
            updated.status = TerminalStatus::Dead;
            write_terminal(pm_dir, &updated)?;
        }
        acted.push(entry.uuid);
    }
    Ok(acted)
}

/// In-process heartbeat thread. Owned by `spacecell agent --window`
/// for the life of the inner command; dropping the handle signals the
/// thread to stop on the next tick.
pub struct HeartbeatThread {
    pm_dir: PathBuf,
    uuid: String,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl HeartbeatThread {
    /// Start a heartbeat thread for `uuid`. Refreshes
    /// `last_heartbeat` every [`HEARTBEAT_INTERVAL`].
    pub fn start(pm_dir: PathBuf, uuid: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let pm_dir_for_thread = pm_dir.clone();
        let uuid_for_thread = uuid.clone();
        let join = thread::spawn(move || loop {
            // Sleep in short ticks so we react to `stop` within a
            // second even though the heartbeat itself is 30s.
            for _ in 0..30 {
                if stop_for_thread.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
            if let Some(mut entry) = load_terminal(&pm_dir_for_thread, &uuid_for_thread) {
                entry.last_heartbeat = Utc::now();
                let _ = write_terminal(&pm_dir_for_thread, &entry);
            }
        });
        HeartbeatThread {
            pm_dir,
            uuid,
            stop,
            join: Some(join),
        }
    }

    /// Stop the heartbeat thread and flip status to `Closed`.
    pub fn stop_and_close(mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        mark_terminal_closed(&self.pm_dir, &self.uuid)
    }
}

impl Drop for HeartbeatThread {
    fn drop(&mut self) {
        // Defensive: if the owner forgot to call `stop_and_close`,
        // we still try to signal the worker to exit so the process
        // can shut down cleanly.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;

    fn tmp_pm_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-registry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_load_round_trips() {
        let dir = tmp_pm_dir();
        let leaf = LeafId::new(TypePrefix::Task, 7);
        let entry = TerminalEntry::new(
            "u-1".into(),
            leaf,
            "TSK7 lock".into(),
            "$SHELL -c 'spacecell agent --window u-1'".into(),
            12345,
        );
        write_terminal(&dir, &entry).unwrap();
        let back = load_terminal(&dir, "u-1").unwrap();
        assert_eq!(back.uuid, entry.uuid);
        assert_eq!(back.scope, entry.scope);
        assert_eq!(back.status, TerminalStatus::Active);
    }

    #[test]
    fn list_returns_sorted_entries() {
        let dir = tmp_pm_dir();
        let leaf = LeafId::new(TypePrefix::Task, 1);
        for n in 0..3 {
            let mut e =
                TerminalEntry::new(format!("u-{n}"), leaf, format!("L{n}"), "cmd".into(), 1);
            // Older entries get earlier spawn timestamps.
            e.spawned_at = Utc::now() - chrono::Duration::seconds((10 - n) as i64);
            write_terminal(&dir, &e).unwrap();
        }
        let listed = list_terminals(&dir).unwrap();
        assert_eq!(listed.len(), 3);
        for window in listed.windows(2) {
            assert!(window[0].spawned_at <= window[1].spawned_at);
        }
    }

    #[test]
    fn mark_closed_updates_status_and_heartbeat() {
        let dir = tmp_pm_dir();
        let leaf = LeafId::new(TypePrefix::Task, 1);
        let e = TerminalEntry::new("u-c".into(), leaf, "L".into(), "cmd".into(), 1);
        write_terminal(&dir, &e).unwrap();
        mark_terminal_closed(&dir, "u-c").unwrap();
        let back = load_terminal(&dir, "u-c").unwrap();
        assert_eq!(back.status, TerminalStatus::Closed);
        assert!(back.last_heartbeat >= e.last_heartbeat);
    }

    #[test]
    fn mark_closed_is_idempotent_for_missing_entries() {
        let dir = tmp_pm_dir();
        // No write first; mark should silently succeed.
        mark_terminal_closed(&dir, "u-missing").unwrap();
    }

    #[test]
    fn purge_marks_dead_when_not_deleting() {
        let dir = tmp_pm_dir();
        let leaf = LeafId::new(TypePrefix::Task, 1);
        let mut e = TerminalEntry::new("u-d".into(), leaf, "L".into(), "cmd".into(), 1);
        // Backdate the heartbeat past TTL.
        e.last_heartbeat = Utc::now() - chrono::Duration::seconds(300);
        write_terminal(&dir, &e).unwrap();

        let acted = purge_dead_terminals(&dir, false).unwrap();
        assert_eq!(acted, vec!["u-d".to_string()]);
        let back = load_terminal(&dir, "u-d").unwrap();
        assert_eq!(back.status, TerminalStatus::Dead);
    }

    #[test]
    fn purge_deletes_when_asked() {
        let dir = tmp_pm_dir();
        let leaf = LeafId::new(TypePrefix::Task, 1);
        let mut e = TerminalEntry::new("u-del".into(), leaf, "L".into(), "cmd".into(), 1);
        e.last_heartbeat = Utc::now() - chrono::Duration::seconds(300);
        write_terminal(&dir, &e).unwrap();

        let acted = purge_dead_terminals(&dir, true).unwrap();
        assert_eq!(acted, vec!["u-del".to_string()]);
        assert!(load_terminal(&dir, "u-del").is_none());
    }
}
