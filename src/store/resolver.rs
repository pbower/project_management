//! Id resolution: any input form -> canonical leaf id + on-disk path.
//!
//! The resolver is the single integration point that ties together id parsing,
//! state.json lookup, alias chasing, and on-disk path verification. CLI verbs
//! and (later) MCP tools call `Resolver::resolve(input)` and never have to
//! think about which form was supplied.

use std::path::PathBuf;

use super::aliases::Aliases;
use super::id::{IdInput, IdParseError, LeafId};
use super::layout::Layout;
use super::state::{ItemEntry, State};

/// Result of a successful resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Canonical leaf id for the ticket.
    pub leaf: LeafId,
    /// Absolute path to the ticket directory.
    pub absolute_path: PathBuf,
    /// Relative path under `.pm/` (the value stored in `state.items[leaf].path`).
    pub relative_path: PathBuf,
    /// Set when the input form did not match directly and an alias was used.
    pub via_alias_from: Option<String>,
}

/// Resolves caller-supplied id strings to canonical leaf + path. Holds borrows
/// of the layout, state, and aliases for the duration of resolution; mutating
/// callers should drop the resolver before writing.
pub struct Resolver<'a> {
    pub layout: &'a Layout,
    pub state: &'a State,
    pub aliases: &'a Aliases,
}

impl<'a> Resolver<'a> {
    pub fn new(layout: &'a Layout, state: &'a State, aliases: &'a Aliases) -> Self {
        Resolver { layout, state, aliases }
    }

    /// Resolve a free-form input string (`"TSK7"`, `"PRJ1-PRD1-EPC3-TSK7"`,
    /// `"projects/pm/.../lock-protocol"` etc.) to a [`Resolved`] handle.
    pub fn resolve(&self, raw: &str) -> Result<Resolved, ResolveError> {
        let input: IdInput = raw.parse().map_err(ResolveError::Parse)?;
        self.resolve_input(&input, raw)
    }

    /// Resolve a pre-parsed [`IdInput`]. `raw` is supplied for alias lookups
    /// (alias keys are the original string forms).
    ///
    /// Resolution order:
    /// 1. Direct lookup of the input's leaf in `state.items` (fast path).
    /// 2. For address-form inputs, consult aliases. A live alias rewrites the
    ///    address; the target's leaf is then looked up in state.
    /// 3. If the leaf is tombstoned, report it.
    /// 4. Otherwise the id is unknown.
    pub fn resolve_input(&self, input: &IdInput, raw: &str) -> Result<Resolved, ResolveError> {
        let leaf = input.leaf();

        // Direct lookup against state.items. Leaves are stable for life, so this
        // covers any address-form input whose leaf is still live (even if the
        // address chain or slugs have changed since the input was written).
        if let Some(entry) = self.state.lookup(leaf) {
            return Ok(self.build_resolved(leaf, entry, None));
        }

        // For address-form inputs, the user-supplied string may have been
        // retired or rewritten. Aliases handle the case where the leaf itself
        // is gone (deleted and replaced) and the user has the old address.
        if let IdInput::Address(_) = input {
            let key = raw.trim();
            if let Some(target) = self.aliases.resolve(key, 16) {
                if target != key {
                    let target_input: IdInput = target.parse().map_err(ResolveError::Parse)?;
                    let target_leaf = target_input.leaf();
                    if let Some(entry) = self.state.lookup(target_leaf) {
                        return Ok(self.build_resolved(target_leaf, entry, Some(key.to_string())));
                    }
                    return Err(ResolveError::AliasTargetMissing {
                        from: key.to_string(),
                        target,
                    });
                }
            }
            // resolve_one returning Some(None) means the key was explicitly
            // retired; treat as a 404 with a clear cause.
            if self.aliases.resolve_one(key) == Some(None) {
                return Err(ResolveError::Retired(key.to_string()));
            }
        }

        if self.state.is_tombstoned(leaf) {
            return Err(ResolveError::Tombstoned(leaf.to_string()));
        }
        Err(ResolveError::Unknown(raw.to_string()))
    }

