//! Legacy `tasks.json` -> v2 layout migration planner.
//!
//! Reads the existing flat `Vec<Task>` JSON used by v0.9.x and computes the v2
//! disk layout it would produce: new leaf ids, parent linkage, slugs, address
//! forms, target paths. Does not write anything; designed to be run as `pm
//! doctor --plan-migration` and reviewed before the real migration in a later
//! phase wires the write path.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::fields::Kind as OldKind;

use super::id::{AddressId, LeafId, TypePrefix};
use super::layout::Layout;
use super::state::State;

/// Minimal shape of the legacy Task record. We only deserialise the fields the
/// planner needs; everything else passes through.
#[derive(Debug, Clone, Deserialize)]
struct LegacyTask {
    id: u64,
    title: String,
    parent: Option<u64>,
    kind: OldKind,
}

/// Top-level legacy file shape. Older versions stored as a bare array; newer
/// ones may wrap in an object. We try both.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum LegacyDb {
    BareArray(Vec<LegacyTask>),
    Wrapped { tasks: Vec<LegacyTask> },
}

impl LegacyDb {
    fn tasks(self) -> Vec<LegacyTask> {
        match self {
            LegacyDb::BareArray(v) => v,
            LegacyDb::Wrapped { tasks } => tasks,
        }
    }
}

/// One ticket's migration outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStep {
    /// Legacy numeric id.
    pub old_id: u64,
    /// Legacy Kind (Product/Epic/Task/Subtask/Milestone).
    pub old_kind: OldKind,
    /// New v2 leaf id (typed prefix + monotonic number).
    pub new_leaf: LeafId,
    /// New parent leaf, if the legacy ticket had a parent.
    pub new_parent: Option<LeafId>,
    /// Kebab-case slug derived from the title.
    pub slug: String,
    /// Full address chain (root -> leaf). For orphans this is `[leaf]` only.
    pub address: AddressId,
    /// Target directory relative to `.pm/` (e.g. `tasks/lock-protocol`).
    pub target_dir: PathBuf,
    /// Original title (preserved for the new front-matter `title:` field).
    pub title: String,
}

/// A complete migration plan for one legacy database file.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub source: PathBuf,
    pub steps: Vec<MigrationStep>,
    /// Old-ids that referenced a parent that does not exist in the source.
    /// Tickets pointing at these are demoted to orphans in the plan.
    pub dangling_parents: Vec<u64>,
}

impl MigrationPlan {
    /// Read `source` and compute the plan. Does not touch the filesystem
    /// beyond the read.
    pub fn plan(layout: &Layout, source: &Path) -> Result<Self, MigrateError> {
        let raw = fs::read_to_string(source).map_err(MigrateError::Io)?;
        let legacy: LegacyDb = serde_json::from_str(&raw).map_err(MigrateError::Parse)?;
        let mut tasks = legacy.tasks();
        // Stable order: by old id so the plan is deterministic.
        tasks.sort_by_key(|t| t.id);

        let mut state = State::fresh();
        // First pass: allocate every ticket its new leaf id and record the
        // old-id -> new-leaf mapping. Parent resolution happens in a second
        // pass after every leaf exists.
        let mut old_to_new: BTreeMap<u64, LeafId> = BTreeMap::new();
        let mut slugs: BTreeMap<u64, String> = BTreeMap::new();
        for t in &tasks {
            let prefix = map_kind(t.kind);
            let leaf = state.allocate(prefix);
            old_to_new.insert(t.id, leaf);
            slugs.insert(t.id, slugify(&t.title));
        }

        // Detect any parent references that point to old ids not present in
        // the source. Demote those tickets to orphans (parent=None).
        let mut dangling: Vec<u64> = Vec::new();
        let known_ids: std::collections::BTreeSet<u64> = tasks.iter().map(|t| t.id).collect();
        for t in &tasks {
            if let Some(p) = t.parent {
                if !known_ids.contains(&p) && !dangling.contains(&p) {
                    dangling.push(p);
                }
            }
        }

        // Second pass: walk parent chain (resolving via old_to_new) to build
        // the address. Compute target directory from the chain plus slug map.
        let mut steps: Vec<MigrationStep> = Vec::with_capacity(tasks.len());
        for t in &tasks {
            let new_leaf = old_to_new[&t.id];
            let slug = slugs[&t.id].clone();

            let parent_old = t.parent.filter(|p| known_ids.contains(p));
            let new_parent = parent_old.map(|p| old_to_new[&p]);

            let chain = walk_chain(t.id, &tasks, &old_to_new, &known_ids);
            let chain_slugs: Vec<&str> = chain.iter().map(|leaf_old_id| slugs[leaf_old_id].as_str()).collect();
            let chain_leaves: Vec<LeafId> = chain.iter().map(|old| old_to_new[old]).collect();

            let address = AddressId::new(chain_leaves).expect("chain has at least the leaf itself");
            let target_dir = layout.directory_for(&address, &chain_slugs).map_err(MigrateError::Layout)?;

            steps.push(MigrationStep {
                old_id: t.id,
                old_kind: t.kind,
                new_leaf,
                new_parent,
                slug,
                address,
                target_dir,
                title: t.title.clone(),
            });
        }

        Ok(MigrationPlan {
            source: source.to_path_buf(),
            steps,
            dangling_parents: dangling,
        })
    }

