//! Migration planner from a `tasks.json`-backed [`Database`] to the v2 layout.
//!
//! Now that [`Task`] carries a [`LeafId`] directly, the planner's job is to
//! work out each ticket's address chain and target directory on disk. It does
//! not renumber anything; identity flows straight from `Task.id`.
//!
//! Read-only. The write side lands in a later phase.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::db::Database;
use crate::fields::Kind;
use crate::task::Task;

use super::id::{AddressId, LeafId, TypePrefix};
use super::layout::Layout;

/// One ticket's migration outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStep {
    /// The ticket's canonical leaf id. Carries the type prefix.
    pub leaf: LeafId,
    /// The ticket's `Kind`. Redundant with `leaf.prefix()` for tickets that
    /// were authored cleanly; preserved so a future audit pass can flag any
    /// task whose `Task.kind` and `Task.id.prefix()` disagree.
    pub kind: Kind,
    /// Parent leaf when the source ticket had a parent in the file's chain.
    pub parent: Option<LeafId>,
    /// Full address chain (root -> leaf). For orphans this is `[leaf]` only.
    pub address: AddressId,
    /// Target directory relative to `.pm/`. Each path segment is a `LeafId`.
    pub target_dir: PathBuf,
    /// Original title, preserved for the new front-matter `title:` field.
    pub title: String,
}

/// A complete migration plan for one source database file.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    pub source: PathBuf,
    pub steps: Vec<MigrationStep>,
    /// Parent leaves that the source referenced but that aren't present in
    /// the source. Tickets pointing at these are demoted to orphans.
    pub dangling_parents: Vec<LeafId>,
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
        // Stable order: by leaf id so the plan is deterministic.
        tasks.sort_by_key(|t| t.id);

        let known_ids: BTreeSet<LeafId> = tasks.iter().map(|t| t.id).collect();

        // Detect any parent references that point to leaves not present in
        // the source. Demote those tickets to orphans (parent=None) in their
        // address chain.
        let mut dangling: Vec<LeafId> = Vec::new();
        for t in &tasks {
            if let Some(p) = t.parent {
                if !known_ids.contains(&p) && !dangling.contains(&p) {
                    dangling.push(p);
                }
            }
        }

        // Build the address chain for each task by walking parents that are
        // known to the source. Anything dangling stops the chain at that level.
        let mut steps: Vec<MigrationStep> = Vec::with_capacity(tasks.len());
        for t in &tasks {
            let parent = t.parent.filter(|p| known_ids.contains(p));

            let chain = walk_chain(t.id, &tasks, &known_ids);
            let address = AddressId::new(chain).expect("chain has at least the leaf itself");
            let target_dir = layout.directory_for(&address);

            steps.push(MigrationStep {
                leaf: t.id,
                kind: t.kind,
                parent,
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
            let names: Vec<String> = self.dangling_parents.iter().map(|l| l.to_string()).collect();
            let _ = writeln!(
                out,
                "  {} dangling parent references (demoted to orphan): {}",
                self.dangling_parents.len(),
                names.join(", "),
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "  {:<5}  {:<9}  {:<32}  {}",
            "LEAF", "KIND", "ADDRESS", "TARGET");
        for s in &self.steps {
            let _ = writeln!(
                out,
                "  {:<5}  {:<9}  {:<32}  {}",
                s.leaf.to_string(),
                format!("{:?}", s.kind),
                s.address.to_string(),
                s.target_dir.display(),
            );
        }
        out
    }
}

/// Translate a [`Kind`] to its v2 [`TypePrefix`].
pub(crate) fn kind_to_prefix(k: Kind) -> TypePrefix {
    match k {
        Kind::Project => TypePrefix::Project,
        Kind::Product => TypePrefix::Product,
        Kind::Epic => TypePrefix::Epic,
        Kind::Task => TypePrefix::Task,
        Kind::Subtask => TypePrefix::Subtask,
        Kind::Milestone => TypePrefix::Milestone,
    }
}

/// Walk a ticket's parent chain to the root, returning leaf ids root-first.
/// Parents that are missing from the source stop the chain at the first
/// known ancestor or at the starting leaf if no ancestor resolves.
fn walk_chain(
    start: LeafId,
    tasks: &[Task],
    known: &BTreeSet<LeafId>,
) -> Vec<LeafId> {
    let by_id: BTreeMap<LeafId, &Task> = tasks.iter().map(|t| (t.id, t)).collect();
    let mut chain = Vec::new();
    let mut cursor = Some(start);
    let mut guard = 0usize;
    while let Some(id) = cursor {
        if guard > 16 { break; } // defensive cap against malformed source cycles
        guard += 1;
        chain.push(id);
        cursor = by_id.get(&id)
            .and_then(|t| t.parent)
            .filter(|p| known.contains(p));
    }
    chain.reverse();
    chain
}

#[derive(Debug)]
pub enum MigrateError {
    SourceMissing(PathBuf),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::SourceMissing(p) => write!(f, "migration source not found: {}", p.display()),
        }
    }
}