    fn build_resolved(&self, leaf: LeafId, entry: &ItemEntry, via_alias_from: Option<String>) -> Resolved {
        let abs = self.layout.root.join(&entry.path);
        Resolved {
            leaf,
            absolute_path: abs,
            relative_path: entry.path.clone(),
            via_alias_from,
        }
    }
}

/// All ways resolution can fail.
#[derive(Debug)]
pub enum ResolveError {
    /// Input did not parse as any id form.
    Parse(IdParseError),
    /// The leaf is known to be tombstoned.
    Tombstoned(String),
    /// An alias entry was found but its target is missing from `state.items`.
    AliasTargetMissing { from: String, target: String },
    /// An alias entry was found and marked retired (treated as a clean 404).
    Retired(String),
    /// The leaf id is not in `state.items` and no usable alias exists.
    Unknown(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::Parse(e) => write!(f, "id parse error: {e}"),
            ResolveError::Tombstoned(s) => write!(f, "id {s} has been deleted"),
            ResolveError::AliasTargetMissing { from, target } => {
                write!(f, "alias {from:?} -> {target:?} but target is not in state.json")
            }
            ResolveError::Retired(s) => write!(f, "id {s} was retired"),
            ResolveError::Unknown(s) => write!(f, "id {s:?} not found"),
        }
    }
}

