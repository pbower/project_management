//! Disk layout primitives for the v2 `.pm/` directory.
//!
//! Owns the on-disk shape:
//!
//! ```text
//! .pm/
//! ├── state.json
//! ├── aliases.json
//! ├── events.log         # touched but unused until Phase 6
//! ├── locks/
//! ├── projects/
//! ├── products/          # orphan products
//! ├── epics/             # orphan epics
//! ├── tasks/             # orphan tasks
//! ├── subtasks/          # orphan subtasks
//! └── milestones/        # cross-project milestones
//! ```
//!
//! Each ticket lives at a path composed of its address chain, with each
//! directory named after the corresponding `LeafId`. For an address
//! `PRJ1-PRD3-EPC7-TSK22` the directory is
//! `projects/PRJ1/products/PRD3/epics/EPC7/tasks/TSK22/`. `LeafId`s are
//! unique by construction, so there's no name-clash question.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::id::{AddressId, LeafId, TypePrefix};
use super::state::atomic_write;

/// Conventional root folder name under which all PM state lives.
pub const PM_DIR_NAME: &str = ".pm";

/// Names of the type-folders at the root of `.pm/` for orphan-scoped items.
pub const TYPE_FOLDER_ROOTS: &[(TypePrefix, &str)] = &[
    (TypePrefix::Project, "projects"),
    (TypePrefix::Product, "products"),
    (TypePrefix::Epic, "epics"),
    (TypePrefix::Task, "tasks"),
    (TypePrefix::Subtask, "subtasks"),
    (TypePrefix::Milestone, "milestones"),
];

/// Owns paths and provides scaffolding for a `.pm/` directory.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Absolute path to the `.pm/` root.
    pub root: PathBuf,
}

impl Layout {
    /// Build a layout rooted at `<base>/.pm/`.
    pub fn under(base: impl AsRef<Path>) -> Self {
        Layout {
            root: base.as_ref().join(PM_DIR_NAME),
        }
    }

