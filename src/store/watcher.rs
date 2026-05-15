//! File-watcher that auto-sweeps `artifacts/` directories on change.
//!
//! Wraps `notify-debouncer-mini`. Callers register one or more
//! `(artifacts_dir, node)` pairs; the watcher fires a debounced sweep through
//! [`crate::store::artifacts::sweep_dir`] whenever something inside a watched
//! directory changes. The debounce window matches PM_DESIGN.md Section 10.3
//! (250ms).
//!
//! Dropping an [`ArtifactsWatcher`] stops the underlying watcher thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};

use super::artifacts::{sweep_dir, ArtifactError, SweepReport};
use super::id::LeafId;

/// Default debounce window. Picked to absorb rapid editor save bursts while
/// still feeling instant to a human watching a side panel.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(250);

/// What to sweep when a watched directory changes.
struct WatchTarget {
    node: LeafId,
}

/// Last sweep outcome per directory, surfaced to callers via [`take_reports`].
#[derive(Debug, Default, Clone)]
struct Reports {
    by_dir: HashMap<PathBuf, SweepReport>,
}

/// Auto-sweeping watcher.
pub struct ArtifactsWatcher {
    // Holding the debouncer keeps the OS watcher thread alive; dropping us
    // drops it.
    debouncer: Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
    targets: Arc<Mutex<HashMap<PathBuf, WatchTarget>>>,
    reports: Arc<Mutex<Reports>>,
}

impl ArtifactsWatcher {
    /// Build a new watcher with the default 250ms debounce.
    pub fn new() -> Result<Self, ArtifactError> {
        Self::with_debounce(DEFAULT_DEBOUNCE)
    }

    /// Build a new watcher with a custom debounce window. Useful for tests
    /// that want to verify the sweep landed within a tight bound.
    pub fn with_debounce(debounce: Duration) -> Result<Self, ArtifactError> {
        let targets: Arc<Mutex<HashMap<PathBuf, WatchTarget>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reports: Arc<Mutex<Reports>> = Arc::new(Mutex::new(Reports::default()));

        let targets_for_callback = Arc::clone(&targets);
        let reports_for_callback = Arc::clone(&reports);
        let debouncer = new_debouncer(debounce, move |res: DebounceEventResult| {
            if let Ok(events) = res {
                let targets_guard = match targets_for_callback.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                // A single debounce window can produce events for several
                // watched roots; sweep each at most once per window.
                let mut already_swept: HashMap<PathBuf, ()> = HashMap::new();
                for event in &events {
                    for (watched, target) in targets_guard.iter() {
                        if already_swept.contains_key(watched) {
                            continue;
                        }
                        if event_belongs_to(&event.path, watched) {
                            if let Ok((report, _idx)) = sweep_dir(watched, target.node) {
                                if let Ok(mut reports) = reports_for_callback.lock() {
                                    reports.by_dir.insert(watched.clone(), report);
                                }
                            }
                            already_swept.insert(watched.clone(), ());
                        }
                    }
                }
            }
        })
        .map_err(map_notify_error)?;

        Ok(ArtifactsWatcher {
            debouncer,
            targets,
            reports,
        })
    }

    /// Start watching `artifacts_dir`. New or changed files trigger a sweep
    /// against `node`.
    pub fn watch(&mut self, artifacts_dir: &Path, node: LeafId) -> Result<(), ArtifactError> {
        self.debouncer
            .watcher()
            .watch(artifacts_dir, RecursiveMode::NonRecursive)
            .map_err(map_notify_error)?;
        let mut guard = self.targets.lock().expect("targets mutex poisoned");
        guard.insert(artifacts_dir.to_path_buf(), WatchTarget { node });
        Ok(())
    }

    /// Stop watching `artifacts_dir`. Idempotent: re-unwatching a path is a
    /// no-op.
    pub fn unwatch(&mut self, artifacts_dir: &Path) -> Result<(), ArtifactError> {
        let _ = self.debouncer.watcher().unwatch(artifacts_dir);
        let mut guard = self.targets.lock().expect("targets mutex poisoned");
        guard.remove(artifacts_dir);
        Ok(())
    }

    /// Drain and return any sweep reports recorded since the last call. The
    /// internal report buffer is cleared. Callers that want a UI to surface
    /// "5 files swept" notifications poll this on a timer.
    pub fn take_reports(&self) -> HashMap<PathBuf, SweepReport> {
        let mut guard = self.reports.lock().expect("reports mutex poisoned");
        std::mem::take(&mut guard.by_dir)
    }
}

/// Decide whether a notify event path is inside the watched directory. The
/// notify backend usually delivers absolute paths but on Linux can deliver
/// paths relative to the watched root; treat both as a match.
fn event_belongs_to(event_path: &Path, watched: &Path) -> bool {
    if event_path == watched {
        return true;
    }
    if event_path.starts_with(watched) {
        return true;
    }
    // A relative event path is interpreted as living inside the watched root.
    if event_path.is_relative() {
        return true;
    }
    false
}

