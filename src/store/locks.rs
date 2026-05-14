//! Advisory lock protocol at `.pm/locks/<LeafId>.lock`.
//!
//! A lock is a claim by an agent or human on a ticket, recorded as a JSON
//! file. Locks are advisory: a soft lock warns on overlap but never blocks a
//! second checkout, so the activity feed - not the filesystem - is the real
//! coordination surface.
//!
//! A lock carries a TTL and a `last_heartbeat`. When the heartbeat goes stale
//! (no refresh within the TTL), the lock is reaped by `pm locks` or
//! `pm doctor`. It also records the `base_commit` at checkout time so checkin
//! can squash every commit made during the checkout span into one.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::id::LeafId;
use super::layout::Layout;
use super::state::atomic_write;

/// Default time-to-live for a lock: 30 minutes. A holder is expected to
/// refresh the heartbeat well inside this window (PM_DESIGN.md Section 7.1).
pub const DEFAULT_TTL_SECONDS: u64 = 1800;

/// How strictly a lock is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LockMode {
    /// Overlap warns but does not block. The default.
    Soft,
    /// Overlap blocks a second checkout. Opt-in (per-product, wired later).
    Hard,
}

impl Default for LockMode {
    fn default() -> Self { LockMode::Soft }
}

/// One lock file's contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockFile {
    /// The ticket this lock claims.
    pub id: LeafId,
    /// Actor holding the lock - `PM_AGENT_ID` or `claude-<host>-<pid>`.
    pub agent: String,
    /// Process id of the acquiring `pm` invocation.
    pub pid: u32,
    /// Host the lock was acquired on.
    pub host: String,
    /// When the lock was first taken.
    pub started_at: DateTime<Utc>,
    /// Optional intent string describing the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Time-to-live in seconds; the lock is stale once `last_heartbeat`
    /// plus this window has passed.
    pub ttl_seconds: u64,
    /// Last heartbeat. Refreshed by `pm heartbeat`; compared against the TTL
    /// to decide staleness.
    pub last_heartbeat: DateTime<Utc>,
    /// Soft or hard.
    #[serde(default)]
    pub mode: LockMode,
    /// Git commit hash at checkout time. checkin squashes every commit since
    /// this one into a single commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
}

impl LockFile {
    /// Build a fresh lock for `id`. The actor, pid, and host are resolved from
    /// the environment; `started_at` and `last_heartbeat` are both set to now.
    pub fn new(
        id: LeafId,
        intent: Option<String>,
        ttl_seconds: u64,
        mode: LockMode,
        base_commit: Option<String>,
    ) -> Self {
        let now = Utc::now();
        let host = gethostname::gethostname().to_string_lossy().into_owned();
        LockFile {
            id,
            agent: super::events::actor(),
            pid: std::process::id(),
            host: if host.trim().is_empty() { "host".into() } else { host },
            started_at: now,
            intent,
            ttl_seconds,
            last_heartbeat: now,
            mode,
            base_commit,
        }
    }

    /// True once `last_heartbeat` plus the TTL window has passed.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        now - self.last_heartbeat > Duration::seconds(self.ttl_seconds as i64)
    }
}

/// What happened when [`acquire`] ran.
#[derive(Debug, Clone, PartialEq)]
pub enum AcquireOutcome {
    /// Lock taken cleanly - no prior holder, the prior holder was stale, or
    /// the same agent re-checked-out.
    Acquired,
    /// A live soft lock by a different agent was overwritten. The new lock is
    /// written; the previous holder is returned so the caller can warn.
    Overlapped { previous: LockFile },
    /// A live hard lock by a different agent blocked the acquire. Nothing was
    /// written.
    Blocked { holder: LockFile },
}