    /// Render a human-readable plan for stdout.
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(out, "Migration plan for {}", self.source.display());
        let _ = writeln!(out, "  {} tickets total", self.steps.len());
        if !self.dangling_parents.is_empty() {
            let _ = writeln!(out, "  {} dangling parent references (demoted to orphan): {:?}",
                self.dangling_parents.len(), self.dangling_parents);
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "  {:>6}  {:>4}  {:<9}  {:<32}  {}",
            "OLD ID", "NEW", "KIND", "ADDRESS", "TARGET");
        for s in &self.steps {
            let _ = writeln!(
                out,
                "  {:>6}  {:>4}  {:<9}  {:<32}  {}",
                s.old_id,
                s.new_leaf.to_string(),
                format!("{:?}", s.old_kind),
                s.address.to_string(),
                s.target_dir.display(),
            );
        }
        out
    }
}

fn map_kind(k: OldKind) -> TypePrefix {
    // Phase 1: legacy Project did not exist as a Kind; products without a
    // parent stay orphan products. Mapping is straightforward.
    match k {
        OldKind::Product => TypePrefix::Product,
        OldKind::Epic => TypePrefix::Epic,
        OldKind::Task => TypePrefix::Task,
        OldKind::Subtask => TypePrefix::Subtask,
        OldKind::Milestone => TypePrefix::Milestone,
    }
}

/// Walk a ticket's parent chain to the root, returning old-ids root-first.
/// Parents that are missing from the source are skipped (the chain stops).
fn walk_chain(
    start: u64,
    tasks: &[LegacyTask],
    old_to_new: &BTreeMap<u64, LeafId>,
    known: &std::collections::BTreeSet<u64>,
) -> Vec<u64> {
    let by_id: BTreeMap<u64, &LegacyTask> = tasks.iter().map(|t| (t.id, t)).collect();
    let mut chain = Vec::new();
    let mut cursor = Some(start);
    let mut guard = 0usize;
    while let Some(id) = cursor {
        if guard > 16 { break; } // defensive cap against legacy cycles
        guard += 1;
        chain.push(id);
        cursor = by_id.get(&id)
            .and_then(|t| t.parent)
            .filter(|p| known.contains(p) && old_to_new.contains_key(p));
    }
    chain.reverse();
    chain
}

/// Title -> kebab-case slug. Lowercase ASCII; non-alphanumeric becomes `-`;
/// repeated hyphens collapsed; leading/trailing hyphens trimmed. Falls back to
/// `untitled-<id>` if the title produces an empty slug.
fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_was_hyphen = true; // suppress leading hyphens
    for c in title.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            Some(c.to_ascii_lowercase())
        } else if c.is_whitespace() || matches!(c, '-' | '_' | '/' | '\\' | '.') {
            Some('-')
        } else {
            None
        };
        if let Some(ch) = mapped {
            if ch == '-' {
                if !last_was_hyphen {
                    out.push('-');
                    last_was_hyphen = true;
                }
            } else {
                out.push(ch);
                last_was_hyphen = false;
            }
        }
    }
    while out.ends_with('-') { out.pop(); }
    if out.is_empty() { return "untitled".to_string(); }
    if out.len() > 63 { out.truncate(63); }
    while out.ends_with('-') { out.pop(); }
    out
}

#[derive(Debug)]
pub enum MigrateError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    Layout(super::layout::LayoutError),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::Io(e) => write!(f, "migration io: {e}"),
            MigrateError::Parse(e) => write!(f, "migration parse: {e}"),
            MigrateError::Layout(e) => write!(f, "migration layout: {e}"),
        }
    }
}

