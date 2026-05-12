//! Migration planner from the existing `tasks.json` format to the v2 layout.
//!
//! Reads the current [`Database`] (`Vec<Task>` keyed by numeric id) and
//! computes the v2 disk layout it would produce: new leaf ids, parent linkage,
//! slugs, address forms, target paths. Does not write anything; designed to be
//! run as `pm doctor --plan-migration` and reviewed before the real migration
//! in a later phase wires the write path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::db::Database;
use crate::fields::Kind;
use crate::task::Task;

use super::id::{AddressId, LeafId, TypePrefix};
use super::layout::Layout;
use super::state::State;

/// One ticket's migration outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStep {
    /// Numeric id from the source `Database`.
    pub source_id: u64,
    /// Source `Kind` (Product/Epic/Task/Subtask/Milestone).
    pub source_kind: Kind,
    /// New v2 leaf id (typed prefix + monotonic number).
    pub new_leaf: LeafId,
    /// New parent leaf, if the source ticket had a parent.
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

/// A complete migration plan for one source database file.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub source: PathBuf,
    pub steps: Vec<MigrationStep>,
    /// Source ids that referenced a parent that does not exist in the source.
    /// Tickets pointing at these are demoted to orphans in the plan.
    pub dangling_parents: Vec<u64>,
}

impl MigrationPlan {
    /// Read `source` (a `tasks.json` produced by [`Database::save`]) and
    /// compute the plan. Does not touch the filesystem beyond the read.
    pub fn plan(layout: &Layout, source: &Path) -> Result<Self, MigrateError> {
        if !source.exists() {
            return Err(MigrateError::SourceMissing(source.to_path_buf()));
        }
        let db = Database::load(source);
        let mut tasks: Vec<Task> = db.tasks;
        // Stable order: by source id so the plan is deterministic.
        tasks.sort_by_key(|t| t.id);

        let mut state = State::fresh();
        // First pass: allocate every ticket its new leaf id and record the
        // source-id -> new-leaf mapping. Parent resolution happens in a second
        // pass after every leaf exists.
        let mut source_to_new: BTreeMap<u64, LeafId> = BTreeMap::new();
        let mut slugs: BTreeMap<u64, String> = BTreeMap::new();
        for t in &tasks {
            let prefix = kind_to_prefix(t.kind);
            let leaf = state.allocate(prefix);
            source_to_new.insert(t.id, leaf);
            slugs.insert(t.id, slugify(&t.title));
        }

        // Detect any parent references that point to source ids not present
        // in the source. Demote those tickets to orphans (parent=None).
        let mut dangling: Vec<u64> = Vec::new();
        let known_ids: std::collections::BTreeSet<u64> = tasks.iter().map(|t| t.id).collect();
        for t in &tasks {
            if let Some(p) = t.parent {
                if !known_ids.contains(&p) && !dangling.contains(&p) {
                    dangling.push(p);
                }
            }
        }

        // Second pass: walk parent chain (resolving via source_to_new) to
        // build the address. Compute target directory from chain + slug map.
        let mut steps: Vec<MigrationStep> = Vec::with_capacity(tasks.len());
        for t in &tasks {
            let new_leaf = source_to_new[&t.id];
            let slug = slugs[&t.id].clone();

            let parent_source = t.parent.filter(|p| known_ids.contains(p));
            let new_parent = parent_source.map(|p| source_to_new[&p]);

            let chain = walk_chain(t.id, &tasks, &source_to_new, &known_ids);
            let chain_slugs: Vec<&str> = chain.iter().map(|src_id| slugs[src_id].as_str()).collect();
            let chain_leaves: Vec<LeafId> = chain.iter().map(|src| source_to_new[src]).collect();

            let address = AddressId::new(chain_leaves).expect("chain has at least the leaf itself");
            let target_dir = layout.directory_for(&address, &chain_slugs).map_err(MigrateError::Layout)?;

            steps.push(MigrationStep {
                source_id: t.id,
                source_kind: t.kind,
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
            "SOURCE", "NEW", "KIND", "ADDRESS", "TARGET");
        for s in &self.steps {
            let _ = writeln!(
                out,
                "  {:>6}  {:>4}  {:<9}  {:<32}  {}",
                s.source_id,
                s.new_leaf.to_string(),
                format!("{:?}", s.source_kind),
                s.address.to_string(),
                s.target_dir.display(),
            );
        }
        out
    }
}

/// Translate a source [`Kind`] to its v2 [`TypePrefix`]. The source enum has
/// five variants (Product/Epic/Task/Subtask/Milestone); v2 adds Project on top
/// but no source ticket can claim that prefix automatically.
fn kind_to_prefix(k: Kind) -> TypePrefix {
    match k {
        Kind::Product => TypePrefix::Product,
        Kind::Epic => TypePrefix::Epic,
        Kind::Task => TypePrefix::Task,
        Kind::Subtask => TypePrefix::Subtask,
        Kind::Milestone => TypePrefix::Milestone,
    }
}

/// Walk a ticket's parent chain to the root, returning source ids root-first.
/// Parents that are missing from the source are skipped (the chain stops).
fn walk_chain(
    start: u64,
    tasks: &[Task],
    source_to_new: &BTreeMap<u64, LeafId>,
    known: &std::collections::BTreeSet<u64>,
) -> Vec<u64> {
    let by_id: BTreeMap<u64, &Task> = tasks.iter().map(|t| (t.id, t)).collect();
    let mut chain = Vec::new();
    let mut cursor = Some(start);
    let mut guard = 0usize;
    while let Some(id) = cursor {
        if guard > 16 { break; } // defensive cap against malformed source cycles
        guard += 1;
        chain.push(id);
        cursor = by_id.get(&id)
            .and_then(|t| t.parent)
            .filter(|p| known.contains(p) && source_to_new.contains_key(p));
    }
    chain.reverse();
    chain
}

/// Title -> kebab-case slug. Lowercase ASCII; non-alphanumeric becomes `-`;
/// repeated hyphens collapsed; leading/trailing hyphens trimmed. Falls back to
/// `untitled` if the title produces an empty slug.
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
    SourceMissing(PathBuf),
    Layout(super::layout::LayoutError),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::SourceMissing(p) => write!(f, "migration source not found: {}", p.display()),
            MigrateError::Layout(e) => write!(f, "migration layout: {e}"),
        }
    }
}

