//! `aliases.json` - the redirect table for moved or renamed tickets.
//!
//! When a ticket moves (its parent chain changes), the address form of its id
//! changes. Aliases preserve resolvability of the old form by recording a
//! redirect from the old string to the new one.
//!
//! Leaf-form lookups never need aliases (leaf ids are stable for life).
//! Address-form lookups consult aliases when the direct lookup fails.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::state::{atomic_write, StateError};

/// On-disk and in-memory representation of `aliases.json`.
///
/// Keys and values are the canonical string forms of address ids (e.g.
/// `"PRJ1-PRD1-EPC3-TSK7"`). A `null` (None) target indicates the alias was
/// retired and the input should be treated as a clean 404.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Aliases {
    /// Map from old-address to new-address (or `None` if retired).
    pub map: BTreeMap<String, Option<String>>,
}

impl Aliases {
    /// Empty alias table.
    pub fn empty() -> Self {
        Aliases { map: BTreeMap::new() }
    }

    /// Load from a `.pm/aliases.json` path. Missing or empty file returns
    /// [`Aliases::empty`].
    pub fn load(path: &Path) -> Result<Self, StateError> {
        if !path.exists() {
            return Ok(Aliases::empty());
        }
        let raw = fs::read_to_string(path).map_err(StateError::Io)?;
        if raw.trim().is_empty() {
            return Ok(Aliases::empty());
        }
        serde_json::from_str(&raw).map_err(StateError::Parse)
    }

    /// Atomically write aliases.json.
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        let json = serde_json::to_string_pretty(self).map_err(StateError::Parse)?;
        atomic_write(path, json.as_bytes()).map_err(StateError::Io)
    }

    /// Resolve one hop of redirection. Returns:
    /// - `Some(Some(target))` if there is a live redirect,
    /// - `Some(None)` if the alias was retired (treat as 404),
    /// - `None` if no alias exists.
    pub fn resolve_one(&self, key: &str) -> Option<Option<&str>> {
        self.map.get(key).map(|v| v.as_deref())
    }

    /// Follow a chain of redirects up to `max_hops` deep, stopping when no
    /// further alias exists or when a retired entry is hit. Returns the final
    /// live target, or `None` if the chain terminates in a retired alias.
    pub fn resolve(&self, key: &str, max_hops: usize) -> Option<String> {
        let mut current = key.to_string();
        for _ in 0..max_hops {
            match self.resolve_one(&current) {
                Some(Some(next)) => current = next.to_string(),
                Some(None) => return None, // retired
                None => return Some(current), // no further redirect
            }
        }
        // Hit hop limit; return whatever we have rather than loop forever.
        Some(current)
    }

    /// Record a new redirect.
    pub fn add(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.map.insert(from.into(), Some(to.into()));
    }

    /// Retire an alias (returns `None` from now on).
    pub fn retire(&mut self, from: impl Into<String>) {
        self.map.insert(from.into(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-aliases-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_aliases_resolves_to_self() {
        let a = Aliases::empty();
        assert_eq!(a.resolve("PRJ1-PRD1-EPC3-TSK7", 5).as_deref(), Some("PRJ1-PRD1-EPC3-TSK7"));
    }

    #[test]
    fn single_hop_redirect() {
        let mut a = Aliases::empty();
        a.add("PRJ1-PRD1-EPC3-TSK7", "PRJ1-PRD1-EPC5-TSK7");
        assert_eq!(
            a.resolve("PRJ1-PRD1-EPC3-TSK7", 5).as_deref(),
            Some("PRJ1-PRD1-EPC5-TSK7"),
        );
    }

    #[test]
    fn chained_redirects_followed() {
        let mut a = Aliases::empty();
        a.add("OLD-A", "OLD-B");
        a.add("OLD-B", "OLD-C");
        a.add("OLD-C", "FINAL");
        assert_eq!(a.resolve("OLD-A", 5).as_deref(), Some("FINAL"));
    }

    #[test]
    fn retired_alias_returns_none() {
        let mut a = Aliases::empty();
        a.add("STEP-1", "STEP-2");
        a.retire("STEP-2");
        // Walking the chain should terminate at the retired entry with None.
        assert_eq!(a.resolve("STEP-1", 5), None);
        assert_eq!(a.resolve("STEP-2", 5), None);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tmp_dir();
        let path = dir.join("aliases.json");
        let mut a = Aliases::empty();
        a.add("PRJ1-PRD1-EPC3-TSK7", "PRJ1-PRD1-EPC5-TSK7");
        a.retire("OLD-SLUG-X");
        a.save(&path).unwrap();
        let loaded = Aliases::load(&path).unwrap();
        assert_eq!(loaded.map.len(), 2);
        assert_eq!(
            loaded.resolve("PRJ1-PRD1-EPC3-TSK7", 5).as_deref(),
            Some("PRJ1-PRD1-EPC5-TSK7"),
        );
        assert_eq!(loaded.resolve("OLD-SLUG-X", 5), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let dir = tmp_dir();
        let path = dir.join("does-not-exist.json");
        let a = Aliases::load(&path).unwrap();
        assert!(a.map.is_empty());
        fs::remove_dir_all(&dir).ok();
    }
}
