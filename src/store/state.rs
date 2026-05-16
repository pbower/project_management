//! `state.json` - the in-memory and on-disk index of all known ticket IDs.
//!
//! `state.json` is the canonical id-to-path index for a `.pm/` directory. It
//! holds:
//!
//! - `next`: monotonic counters per type prefix for allocating new IDs.
//! - `tombstones`: per-type lists of leaf numbers that have been allocated and
//!   then deleted; never reused.
//! - `items`: a map from leaf id to its on-disk path (relative to `.pm/`).
//! - `templates`: named [`TaskTemplate`] presets used by the `pm template ...`
//!   commands for rapid task creation.
//!
//! The file is treated as a derived cache for the ticket tree: it can always
//! be rebuilt from the filesystem via `pm doctor`. State writes are atomic
//! (temp-file + rename).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::task::TaskTemplate;

use super::id::{IdParseError, LeafId, TypePrefix};

/// On-disk and in-memory representation of `state.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Next monotonic counter per type. New IDs are minted as `prefix + next[prefix]`,
    /// then `next[prefix]` is incremented and the previous value moves to `items`.
    #[serde(default)]
    pub next: BTreeMap<TypePrefix, u64>,
    /// Per-type tombstoned numbers. Never reused.
    #[serde(default)]
    pub tombstones: BTreeMap<TypePrefix, BTreeSet<u64>>,
    /// Map from leaf id to on-disk entry. Path is relative to `.pm/`.
    #[serde(default)]
    pub items: BTreeMap<LeafId, ItemEntry>,
    /// Named task-creation templates. Carried in state.json so the v2
    /// workspace owns both the ID index and the templates the UI uses.
    #[serde(default)]
    pub templates: Vec<TaskTemplate>,
}

/// Per-ticket index entry. Currently only carries the relative path; future
/// phases may add cached metadata here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemEntry {
    /// Relative to the `.pm/` root.
    pub path: PathBuf,
}

impl State {
    /// Build a fresh empty state with the `next` counters initialised to 1 for
    /// every known prefix (counter 0 is reserved by convention).
    pub fn fresh() -> Self {
        let mut next = BTreeMap::new();
        let mut tombstones = BTreeMap::new();
        for prefix in TypePrefix::all() {
            next.insert(*prefix, 1);
            tombstones.insert(*prefix, BTreeSet::new());
        }
        State {
            next,
            tombstones,
            items: BTreeMap::new(),
            templates: Vec::new(),
        }
    }

    /// Load from a `.pm/state.json` path. If the file does not exist, returns
    /// [`State::fresh`]. Other I/O or parse failures are reported.
    pub fn load(state_path: &Path) -> Result<Self, StateError> {
        if !state_path.exists() {
            return Ok(State::fresh());
        }
        let raw = fs::read_to_string(state_path).map_err(StateError::Io)?;
        if raw.trim().is_empty() {
            return Ok(State::fresh());
        }
        let mut state: State = serde_json::from_str(&raw).map_err(StateError::Parse)?;
        // Backfill any missing prefix entries so callers can index into the
        // counters and tombstone sets without checking.
        for prefix in TypePrefix::all() {
            state.next.entry(*prefix).or_insert(1);
            state
                .tombstones
                .entry(*prefix)
                .or_insert_with(BTreeSet::new);
        }
        Ok(state)
    }

    /// Atomically write to `state_path`. Writes to `<state_path>.tmp.<pid>` first,
    /// fsyncs, then renames.
    pub fn save(&self, state_path: &Path) -> Result<(), StateError> {
        let json = serde_json::to_string_pretty(self).map_err(StateError::Parse)?;
        atomic_write(state_path, json.as_bytes()).map_err(StateError::Io)
    }

    /// Allocate the next monotonic id for `prefix`. Skips any tombstoned numbers
    /// the counter happens to be pointing at, then advances past the chosen
    /// number. Returns the newly-minted leaf id.
    pub fn allocate(&mut self, prefix: TypePrefix) -> LeafId {
        let counter = self.next.entry(prefix).or_insert(1);
        let tombs = self.tombstones.entry(prefix).or_insert_with(BTreeSet::new);
        while tombs.contains(counter) {
            *counter = counter.checked_add(1).expect("id counter overflow");
        }
        let chosen = *counter;
        *counter = counter.checked_add(1).expect("id counter overflow");
        LeafId::new(prefix, chosen)
    }