/// Errors emitted by the locks layer.
#[derive(Debug)]
pub enum LockError {
    Io(std::io::Error),
    Serialise(serde_json::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Io(e) => write!(f, "lock io: {e}"),
            LockError::Serialise(e) => write!(f, "lock serialise: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Result type used across the locks layer.
pub type LockResult<T> = Result<T, LockError>;

/// Path of the lock file for `id` under `pm_dir`.
fn lock_path(pm_dir: &Path, id: LeafId) -> std::path::PathBuf {
    Layout::at(pm_dir).locks_dir().join(format!("{id}.lock"))
}

/// Read the lock for `id`, if one exists. A malformed lock file reads as
/// `None` rather than an error - a corrupt lock should not wedge the workspace.
pub fn read(pm_dir: &Path, id: LeafId) -> LockResult<Option<LockFile>> {
    let path = lock_path(pm_dir, id);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(LockError::Io)?;
    Ok(serde_json::from_str::<LockFile>(&raw).ok())
}

/// Write `lock` to disk, replacing any existing lock for the same id.
fn write(pm_dir: &Path, lock: &LockFile) -> LockResult<()> {
    let locks_dir = Layout::at(pm_dir).locks_dir();
    fs::create_dir_all(&locks_dir).map_err(LockError::Io)?;
    let json = serde_json::to_string_pretty(lock).map_err(LockError::Serialise)?;
    atomic_write(&lock_path(pm_dir, lock.id), json.as_bytes()).map_err(LockError::Io)
}

/// Acquire `lock`. The outcome depends on any existing lock for the same id:
///
/// - none, stale, or held by the same agent -> the new lock is written,
///   [`AcquireOutcome::Acquired`].
/// - a live soft lock by a different agent -> the new lock is written
///   (last-writer-wins, advisory), [`AcquireOutcome::Overlapped`].
/// - a live hard lock by a different agent -> nothing is written,
///   [`AcquireOutcome::Blocked`].
pub fn acquire(pm_dir: &Path, lock: &LockFile, now: DateTime<Utc>) -> LockResult<AcquireOutcome> {
    match read(pm_dir, lock.id)? {
        None => {
            write(pm_dir, lock)?;
            Ok(AcquireOutcome::Acquired)
        }
        Some(existing) if existing.is_stale(now) || existing.agent == lock.agent => {
            write(pm_dir, lock)?;
            Ok(AcquireOutcome::Acquired)
        }
        Some(existing) => match existing.mode {
            LockMode::Hard => Ok(AcquireOutcome::Blocked { holder: existing }),
            LockMode::Soft => {
                write(pm_dir, lock)?;
                Ok(AcquireOutcome::Overlapped { previous: existing })
            }
        },
    }
}

/// Release the lock for `id`. Returns whether a lock file was present.
pub fn release(pm_dir: &Path, id: LeafId) -> LockResult<bool> {
    let path = lock_path(pm_dir, id);
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).map_err(LockError::Io)?;
    Ok(true)
}

/// List every live and stale lock under `pm_dir`, sorted by id. Malformed
/// lock files are skipped.
pub fn list(pm_dir: &Path) -> LockResult<Vec<LockFile>> {
    let locks_dir = Layout::at(pm_dir).locks_dir();
    if !locks_dir.exists() {
        return Ok(Vec::new());
    }
    let mut locks = Vec::new();
    for entry in fs::read_dir(&locks_dir).map_err(LockError::Io)? {
        let entry = entry.map_err(LockError::Io)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lock") {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(lock) = serde_json::from_str::<LockFile>(&raw) {
                locks.push(lock);
            }
        }
    }
    locks.sort_by_key(|l| l.id);
    Ok(locks)
}

/// Remove every lock whose heartbeat has gone stale relative to `now`.
/// Returns the ids that were reaped, sorted.
pub fn reap_stale(pm_dir: &Path, now: DateTime<Utc>) -> LockResult<Vec<LeafId>> {
    let mut reaped = Vec::new();
    for lock in list(pm_dir)? {
        if lock.is_stale(now) {
            release(pm_dir, lock.id)?;
            reaped.push(lock.id);
        }
    }
    reaped.sort();
    Ok(reaped)
}