impl std::error::Error for MigrateError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::fields::{Priority, ProcessStage, Status, Urgency};
    use crate::store::State;
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
    fn task(id: LeafId, title: &str, parent: Option<LeafId>, kind: Kind) -> Task {
        Task {
            id,
            title: title.to_string(),
            summary: None,
            description: None,
            user_story: None,
            requirements: None,
            tags: Vec::new(),
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
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

    fn leaf(prefix: TypePrefix, n: u64) -> LeafId {
        LeafId::new(prefix, n)
    }

    fn write_db(dir: &Path, name: &str, tasks: Vec<Task>) -> PathBuf {
        let p = dir.join(name);
        let db = Database { tasks, state: State::fresh() };
        db.save(&p).unwrap();
        p
    }

    #[test]
    fn kind_maps_one_for_one() {
        assert_eq!(kind_to_prefix(Kind::Project), TypePrefix::Project);
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

        let prd1 = leaf(TypePrefix::Product, 1);
        let epc1 = leaf(TypePrefix::Epic, 1);
        let tsk1 = leaf(TypePrefix::Task, 1);
        let sbt1 = leaf(TypePrefix::Subtask, 1);

        let src = write_db(&dir, "tasks.json", vec![
            task(prd1, "E-commerce Platform", None, Kind::Product),
            task(epc1, "User Management System", Some(prd1), Kind::Epic),
            task(tsk1, "User Registration", Some(epc1), Kind::Task),
            task(sbt1, "Email Validation", Some(tsk1), Kind::Subtask),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 4);
        assert!(plan.dangling_parents.is_empty());

        // Steps come back in leaf-id order (PRD < EPC < TSK < SBT by prefix
        // ordering in TypePrefix). PRD1 lands first.
        let by_leaf: BTreeMap<LeafId, &MigrationStep> =
            plan.steps.iter().map(|s| (s.leaf, s)).collect();

        let product = by_leaf[&prd1];
        assert_eq!(product.address.to_string(), "PRD1");
        assert_eq!(product.target_dir, PathBuf::from("products/PRD1"));

        let epic = by_leaf[&epc1];
        assert_eq!(epic.address.to_string(), "PRD1-EPC1");
        assert_eq!(epic.target_dir, PathBuf::from("products/PRD1/epics/EPC1"));

        let tsk = by_leaf[&tsk1];
        assert_eq!(tsk.address.to_string(), "PRD1-EPC1-TSK1");
        assert_eq!(tsk.target_dir, PathBuf::from("products/PRD1/epics/EPC1/tasks/TSK1"));

        let sub = by_leaf[&sbt1];
        assert_eq!(sub.address.to_string(), "PRD1-EPC1-TSK1-SBT1");
        assert_eq!(
            sub.target_dir,
            PathBuf::from("products/PRD1/epics/EPC1/tasks/TSK1/subtasks/SBT1"),
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dangling_parent_demoted_to_orphan() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let tsk7 = leaf(TypePrefix::Task, 7);
        let ghost = leaf(TypePrefix::Task, 999);

        let src = write_db(&dir, "tasks.json", vec![
            task(tsk7, "Orphan Task", Some(ghost), Kind::Task),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.dangling_parents, vec![ghost]);
        let step = &plan.steps[0];
        assert!(step.parent.is_none(), "ticket pointing at missing parent demoted");
        assert_eq!(step.address.depth(), 1);
        assert_eq!(step.target_dir, PathBuf::from("tasks/TSK7"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn milestone_with_no_parent_stays_orphan() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let mls1 = leaf(TypePrefix::Milestone, 1);
        let src = write_db(&dir, "tasks.json", vec![
            task(mls1, "Solo Milestone", None, Kind::Milestone),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].leaf.to_string(), "MLS1");
        assert_eq!(plan.steps[0].target_dir, PathBuf::from("milestones/MLS1"));

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

        let prd1 = leaf(TypePrefix::Product, 1);
        let epc1 = leaf(TypePrefix::Epic, 1);
        let src = write_db(&dir, "tasks.json", vec![
            task(prd1, "P", None, Kind::Product),
            task(epc1, "E", Some(prd1), Kind::Epic),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        let rendered = plan.render();
        assert!(rendered.contains("Migration plan for"));
        assert!(rendered.contains("2 tickets total"));
        assert!(rendered.contains("PRD1"));
        assert!(rendered.contains("EPC1"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn project_root_chain_keeps_prj_at_top() {
        let dir = tmp_dir();
        let layout = Layout::under(&dir);
        layout.init().unwrap();

        let prj1 = leaf(TypePrefix::Project, 1);
        let prd1 = leaf(TypePrefix::Product, 1);
        let src = write_db(&dir, "tasks.json", vec![
            task(prj1, "Top project", None, Kind::Project),
            task(prd1, "First product", Some(prj1), Kind::Product),
        ]);

        let plan = MigrationPlan::plan(&layout, &src).unwrap();
        let by_leaf: BTreeMap<LeafId, &MigrationStep> =
            plan.steps.iter().map(|s| (s.leaf, s)).collect();

        assert_eq!(by_leaf[&prj1].address.to_string(), "PRJ1");
        assert_eq!(by_leaf[&prj1].target_dir, PathBuf::from("projects/PRJ1"));
        assert_eq!(by_leaf[&prd1].address.to_string(), "PRJ1-PRD1");
        assert_eq!(by_leaf[&prd1].target_dir, PathBuf::from("projects/PRJ1/products/PRD1"));

        fs::remove_dir_all(&dir).ok();
    }
}