    /// Build a layout rooted at an explicit `.pm/` directory (the directory
    /// itself, not its parent).
    pub fn at(root: impl AsRef<Path>) -> Self {
        Layout {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn state_path(&self) -> PathBuf {
        self.root.join("state.json")
    }
    pub fn aliases_path(&self) -> PathBuf {
        self.root.join("aliases.json")
    }
    pub fn events_log_path(&self) -> PathBuf {
        self.root.join("events.log")
    }
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// Path to a top-level type folder (e.g. `tasks/` for orphan tasks).
    pub fn type_folder_root(&self, prefix: TypePrefix) -> PathBuf {
        self.root.join(prefix.type_folder())
    }

    /// Initialise the layout if it does not already exist. Creates `.pm/` and
    /// all six type-folder roots plus `locks/`, and writes empty `state.json`,
    /// `aliases.json`, and `events.log` files. Idempotent.
    pub fn init(&self) -> Result<(), LayoutError> {
        fs::create_dir_all(&self.root).map_err(LayoutError::Io)?;
        for (_prefix, folder) in TYPE_FOLDER_ROOTS {
            fs::create_dir_all(self.root.join(folder)).map_err(LayoutError::Io)?;
        }
        fs::create_dir_all(self.locks_dir()).map_err(LayoutError::Io)?;

        // events.log is touch-only; locked write is Phase 6 territory.
        let events = self.events_log_path();
        if !events.exists() {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&events)
                .map_err(LayoutError::Io)?;
        }

        // state.json and aliases.json: only create if missing.
        let state = self.state_path();
        if !state.exists() {
            let fresh = super::state::State::fresh();
            let json = serde_json::to_string_pretty(&fresh).map_err(LayoutError::Parse)?;
            atomic_write(&state, json.as_bytes()).map_err(LayoutError::Io)?;
        }
        let aliases = self.aliases_path();
        if !aliases.exists() {
            atomic_write(&aliases, b"{}").map_err(LayoutError::Io)?;
        }
        Ok(())
    }

    /// True if `.pm/state.json` exists (a reasonable proxy for "initialised").
    pub fn is_initialised(&self) -> bool {
        self.state_path().exists()
    }

    /// Compute the relative directory path for a ticket given its full
    /// address-id. Each segment becomes `<type-folder>/<LeafId>/`. The path
    /// returned is relative to `.pm/`.
    ///
    /// Example: address `PRJ1-PRD3-EPC7-TSK22` ->
    /// `projects/PRJ1/products/PRD3/epics/EPC7/tasks/TSK22`.
    pub fn directory_for(&self, address: &AddressId) -> PathBuf {
        let mut p = PathBuf::new();
        for leaf in address.segments() {
            p.push(leaf.prefix().type_folder());
            p.push(leaf.to_string());
        }
        p
    }

    /// Compute the on-disk directory for an orphan leaf. Orphans live directly
    /// under the root type folder, named by their `LeafId`.
    pub fn orphan_directory_for(&self, leaf: LeafId) -> PathBuf {
        let mut p = PathBuf::new();
        p.push(leaf.prefix().type_folder());
        p.push(leaf.to_string());
        p
    }

    /// Create a ticket directory plus its `artifacts/` subdirectory, relative
    /// to `.pm/`. Returns the absolute path. Idempotent.
    pub fn ensure_node_path(&self, rel: &Path) -> Result<PathBuf, LayoutError> {
        let abs = self.root.join(rel);
        fs::create_dir_all(&abs).map_err(LayoutError::Io)?;
        fs::create_dir_all(abs.join("artifacts")).map_err(LayoutError::Io)?;
        Ok(abs)
    }
}

#[derive(Debug)]
pub enum LayoutError {
    Io(io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::Io(e) => write!(f, "layout io error: {e}"),
            LayoutError::Parse(e) => write!(f, "layout parse error: {e}"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::IdInput;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-layout-{}-{}",
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
    fn init_creates_full_layout() {
        let base = tmp_dir();
        let layout = Layout::under(&base);
        layout.init().unwrap();

        assert!(layout.root.is_dir());
        assert!(layout.state_path().is_file());
        assert!(layout.aliases_path().is_file());
        assert!(layout.events_log_path().is_file());
        assert!(layout.locks_dir().is_dir());

        for (_prefix, folder) in TYPE_FOLDER_ROOTS {
            assert!(
                layout.root.join(folder).is_dir(),
                "missing type folder: {folder}"
            );
        }
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn init_is_idempotent() {
        let base = tmp_dir();
        let layout = Layout::under(&base);
        layout.init().unwrap();
        // Plant a sentinel inside state.json to verify init does not clobber.
        fs::write(
            layout.state_path(),
            r#"{"sentinel":true,"next":{"TSK":99}}"#,
        )
        .unwrap();
        layout.init().unwrap();
        let after = fs::read_to_string(layout.state_path()).unwrap();
        assert!(
            after.contains("sentinel"),
            "init clobbered existing state.json"
        );
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn directory_for_full_chain() {
        let layout = Layout::under("/tmp");
        let addr: AddressId = "PRJ1-PRD3-EPC7-TSK22".parse().unwrap();
        let path = layout.directory_for(&addr);
        assert_eq!(
            path,
            PathBuf::from("projects/PRJ1/products/PRD3/epics/EPC7/tasks/TSK22"),
        );
    }

    #[test]
    fn directory_for_with_subtask() {
        let layout = Layout::under("/tmp");
        let addr: AddressId = "PRJ1-PRD3-EPC7-TSK22-SBT1".parse().unwrap();
        let path = layout.directory_for(&addr);
        assert_eq!(
            path,
            PathBuf::from("projects/PRJ1/products/PRD3/epics/EPC7/tasks/TSK22/subtasks/SBT1"),
        );
    }

    #[test]
    fn orphan_directory_for_root_task() {
        let layout = Layout::under("/tmp");
        let input: IdInput = "TSK15".parse().unwrap();
        let path = layout.orphan_directory_for(input.leaf());
        assert_eq!(path, PathBuf::from("tasks/TSK15"));
    }

    #[test]
    fn ensure_node_path_creates_artifacts_subdir() {
        let base = tmp_dir();
        let layout = Layout::under(&base);
        layout.init().unwrap();
        let rel = PathBuf::from("tasks/TSK1");
        let abs = layout.ensure_node_path(&rel).unwrap();
        assert!(abs.is_dir());
        assert!(abs.join("artifacts").is_dir());
        fs::remove_dir_all(&base).ok();
    }
}