impl std::error::Error for ResolveError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;
    use std::path::PathBuf;

    fn fresh_setup() -> (Layout, State, Aliases) {
        let root = std::env::temp_dir().join(format!(
            "pm-store-resolver-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let layout = Layout::under(&root);
        layout.init().unwrap();
        (layout, State::fresh(), Aliases::empty())
    }

    #[test]
    fn leaf_form_resolves_directly() {
        let (layout, mut state, aliases) = fresh_setup();
        let leaf = state.allocate(TypePrefix::Task);
        let rel = PathBuf::from("tasks/lock-protocol");
        std::fs::create_dir_all(layout.root.join(&rel)).unwrap();
        state.insert(leaf, ItemEntry { path: rel.clone() });

        let resolver = Resolver::new(&layout, &state, &aliases);
        let got = resolver.resolve("TSK1").unwrap();
        assert_eq!(got.leaf, leaf);
        assert_eq!(got.relative_path, rel);
        assert_eq!(got.absolute_path, layout.root.join(&rel));
        assert!(got.via_alias_from.is_none());

        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn address_form_resolves_to_leaf() {
        let (layout, mut state, aliases) = fresh_setup();
        let prj = state.allocate(TypePrefix::Project);
        let prd = state.allocate(TypePrefix::Product);
        let epc = state.allocate(TypePrefix::Epic);
        let tsk = state.allocate(TypePrefix::Task);
        let rel = PathBuf::from("projects/pm/products/core/epics/checkouts/tasks/lock-protocol");
        std::fs::create_dir_all(layout.root.join(&rel)).unwrap();
        state.insert(tsk, ItemEntry { path: rel.clone() });

        let resolver = Resolver::new(&layout, &state, &aliases);
        let key = format!("{prj}-{prd}-{epc}-{tsk}");
        let got = resolver.resolve(&key).unwrap();
        assert_eq!(got.leaf, tsk);

        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn slugged_address_resolves_to_leaf() {
        let (layout, mut state, aliases) = fresh_setup();
        let tsk = state.allocate(TypePrefix::Task);
        let rel = PathBuf::from("tasks/lock-protocol");
        std::fs::create_dir_all(layout.root.join(&rel)).unwrap();
        state.insert(tsk, ItemEntry { path: rel.clone() });

        let resolver = Resolver::new(&layout, &state, &aliases);
        let got = resolver.resolve("TSK1-lock-protocol").unwrap();
        assert_eq!(got.leaf, tsk);

        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn tombstoned_rejected() {
        let (layout, mut state, aliases) = fresh_setup();
        let leaf = state.allocate(TypePrefix::Task);
        state.tombstone(leaf);
        let resolver = Resolver::new(&layout, &state, &aliases);
        let err = resolver.resolve("TSK1").unwrap_err();
        assert!(matches!(err, ResolveError::Tombstoned(_)));
        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn unknown_id_rejected() {
        let (layout, state, aliases) = fresh_setup();
        let resolver = Resolver::new(&layout, &state, &aliases);
        let err = resolver.resolve("TSK99").unwrap_err();
        assert!(matches!(err, ResolveError::Unknown(_)));
        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn moved_ticket_still_resolves_via_leaf_lookup() {
        // Leaves are stable for life: when a ticket moves, its leaf id stays
        // the same and state.items reflects the new path. A user supplying the
        // OLD address chain (with stale parent leaves) still resolves to the
        // correct ticket via direct leaf lookup; no alias needed.
        let (layout, mut state, aliases) = fresh_setup();
        let _prj = state.allocate(TypePrefix::Project);
        let _prd = state.allocate(TypePrefix::Product);
        let _old_epc = state.allocate(TypePrefix::Epic);
        let _new_epc = state.allocate(TypePrefix::Epic);
        let tsk = state.allocate(TypePrefix::Task);
        let new_rel = PathBuf::from("projects/pm/products/core/epics/locking/tasks/lock-protocol");
        std::fs::create_dir_all(layout.root.join(&new_rel)).unwrap();
        state.insert(tsk, ItemEntry { path: new_rel.clone() });

        let resolver = Resolver::new(&layout, &state, &aliases);
        // Old address PRJ1-PRD1-EPC3-TSK1 still resolves because TSK1 is the
        // leaf and state knows where it lives now.
        let got = resolver.resolve("PRJ1-PRD1-EPC3-TSK1").unwrap();
        assert_eq!(got.leaf, tsk);
        assert_eq!(got.relative_path, new_rel);
        assert!(got.via_alias_from.is_none(), "no alias needed when leaf is live");
        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn alias_rewrites_address_to_different_leaf() {
        // Aliases earn their keep when the LEAF in the supplied address is no
        // longer live (deleted + replaced) and the alias forwards to a new
        // leaf that is.
        let (layout, mut state, mut aliases) = fresh_setup();
        let _prj = state.allocate(TypePrefix::Project);
        let _prd = state.allocate(TypePrefix::Product);
        let _epc = state.allocate(TypePrefix::Epic);
        // First task TSK1 never reached state.items (e.g. deleted before save).
        let _ghost = state.allocate(TypePrefix::Task);
        // A replacement task TSK2 lives at the same address slot.
        let replacement = state.allocate(TypePrefix::Task);
        let rel = PathBuf::from("projects/pm/products/core/epics/checkouts/tasks/lock-protocol");
        std::fs::create_dir_all(layout.root.join(&rel)).unwrap();
        state.insert(replacement, ItemEntry { path: rel.clone() });

        aliases.add("PRJ1-PRD1-EPC1-TSK1", "PRJ1-PRD1-EPC1-TSK2");

        let resolver = Resolver::new(&layout, &state, &aliases);
        let got = resolver.resolve("PRJ1-PRD1-EPC1-TSK1").unwrap();
        assert_eq!(got.leaf, replacement);
        assert_eq!(got.via_alias_from.as_deref(), Some("PRJ1-PRD1-EPC1-TSK1"));
        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn retired_alias_returns_retired_error() {
        let (layout, state, mut aliases) = fresh_setup();
        aliases.retire("PRJ1-PRD1-EPC9-TSK77");
        let resolver = Resolver::new(&layout, &state, &aliases);
        let err = resolver.resolve("PRJ1-PRD1-EPC9-TSK77").unwrap_err();
        assert!(matches!(err, ResolveError::Retired(_)));
        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }

    #[test]
    fn alias_target_missing_reports_clearly() {
        let (layout, state, mut aliases) = fresh_setup();
        // Alias points at an id that state doesn't know about.
        aliases.add("PRJ1-PRD1-EPC3-TSK1", "PRJ1-PRD1-EPC9-TSK999");
        let resolver = Resolver::new(&layout, &state, &aliases);
        let err = resolver.resolve("PRJ1-PRD1-EPC3-TSK1").unwrap_err();
        match err {
            ResolveError::AliasTargetMissing { from, target } => {
                assert_eq!(from, "PRJ1-PRD1-EPC3-TSK1");
                assert_eq!(target, "PRJ1-PRD1-EPC9-TSK999");
            }
            other => panic!("expected AliasTargetMissing, got {other:?}"),
        }
        std::fs::remove_dir_all(layout.root.parent().unwrap()).ok();
    }
}