/// Wrap a notify error in [`ArtifactError`]. notify's error type doesn't
/// implement Into<io::Error> cleanly, so we lift through io::Error::other.
fn map_notify_error(e: notify_debouncer_mini::notify::Error) -> ArtifactError {
    ArtifactError::Io(std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::artifacts::{ArtifactsIndex, ARTIFACTS_MD};
    use crate::store::id::TypePrefix;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Instant;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-watcher-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn task_leaf() -> LeafId {
        LeafId::new(TypePrefix::Task, 7)
    }

    /// Detection deadline that the watcher tests assert against. inotify on
    /// Linux and ReadDirectoryChangesW on Windows are push-based and reliably
    /// fire well under 500ms; FSEvents on macOS adds coalescing latency that
    /// makes shared CI runners flaky at that budget. macOS gets a wider
    /// window so the test still verifies the functional path without
    /// flapping.
    const DETECT_DEADLINE: Duration = if cfg!(target_os = "macos") {
        Duration::from_millis(3000)
    } else {
        Duration::from_millis(500)
    };

    /// Poll the index file until either the predicate matches or the deadline
    /// passes. Returns the time it took for the predicate to match, or None
    /// if it never did.
    fn wait_for<F: Fn(&ArtifactsIndex) -> bool>(
        index_path: &Path,
        deadline: Duration,
        pred: F,
    ) -> Option<Duration> {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if let Ok(idx) = ArtifactsIndex::load(index_path) {
                if pred(&idx) {
                    return Some(start.elapsed());
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn added_file_appears_in_index() {
        let dir = tmp_dir();
        let artifacts = dir.join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        // Pre-create the index so the watcher has a baseline.
        let (_, _) = sweep_dir(&artifacts, task_leaf()).unwrap();

        let mut watcher =
            ArtifactsWatcher::with_debounce(Duration::from_millis(80)).expect("create watcher");
        watcher.watch(&artifacts, task_leaf()).expect("watch dir");

        // Small grace period so the OS watcher is fully wired.
        std::thread::sleep(Duration::from_millis(40));
        fs::write(artifacts.join("schema.png"), b"PNG").unwrap();

        let index_path = artifacts.join(ARTIFACTS_MD);
        let elapsed = wait_for(&index_path, DETECT_DEADLINE, |idx| {
            idx.find("schema.png").is_some()
        });
        assert!(
            elapsed.is_some(),
            "watcher did not register schema.png within {:?}",
            DETECT_DEADLINE,
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removed_file_drops_out_of_index() {
        let dir = tmp_dir();
        let artifacts = dir.join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join("bench.csv"), b"a,b").unwrap();
        let (_, _) = sweep_dir(&artifacts, task_leaf()).unwrap();

        let mut watcher =
            ArtifactsWatcher::with_debounce(Duration::from_millis(80)).expect("create watcher");
        watcher.watch(&artifacts, task_leaf()).expect("watch dir");
        std::thread::sleep(Duration::from_millis(40));

        fs::remove_file(artifacts.join("bench.csv")).unwrap();
        let index_path = artifacts.join(ARTIFACTS_MD);
        let elapsed = wait_for(&index_path, DETECT_DEADLINE, |idx| {
            idx.find("bench.csv").is_none()
        });
        assert!(
            elapsed.is_some(),
            "watcher did not remove bench.csv within {:?}",
            DETECT_DEADLINE,
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unwatch_stops_further_sweeps() {
        let dir = tmp_dir();
        let artifacts = dir.join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        let (_, _) = sweep_dir(&artifacts, task_leaf()).unwrap();

        let mut watcher =
            ArtifactsWatcher::with_debounce(Duration::from_millis(80)).expect("create watcher");
        watcher.watch(&artifacts, task_leaf()).expect("watch dir");
        std::thread::sleep(Duration::from_millis(40));
        watcher.unwatch(&artifacts).expect("unwatch dir");
        std::thread::sleep(Duration::from_millis(40));

        // After unwatching, a new file must not be picked up.
        fs::write(artifacts.join("late.txt"), b"x").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let idx = ArtifactsIndex::load(&artifacts.join(ARTIFACTS_MD)).unwrap();
        assert!(
            idx.find("late.txt").is_none(),
            "unwatched directory should not auto-sweep"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn drop_stops_watcher() {
        let dir = tmp_dir();
        let artifacts = dir.join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        let (_, _) = sweep_dir(&artifacts, task_leaf()).unwrap();

        {
            let mut watcher =
                ArtifactsWatcher::with_debounce(Duration::from_millis(80)).expect("create watcher");
            watcher.watch(&artifacts, task_leaf()).expect("watch dir");
            // Drop the watcher when this block exits.
        }
        // Without a live watcher, a new file should not appear in the index.
        std::thread::sleep(Duration::from_millis(40));
        fs::write(artifacts.join("post-drop.txt"), b"x").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let idx = ArtifactsIndex::load(&artifacts.join(ARTIFACTS_MD)).unwrap();
        assert!(
            idx.find("post-drop.txt").is_none(),
            "dropped watcher must not continue sweeping"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