impl std::error::Error for MigrateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::fields::{Priority, ProcessStage, Status, Urgency};
    use crate::task::Task;
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

    /// Build a [`Task`] with the bare-minimum fields the migration planner
    /// touches. Avoids hand-rolled JSON in tests.
    fn task(id: u64, title: &str, parent: Option<u64>, kind: Kind) -> Task {
        Task {
            id,
            title: title.to_string(),
            summary: None,
            description: None,
            user_story: None,
            requirements: None,
            tags: Vec::new(),
            project: None,
            due: None,
            parent,
            kind,
            status: Status::Open,
            priority_level: None::<Priority>,
            urgency: None::<Urgency>,
            process_stage: None::<ProcessStage>,
            issue_link: None,
            pr_link: None,
            artifacts: Vec::new(),
            created_at_utc: 0,
            updated_at_utc: 0,
        }
    }

    fn write_db(dir: &Path, name: &str, tasks: Vec<Task>) -> PathBuf {
        let p = dir.join(name);
        let db = Database { tasks, templates: Vec::new() };
        db.save(&p).unwrap();
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
    fn kind_maps_one_for_one() {
        assert_eq!(kind_to_prefix(Kind::Product), TypePrefix::Product);
        assert_eq!(kind_to_prefix(Kind::Epic), TypePrefix::Epic);
        assert_eq!(kind_to_prefix(Kind::Task), TypePrefix::Task);
        assert_eq!(kind_to_prefix(Kind::Subtask), TypePrefix::Subtask);
        assert_eq!(kind_to_prefix(Kind::Milestone), TypePrefix::Milestone);
    }

    #[test]
    fn plans_hierarchical_chain() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let src = write_db(&dir, "tasks.json", vec![
            task(1, "E-commerce Platform", None, Kind::Product),
            task(2, "User Management System", Some(1), Kind::Epic),
            task(3, "User Registration", Some(2), Kind::Task),
            task(4, "Email Validation", Some(3), Kind::Subtask),
        ]);

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

        let tsk = &plan.steps[2];
        assert_eq!(tsk.address.to_string(), "PRD1-EPC1-TSK1");

        let sub = &plan.steps[3];
        assert_eq!(sub.new_leaf.to_string(), "SBT1");
        assert_eq!(sub.address.to_string(), "PRD1-EPC1-TSK1-SBT1");
        assert_eq!(
            sub.target_dir,
            PathBuf::from("products/e-commerce-platform/epics/user-management-system/tasks/user-registration/subtasks/email-validation"),
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dangling_parent_demoted_to_orphan() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let src = write_db(&dir, "tasks.json", vec![
            task(7, "Orphan Task", Some(999), Kind::Task),
        ]);

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
    fn milestone_with_no_parent_stays_orphan() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let src = write_db(&dir, "tasks.json", vec![
            task(1, "Solo Milestone", None, Kind::Milestone),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].new_leaf.to_string(), "MLS1");
        assert_eq!(plan.steps[0].target_dir, PathBuf::from("milestones/solo-milestone"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_source_is_an_error() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();
        let err = MigrationPlan::plan(&layout, &dir.join("nothing.json")).unwrap_err();
        assert!(matches!(err, MigrateError::SourceMissing(_)));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_produces_table() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let src = write_db(&dir, "tasks.json", vec![
            task(1, "P", None, Kind::Product),
            task(2, "E", Some(1), Kind::Epic),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        let rendered = plan.render();
        assert!(rendered.contains("Migration plan for"));
        assert!(rendered.contains("2 tickets total"));
        assert!(rendered.contains("PRD1"));
        assert!(rendered.contains("EPC1"));

        fs::remove_dir_all(&dir).ok();
    }
}