/// Bump `last_heartbeat` on the lock for `id` to `now`. Returns whether a
/// lock was present to refresh.
pub fn refresh_heartbeat(pm_dir: &Path, id: LeafId, now: DateTime<Utc>) -> LockResult<bool> {
    match read(pm_dir, id)? {
        None => Ok(false),
        Some(mut lock) => {
            lock.last_heartbeat = now;
            write(pm_dir, &lock)?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-locks-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tsk(n: u64) -> LeafId { LeafId::new(TypePrefix::Task, n) }

    /// Build a lock with explicit agent and timestamps for deterministic tests.
    fn lock_for(id: LeafId, agent: &str, started: DateTime<Utc>, heartbeat: DateTime<Utc>, mode: LockMode) -> LockFile {
        LockFile {
            id,
            agent: agent.to_string(),
            pid: 1234,
            host: "testhost".into(),
            started_at: started,
            intent: None,
            ttl_seconds: DEFAULT_TTL_SECONDS,
            last_heartbeat: heartbeat,
            mode,
            base_commit: None,
        }
    }

    #[test]
    fn acquire_on_empty_writes_lock() {
        let dir = tmp_dir();
        let now = Utc::now();
        let lock = lock_for(tsk(7), "claude-be", now, now, LockMode::Soft);
        assert_eq!(acquire(&dir, &lock, now).unwrap(), AcquireOutcome::Acquired);
        let back = read(&dir, tsk(7)).unwrap().unwrap();
        assert_eq!(back.agent, "claude-be");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn acquire_over_live_soft_lock_overlaps_and_overwrites() {
        let dir = tmp_dir();
        let now = Utc::now();
        let first = lock_for(tsk(7), "claude-be", now, now, LockMode::Soft);
        acquire(&dir, &first, now).unwrap();

        let second = lock_for(tsk(7), "claude-fe", now, now, LockMode::Soft);
        match acquire(&dir, &second, now).unwrap() {
            AcquireOutcome::Overlapped { previous } => assert_eq!(previous.agent, "claude-be"),
            other => panic!("expected Overlapped, got {other:?}"),
        }
        // Last-writer-wins: the recorded holder is now claude-fe.
        assert_eq!(read(&dir, tsk(7)).unwrap().unwrap().agent, "claude-fe");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn acquire_over_live_hard_lock_is_blocked() {
        let dir = tmp_dir();
        let now = Utc::now();
        let first = lock_for(tsk(7), "claude-be", now, now, LockMode::Hard);
        acquire(&dir, &first, now).unwrap();

        let second = lock_for(tsk(7), "claude-fe", now, now, LockMode::Hard);
        match acquire(&dir, &second, now).unwrap() {
            AcquireOutcome::Blocked { holder } => assert_eq!(holder.agent, "claude-be"),
            other => panic!("expected Blocked, got {other:?}"),
        }
        // Nothing was overwritten - claude-be still holds it.
        assert_eq!(read(&dir, tsk(7)).unwrap().unwrap().agent, "claude-be");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn acquire_over_stale_lock_just_acquires() {
        let dir = tmp_dir();
        let now = Utc::now();
        // Heartbeat is two TTL windows in the past - well stale.
        let stale_hb = now - Duration::seconds((DEFAULT_TTL_SECONDS * 2) as i64);
        let stale = lock_for(tsk(7), "claude-be", stale_hb, stale_hb, LockMode::Hard);
        acquire(&dir, &stale, now).unwrap();

        let fresh = lock_for(tsk(7), "claude-fe", now, now, LockMode::Soft);
        // Even a hard stale lock does not block - it is reaped by the overwrite.
        assert_eq!(acquire(&dir, &fresh, now).unwrap(), AcquireOutcome::Acquired);
        assert_eq!(read(&dir, tsk(7)).unwrap().unwrap().agent, "claude-fe");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn same_agent_recheckout_just_acquires() {
        let dir = tmp_dir();
        let now = Utc::now();
        let first = lock_for(tsk(7), "claude-be", now, now, LockMode::Soft);
        acquire(&dir, &first, now).unwrap();
        let again = lock_for(tsk(7), "claude-be", now, now, LockMode::Soft);
        assert_eq!(acquire(&dir, &again, now).unwrap(), AcquireOutcome::Acquired);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn release_removes_lock() {
        let dir = tmp_dir();
        let now = Utc::now();
        acquire(&dir, &lock_for(tsk(7), "claude-be", now, now, LockMode::Soft), now).unwrap();
        assert!(release(&dir, tsk(7)).unwrap());
        assert!(read(&dir, tsk(7)).unwrap().is_none());
        // Releasing again is a no-op that reports "not present".
        assert!(!release(&dir, tsk(7)).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_returns_all_locks_sorted() {
        let dir = tmp_dir();
        let now = Utc::now();
        acquire(&dir, &lock_for(tsk(44), "claude-fe", now, now, LockMode::Soft), now).unwrap();
        acquire(&dir, &lock_for(tsk(7), "claude-be", now, now, LockMode::Soft), now).unwrap();
        let locks = list(&dir).unwrap();
        let ids: Vec<String> = locks.iter().map(|l| l.id.to_string()).collect();
        assert_eq!(ids, vec!["TSK7", "TSK44"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reap_stale_removes_only_expired_locks() {
        let dir = tmp_dir();
        let now = Utc::now();
        let stale_hb = now - Duration::seconds((DEFAULT_TTL_SECONDS + 60) as i64);
        // One fresh, one stale.
        acquire(&dir, &lock_for(tsk(7), "claude-be", now, now, LockMode::Soft), now).unwrap();
        acquire(&dir, &lock_for(tsk(44), "claude-fe", stale_hb, stale_hb, LockMode::Soft), now).unwrap();

        let reaped = reap_stale(&dir, now).unwrap();
        assert_eq!(reaped, vec![tsk(44)]);
        // The fresh lock survives, the stale one is gone.
        assert!(read(&dir, tsk(7)).unwrap().is_some());
        assert!(read(&dir, tsk(44)).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_heartbeat_bumps_timestamp() {
        let dir = tmp_dir();
        let now = Utc::now();
        let old_hb = now - Duration::seconds(120);
        acquire(&dir, &lock_for(tsk(7), "claude-be", old_hb, old_hb, LockMode::Soft), now).unwrap();

        assert!(refresh_heartbeat(&dir, tsk(7), now).unwrap());
        assert_eq!(read(&dir, tsk(7)).unwrap().unwrap().last_heartbeat, now);
        // Refreshing a missing lock reports "not present".
        assert!(!refresh_heartbeat(&dir, tsk(99), now).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_stale_respects_ttl_window() {
        let now = Utc::now();
        let lock = lock_for(tsk(7), "claude-be", now, now, LockMode::Soft);
        assert!(!lock.is_stale(now));
        assert!(!lock.is_stale(now + Duration::seconds(DEFAULT_TTL_SECONDS as i64 - 1)));
        assert!(lock.is_stale(now + Duration::seconds(DEFAULT_TTL_SECONDS as i64 + 1)));
    }

    #[test]
    fn lockfile_json_round_trip() {
        let now = Utc::now();
        let mut lock = lock_for(tsk(7), "claude-be", now, now, LockMode::Soft);
        lock.intent = Some("implement TTL".into());
        lock.base_commit = Some("abc1234".into());
        let json = serde_json::to_string(&lock).unwrap();
        let back: LockFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, lock);
    }
}