impl std::error::Error for MigrateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-migrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_legacy(dir: &Path, name: &str, json: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, json).unwrap();
        p
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Lock protocol"), "lock-protocol");
        assert_eq!(slugify("E-commerce Platform"), "e-commerce-platform");
        assert_eq!(slugify("   Leading and trailing   "), "leading-and-trailing");
        assert_eq!(slugify("User Story / Requirements"), "user-story-requirements");
        assert_eq!(slugify("Implement Foo (v2)"), "implement-foo-v2");
        assert_eq!(slugify("!!!"), "untitled");
        assert_eq!(slugify(""), "untitled");
    }

    #[test]
    fn maps_old_kinds_to_new_prefixes() {
        assert_eq!(map_kind(OldKind::Product), TypePrefix::Product);
        assert_eq!(map_kind(OldKind::Epic), TypePrefix::Epic);
        assert_eq!(map_kind(OldKind::Task), TypePrefix::Task);
        assert_eq!(map_kind(OldKind::Subtask), TypePrefix::Subtask);
        assert_eq!(map_kind(OldKind::Milestone), TypePrefix::Milestone);
    }

    #[test]
    fn plans_hierarchical_chain() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let legacy = r#"[
            {"id":1,"title":"E-commerce Platform","parent":null,"kind":"product","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0},
            {"id":2,"title":"User Management System","parent":1,"kind":"epic","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0},
            {"id":3,"title":"User Registration","parent":2,"kind":"task","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0},
            {"id":4,"title":"Email Validation","parent":3,"kind":"subtask","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0}
        ]"#;
        let src = write_legacy(&dir, "tasks.json", legacy);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 4);
        assert!(plan.dangling_parents.is_empty());

        let product = &plan.steps[0];
        assert_eq!(product.new_leaf.to_string(), "PRD1");
        assert_eq!(product.address.to_string(), "PRD1");
        assert_eq!(product.target_dir, PathBuf::from("products/e-commerce-platform"));

        let epic = &plan.steps[1];
        assert_eq!(epic.new_leaf.to_string(), "EPC1");
        assert_eq!(epic.address.to_string(), "PRD1-EPC1");
        assert_eq!(epic.target_dir, PathBuf::from("products/e-commerce-platform/epics/user-management-system"));

        let task = &plan.steps[2];
        assert_eq!(task.address.to_string(), "PRD1-EPC1-TSK1");

        let subtask = &plan.steps[3];
        assert_eq!(subtask.new_leaf.to_string(), "SBT1");
        assert_eq!(subtask.address.to_string(), "PRD1-EPC1-TSK1-SBT1");
        assert_eq!(
            subtask.target_dir,
            PathBuf::from("products/e-commerce-platform/epics/user-management-system/tasks/user-registration/subtasks/email-validation"),
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dangling_parent_demoted_to_orphan() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let legacy = r#"[
            {"id":7,"title":"Orphan Task","parent":999,"kind":"task","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0}
        ]"#;
        let src = write_legacy(&dir, "tasks.json", legacy);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.dangling_parents, vec![999]);
        let step = &plan.steps[0];
        assert!(step.new_parent.is_none(), "ticket pointing at missing parent demoted");
        assert_eq!(step.address.depth(), 1);
        assert_eq!(step.target_dir, PathBuf::from("tasks/orphan-task"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrapped_object_legacy_format_accepted() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let legacy = r#"{ "tasks": [
            {"id":1,"title":"Solo Milestone","parent":null,"kind":"milestone","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0}
        ] }"#;
        let src = write_legacy(&dir, "wrapped.json", legacy);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].new_leaf.to_string(), "MLS1");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_produces_table() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let legacy = r#"[
            {"id":1,"title":"P","parent":null,"kind":"product","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0},
            {"id":2,"title":"E","parent":1,"kind":"epic","tags":[],"artifacts":[],"status":"open","created_at_utc":0,"updated_at_utc":0}
        ]"#;
        let src = write_legacy(&dir, "tasks.json", legacy);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        let rendered = plan.render();
        assert!(rendered.contains("Migration plan for"));
        assert!(rendered.contains("2 tickets total"));
        assert!(rendered.contains("PRD1"));
        assert!(rendered.contains("EPC1"));

        fs::remove_dir_all(&dir).ok();
    }
}