    /// Tombstone a leaf id and remove it from the items index. Idempotent.
    pub fn tombstone(&mut self, leaf: LeafId) {
        self.items.remove(&leaf);
        self.tombstones
            .entry(leaf.prefix())
            .or_insert_with(BTreeSet::new)
            .insert(leaf.number());
    }

    /// True if this leaf was previously allocated and then deleted.
    pub fn is_tombstoned(&self, leaf: LeafId) -> bool {
        self.tombstones
            .get(&leaf.prefix())
            .map(|set| set.contains(&leaf.number()))
            .unwrap_or(false)
    }

    /// Look up the on-disk path for a leaf. Does not consult aliases; callers
    /// that want alias fallback should use [`crate::store::resolver::Resolver`]
    /// (added in a later commit).
    pub fn lookup(&self, leaf: LeafId) -> Option<&ItemEntry> {
        self.items.get(&leaf)
    }

    /// Insert or replace an item entry.
    pub fn insert(&mut self, leaf: LeafId, entry: ItemEntry) {
        self.items.insert(leaf, entry);
    }
}

/// Atomic-write helper used by both `State` and `Aliases`. Writes to
/// `<path>.tmp.<pid>.<nanos>`, fsyncs, then renames. Caller should ensure the
/// parent directory exists.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("tmp.{pid}.{nanos}"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// All ways state.json IO and parsing can fail.
#[derive(Debug)]
pub enum StateError {
    Io(io::Error),
    Parse(serde_json::Error),
    Id(IdParseError),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::Io(e) => write!(f, "state.json io error: {e}"),
            StateError::Parse(e) => write!(f, "state.json parse error: {e}"),
            StateError::Id(e) => write!(f, "state.json id error: {e}"),
        }
    }
}

