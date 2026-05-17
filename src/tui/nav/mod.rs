//! Hierarchy navigation primitives. Salvaged from the v0.9 `App` so the
//! traversal logic and parent-chain walk live as free functions any TUI
//! surface (board, workbench, future LHP) can compose.
//!
//! Five levels: `Project` -> `Product` -> `Epic` -> `Task` -> `Subtask`.
//! `Milestone` is a cross-cutting marker, not part of the strict tree.

use crate::db::Database;
use crate::fields::Kind;
use crate::store::LeafId;

/// The five linear levels in the work hierarchy. `Milestone` is excluded
/// because it is not strictly above or below the other kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Project,
    Product,
    Epic,
    Task,
    Subtask,
}

impl Level {
    /// Construct from a [`Kind`]. Returns `None` for `Milestone` since it
    /// is not on the linear hierarchy.
    pub fn from_kind(kind: Kind) -> Option<Level> {
        match kind {
            Kind::Project => Some(Level::Project),
            Kind::Product => Some(Level::Product),
            Kind::Epic => Some(Level::Epic),
            Kind::Task => Some(Level::Task),
            Kind::Subtask => Some(Level::Subtask),
            Kind::Milestone => None,
        }
    }

    /// The corresponding [`Kind`].
    pub fn kind(self) -> Kind {
        match self {
            Level::Project => Kind::Project,
            Level::Product => Kind::Product,
            Level::Epic => Kind::Epic,
            Level::Task => Kind::Task,
            Level::Subtask => Kind::Subtask,
        }
    }

    /// One step deeper in the tree, or `None` at `Subtask`.
    pub fn child(self) -> Option<Level> {
        match self {
            Level::Project => Some(Level::Product),
            Level::Product => Some(Level::Epic),
            Level::Epic => Some(Level::Task),
            Level::Task => Some(Level::Subtask),
            Level::Subtask => None,
        }
    }

    /// One step shallower in the tree, or `None` at `Project`.
    pub fn parent(self) -> Option<Level> {
        match self {
            Level::Project => None,
            Level::Product => Some(Level::Project),
            Level::Epic => Some(Level::Product),
            Level::Task => Some(Level::Epic),
            Level::Subtask => Some(Level::Task),
        }
    }

    /// Short uppercase label for headers (`PROJECT`, `PRODUCT`, ...).
    pub fn label_upper(self) -> &'static str {
        match self {
            Level::Project => "PROJECT",
            Level::Product => "PRODUCT",
            Level::Epic => "EPIC",
            Level::Task => "TASK",
            Level::Subtask => "SUBTASK",
        }
    }
}

/// Walk parent links from `leaf` up to the root. Returns the chain
/// root-first, including the leaf as the last element. Guards against
/// cycles via a depth cap so a corrupted tree cannot hang the caller.
pub fn ancestor_chain(db: &Database, leaf: LeafId) -> Vec<LeafId> {
    let mut chain: Vec<LeafId> = Vec::new();
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(cid) = cursor {
        if guard > 16 {
            break;
        }
        guard += 1;
        let Some(task) = db.get(cid) else {
            break;
        };
        chain.push(task.id);
        cursor = task.parent;
    }
    chain.reverse();
    chain
}

/// All tickets at the given level under the optional `parent_filter`.
/// When `parent_filter` is `None`, returns every ticket of that level
/// across the workspace.
pub fn tickets_at(db: &Database, level: Level, parent_filter: Option<LeafId>) -> Vec<LeafId> {
    let target_kind = level.kind();
    db.tasks
        .iter()
        .filter(|t| t.kind == target_kind)
        .filter(|t| match parent_filter {
            Some(p) => t.parent == Some(p),
            None => true,
        })
        .map(|t| t.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::format_kind;

    #[test]
    fn level_traversal_round_trips() {
        let path = [
            Level::Project,
            Level::Product,
            Level::Epic,
            Level::Task,
            Level::Subtask,
        ];
        for window in path.windows(2) {
            let (a, b) = (window[0], window[1]);
            assert_eq!(a.child(), Some(b));
            assert_eq!(b.parent(), Some(a));
        }
        assert_eq!(Level::Subtask.child(), None);
        assert_eq!(Level::Project.parent(), None);
    }

    #[test]
    fn label_upper_matches_kind() {
        for level in [
            Level::Project,
            Level::Product,
            Level::Epic,
            Level::Task,
            Level::Subtask,
        ] {
            assert!(!level.label_upper().is_empty());
            assert_eq!(
                level.label_upper(),
                format_kind(level.kind()).to_uppercase()
            );
        }
    }

    #[test]
    fn from_kind_excludes_milestone() {
        assert_eq!(Level::from_kind(Kind::Project), Some(Level::Project));
        assert_eq!(Level::from_kind(Kind::Subtask), Some(Level::Subtask));
        assert_eq!(Level::from_kind(Kind::Milestone), None);
    }
}