impl std::error::Error for StateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fresh_initialises_all_counters_to_one() {
        let s = State::fresh();
        for prefix in TypePrefix::all() {
            assert_eq!(s.next.get(prefix).copied(), Some(1));
            assert!(s.tombstones.get(prefix).unwrap().is_empty());
        }
        assert!(s.items.is_empty());
    }

    #[test]
    fn allocate_advances_counter_per_type() {
        let mut s = State::fresh();
        let a = s.allocate(TypePrefix::Task);
        let b = s.allocate(TypePrefix::Task);
        let c = s.allocate(TypePrefix::Subtask);
        assert_eq!(a.to_string(), "TSK1");
        assert_eq!(b.to_string(), "TSK2");
        assert_eq!(c.to_string(), "SBT1");
        assert_eq!(s.next[&TypePrefix::Task], 3);
        assert_eq!(s.next[&TypePrefix::Subtask], 2);
    }

    #[test]
    fn tombstones_skip_reused_numbers() {
        let mut s = State::fresh();
        // Allocate three tasks then tombstone the middle one. Future allocations
        // never produce TSK2 again.
        let _t1 = s.allocate(TypePrefix::Task);
        let t2 = s.allocate(TypePrefix::Task);
        let _t3 = s.allocate(TypePrefix::Task);
        s.tombstone(t2);

        // Manually rewind the counter to simulate state-file edit; allocate
        // should jump past the tombstoned number.
        s.next.insert(TypePrefix::Task, 2);
        let next = s.allocate(TypePrefix::Task);
        assert_eq!(next.to_string(), "TSK3", "must skip tombstoned TSK2");
        // Next allocation should now produce TSK4.
        let after = s.allocate(TypePrefix::Task);
        assert_eq!(after.to_string(), "TSK4");
    }

    #[test]
    fn tombstone_removes_from_items() {
        let mut s = State::fresh();
        let leaf = s.allocate(TypePrefix::Task);
        s.insert(
            leaf,
            ItemEntry {
                path: PathBuf::from("tasks/x/"),
            },
        );
        assert!(s.lookup(leaf).is_some());
        s.tombstone(leaf);
        assert!(s.lookup(leaf).is_none());
        assert!(s.is_tombstoned(leaf));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tmp_dir();
        let path = dir.join("state.json");

        let mut s = State::fresh();
        let leaf = s.allocate(TypePrefix::Task);
        s.insert(
            leaf,
            ItemEntry {
                path: PathBuf::from("tasks/lock-protocol/"),
            },
        );
        let dropped = s.allocate(TypePrefix::Subtask);
        s.tombstone(dropped);

        s.save(&path).unwrap();
        let loaded = State::load(&path).unwrap();

        assert_eq!(loaded.next[&TypePrefix::Task], s.next[&TypePrefix::Task]);
        assert_eq!(
            loaded.next[&TypePrefix::Subtask],
            s.next[&TypePrefix::Subtask]
        );
        assert!(loaded.is_tombstoned(dropped));
        assert_eq!(loaded.lookup(leaf), s.lookup(leaf));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_returns_fresh() {
        let dir = tmp_dir();
        let path = dir.join("does-not-exist.json");
        let s = State::load(&path).unwrap();
        assert!(s.items.is_empty());
        assert_eq!(s.next[&TypePrefix::Task], 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_empty_file_returns_fresh() {
        let dir = tmp_dir();
        let path = dir.join("state.json");
        fs::write(&path, "").unwrap();
        let s = State::load(&path).unwrap();
        assert_eq!(s.next[&TypePrefix::Task], 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_partial_state_backfills_missing_prefixes() {
        let dir = tmp_dir();
        let path = dir.join("state.json");
        // Hand-write a state.json that only has counters for Task and Project.
        fs::write(&path, r#"{ "next": { "TSK": 5, "PRJ": 2 } }"#).unwrap();
        let s = State::load(&path).unwrap();
        // Mentioned prefixes preserved.
        assert_eq!(s.next[&TypePrefix::Task], 5);
        assert_eq!(s.next[&TypePrefix::Project], 2);
        // Missing prefixes backfilled to 1.
        for prefix in TypePrefix::all() {
            assert!(s.next.contains_key(prefix));
            assert!(s.tombstones.contains_key(prefix));
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_replaces_existing() {
        let dir = tmp_dir();
        let path = dir.join("state.json");
        atomic_write(&path, b"hello").unwrap();
        atomic_write(&path, b"world").unwrap();
        let read = fs::read_to_string(&path).unwrap();
        assert_eq!(read, "world");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn templates_round_trip_through_save_and_load() {
        use crate::fields::{Kind, Priority, ProcessStage, Status};
        use crate::task::TaskTemplate;

        let dir = tmp_dir();
        let path = dir.join("state.json");

        let mut s = State::fresh();
        s.templates.push(TaskTemplate {
            name: "spike".to_string(),
            title_template: Some("Spike: {title}".to_string()),
            description_template: Some("Time-boxed exploration.".to_string()),
            tags: vec!["research".to_string()],
            kind: Kind::Task,
            priority_level: Some(Priority::NiceToHave),
            urgency: None,
            process_stage: Some(ProcessStage::Ideation),
            status: Status::Open,
        });
        s.save(&path).unwrap();

        let loaded = State::load(&path).unwrap();
        assert_eq!(loaded.templates.len(), 1);
        let t = &loaded.templates[0];
        assert_eq!(t.name, "spike");
        assert_eq!(t.title_template.as_deref(), Some("Spike: {title}"));
        assert_eq!(t.kind, Kind::Task);
        assert_eq!(t.priority_level, Some(Priority::NiceToHave));
        assert_eq!(t.tags, vec!["research".to_string()]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pre_templates_state_json_loads_with_empty_templates() {
        let dir = tmp_dir();
        let path = dir.join("state.json");
        // A state.json from before the templates field existed.
        fs::write(&path, r#"{ "next": { "TSK": 3 } }"#).unwrap();
        let s = State::load(&path).unwrap();
        assert!(
            s.templates.is_empty(),
            "missing field must backfill to empty Vec"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
