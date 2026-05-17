//! Typed-prefix monotonic IDs for v2 tickets.
//!
//! IDs use a fixed 3-character uppercase prefix encoding the ticket type, followed
//! by a monotonic decimal counter. All forms accepted as input:
//!
//! - Leaf: `TSK7` (durable, stored in front-matter, never changes)
//! - Address: `PRJ1-PRD3-EPC7-TSK22` (derived hierarchy, regenerated on move)
//! - Labelled leaf: `TSK7-lock-protocol` (parser tolerates a trailing label; the label is not part of any on-disk directory name)
//! - Labelled address: `PRJ1/PRD3/EPC7-checkouts/TSK22-lock-protocol`
//!
//! The parser keys on `(PRJ|PRD|EPC|TSK|SBT|MLS)\d+` and ignores any trailing
//! label text after the digits.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The six ticket types in v2.
///
/// Three-character uppercase prefix; depth of nesting is encoded by directory
/// structure on disk, not by the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TypePrefix {
    /// Top-level container. Lives under `.pm/projects/<PRJ>/`.
    #[serde(rename = "PRJ")]
    Project,
    /// Product within a project. Lives under `<project>/products/<PRD>/`.
    #[serde(rename = "PRD")]
    Product,
    /// Epic within a product. Lives under `<product>/epics/<EPC>/`.
    #[serde(rename = "EPC")]
    Epic,
    /// Task within an epic. Lives under `<epic>/tasks/<TSK>/`.
    #[serde(rename = "TSK")]
    Task,
    /// Subtask within a task. Lives under `<task>/subtasks/<SBT>/`.
    #[serde(rename = "SBT")]
    Subtask,
    /// Milestone marker. Cross-cutting; project-scoped by default.
    #[serde(rename = "MLS")]
    Milestone,
}

impl TypePrefix {
    /// Three-letter uppercase prefix as written in IDs.
    pub fn as_str(&self) -> &'static str {
        match self {
            TypePrefix::Project => "PRJ",
            TypePrefix::Product => "PRD",
            TypePrefix::Epic => "EPC",
            TypePrefix::Task => "TSK",
            TypePrefix::Subtask => "SBT",
            TypePrefix::Milestone => "MLS",
        }
    }

    /// Parse a 3-letter uppercase prefix. Case sensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "PRJ" => Some(TypePrefix::Project),
            "PRD" => Some(TypePrefix::Product),
            "EPC" => Some(TypePrefix::Epic),
            "TSK" => Some(TypePrefix::Task),
            "SBT" => Some(TypePrefix::Subtask),
            "MLS" => Some(TypePrefix::Milestone),
            _ => None,
        }
    }

    /// Singular display name for human-readable output.
    pub fn display_singular(&self) -> &'static str {
        match self {
            TypePrefix::Project => "Project",
            TypePrefix::Product => "Product",
            TypePrefix::Epic => "Epic",
            TypePrefix::Task => "Task",
            TypePrefix::Subtask => "Subtask",
            TypePrefix::Milestone => "Milestone",
        }
    }

    /// Plural type-folder name used on disk (e.g. `tasks/`, `subtasks/`).
    pub fn type_folder(&self) -> &'static str {
        match self {
            TypePrefix::Project => "projects",
            TypePrefix::Product => "products",
            TypePrefix::Epic => "epics",
            TypePrefix::Task => "tasks",
            TypePrefix::Subtask => "subtasks",
            TypePrefix::Milestone => "milestones",
        }
    }

    /// All six prefixes in declaration order.
    pub fn all() -> &'static [TypePrefix] {
        &[
            TypePrefix::Project,
            TypePrefix::Product,
            TypePrefix::Epic,
            TypePrefix::Task,
            TypePrefix::Subtask,
            TypePrefix::Milestone,
        ]
    }
}

impl fmt::Display for TypePrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical leaf identifier for a single ticket. Stable for the lifetime of the
/// ticket, never reused (deleted IDs are tombstoned in [`crate::store::State`]).
///
/// Serialised as the 3-letter prefix concatenated with the decimal counter
/// (e.g. `"TSK7"`), in both JSON and front-matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LeafId {
    prefix: TypePrefix,
    number: u64,
}

impl LeafId {
    /// Build a leaf id from its parts. Numbers are unsigned and may be zero,
    /// but zero is reserved by convention (see PM_DESIGN.md Section 5.2).
    pub fn new(prefix: TypePrefix, number: u64) -> Self {
        LeafId { prefix, number }
    }

    pub fn prefix(&self) -> TypePrefix {
        self.prefix
    }
    pub fn number(&self) -> u64 {
        self.number
    }

    /// Render as the canonical 3-letter-plus-decimal form (e.g. `"TSK7"`).
    pub fn as_string(&self) -> String {
        format!("{}{}", self.prefix.as_str(), self.number)
    }
}

impl fmt::Display for LeafId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.prefix.as_str(), self.number)
    }
}

impl FromStr for LeafId {
    type Err = IdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_one_leaf(s)
            .map(|(leaf, rest)| {
                if rest.is_empty() {
                    Ok(leaf)
                } else {
                    Err(IdParseError::TrailingInput(rest.to_string()))
                }
            })
            .and_then(|r| r)
    }
}

impl Serialize for LeafId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.as_string())
    }
}

impl<'de> Deserialize<'de> for LeafId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A hierarchical chain of leaf IDs representing a ticket's location in the
/// tree. Derived from the parent chain on disk; not stored as a primary key.
///
/// Always at least one leaf. Up to five for the full PRJ -> PRD -> EPC -> TSK
/// -> SBT chain. Milestones may appear as `MLS5` (orphan) or `PRJ1-MLS5`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressId {
    segments: Vec<LeafId>,
}

impl AddressId {
    /// Build from a non-empty sequence of leaves.
    pub fn new(segments: Vec<LeafId>) -> Result<Self, IdParseError> {
        if segments.is_empty() {
            return Err(IdParseError::EmptyAddress);
        }
        Ok(AddressId { segments })
    }

    /// The terminal leaf id, which is always the canonical handle for the ticket.
    pub fn leaf(&self) -> LeafId {
        *self
            .segments
            .last()
            .expect("AddressId invariant: at least one segment")
    }

    pub fn segments(&self) -> &[LeafId] {
        &self.segments
    }
    pub fn depth(&self) -> usize {
        self.segments.len()
    }

    /// Render as the canonical dash-joined form (e.g. `"PRJ1-PRD3-EPC7-TSK22"`).
    pub fn as_string(&self) -> String {
        self.segments
            .iter()
            .map(|l| l.as_string())
            .collect::<Vec<_>>()
            .join("-")
    }
}

impl fmt::Display for AddressId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_string())
    }
}

impl FromStr for AddressId {
    type Err = IdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut segments = Vec::new();
        let mut rest = s;
        loop {
            let (leaf, after) = parse_one_leaf(rest)?;
            segments.push(leaf);
            if after.is_empty() {
                break;
            }
            // Expect a dash separator before the next leaf.
            let next = after
                .strip_prefix('-')
                .ok_or_else(|| IdParseError::UnexpectedSeparator(after.to_string()))?;
            rest = next;
        }
        AddressId::new(segments)
    }
}

/// Any caller-supplied form of an id, normalised once at parse time.
///
/// Accepts (and remembers which form it came from):
/// - Leaf: `TSK7`
/// - Leaf with label suffix: `TSK7-lock-protocol`
/// - Address: `PRJ1-PRD3-EPC7-TSK22`
/// - Labelled address: `PRJ1/PRD3/EPC7-checkouts/TSK22-lock-protocol`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdInput {
    /// A bare leaf or leaf-with-label.
    Leaf(LeafId),
    /// A multi-segment address; the leaf is always `.leaf()`.
    Address(AddressId),
}

impl IdInput {
    /// The canonical leaf id regardless of how the input was supplied.
    pub fn leaf(&self) -> LeafId {
        match self {
            IdInput::Leaf(l) => *l,
            IdInput::Address(a) => a.leaf(),
        }
    }
}

impl FromStr for IdInput {
    type Err = IdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = s.trim();
        if raw.is_empty() {
            return Err(IdParseError::Empty);
        }

        // Labelled-address form: contains '/' separators. Pick up the leaf of
        // each path segment in turn (each segment of the form `<LEAF>` or
        // `<LEAF>-<label>`).
        if raw.contains('/') {
            let mut segments = Vec::new();
            for part in raw.split('/').filter(|p| !p.is_empty()) {
                let (leaf, _rest) = parse_one_leaf(part)?;
                segments.push(leaf);
            }
            return Ok(IdInput::Address(AddressId::new(segments)?));
        }

        // Single segment: either a leaf-with-label, a multi-leaf address, or
        // both. Try parsing successive leaves separated by '-' until we run
        // out of leaves (the residual must then be either empty or a label
        // starting with `-`).
        let mut segments = Vec::new();
        let mut rest = raw;
        loop {
            let (leaf, after) = parse_one_leaf(rest)?;
            segments.push(leaf);
            if after.is_empty() {
                break;
            }
            // A '-' may introduce either the next leaf or a label. Peek ahead.
            let candidate = after
                .strip_prefix('-')
                .ok_or_else(|| IdParseError::UnexpectedSeparator(after.to_string()))?;
            if looks_like_leaf_start(candidate) {
                rest = candidate;
            } else {
                // Label suffix; we are done collecting leaves.
                break;
            }
        }

        if segments.len() == 1 {
            Ok(IdInput::Leaf(segments[0]))
        } else {
            Ok(IdInput::Address(AddressId::new(segments)?))
        }
    }
}

fn looks_like_leaf_start(s: &str) -> bool {
    if s.len() < 4 {
        return false;
    }
    let prefix = &s[..3];
    if TypePrefix::parse(prefix).is_none() {
        return false;
    }
    s.as_bytes()
        .get(3)
        .map(|b| b.is_ascii_digit())
        .unwrap_or(false)
}

/// Parse one leading leaf id from `s`. Returns the leaf and the residual.
///
/// Accepts forms like `TSK7`, `TSK7rest`, `TSK7-rest`. The residual is whatever
/// remains after the digit run; it may start with `-` or with non-digit text or
/// be empty.
fn parse_one_leaf(s: &str) -> Result<(LeafId, &str), IdParseError> {
    if s.len() < 4 {
        return Err(IdParseError::TooShort(s.to_string()));
    }
    let prefix_str = &s[..3];
    let prefix = TypePrefix::parse(prefix_str)
        .ok_or_else(|| IdParseError::UnknownPrefix(prefix_str.to_string()))?;

    let mut digit_end = 3;
    let bytes = s.as_bytes();
    while digit_end < bytes.len() && bytes[digit_end].is_ascii_digit() {
        digit_end += 1;
    }
    if digit_end == 3 {
        return Err(IdParseError::MissingDigits(prefix_str.to_string()));
    }
    let number: u64 = s[3..digit_end]
        .parse()
        .map_err(|_| IdParseError::NumberOverflow(s[3..digit_end].to_string()))?;
    Ok((LeafId::new(prefix, number), &s[digit_end..]))
}

/// All ways a caller-supplied id string can fail to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdParseError {
    /// Input was empty or whitespace.
    Empty,
    /// Input was shorter than the minimum 4 characters (`XXXn`).
    TooShort(String),
    /// First three characters did not match any known prefix.
    UnknownPrefix(String),
    /// Prefix matched but no digits followed.
    MissingDigits(String),
    /// Digits parsed but exceeded `u64::MAX`.
    NumberOverflow(String),
    /// Trailing input remained after a leaf was expected to be terminal.
    TrailingInput(String),
    /// A `-` separator was expected but something else was found.
    UnexpectedSeparator(String),
    /// An address-form id was constructed with zero segments.
    EmptyAddress,
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdParseError::Empty => write!(f, "empty id input"),
            IdParseError::TooShort(s) => write!(f, "id too short: {:?}", s),
            IdParseError::UnknownPrefix(p) => write!(f, "unknown id prefix: {:?}", p),
            IdParseError::MissingDigits(p) => write!(f, "missing digits after prefix {:?}", p),
            IdParseError::NumberOverflow(n) => write!(f, "id number overflow: {:?}", n),
            IdParseError::TrailingInput(t) => write!(f, "trailing input after id: {:?}", t),
            IdParseError::UnexpectedSeparator(s) => write!(f, "unexpected separator near {:?}", s),
            IdParseError::EmptyAddress => write!(f, "address id must contain at least one leaf"),
        }
    }
}

impl std::error::Error for IdParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_parses_all_prefixes() {
        for prefix in TypePrefix::all() {
            let raw = format!("{}{}", prefix.as_str(), 7);
            let parsed: LeafId = raw.parse().unwrap();
            assert_eq!(parsed.prefix(), *prefix);
            assert_eq!(parsed.number(), 7);
            assert_eq!(parsed.as_string(), raw);
        }
    }

    #[test]
    fn leaf_rejects_unknown_prefix() {
        let err: Result<LeafId, _> = "XYZ1".parse();
        assert!(matches!(err, Err(IdParseError::UnknownPrefix(_))));
    }

    #[test]
    fn leaf_rejects_no_digits() {
        let err: Result<LeafId, _> = "TSK".parse();
        // "TSK" is len 3 -> TooShort.
        assert!(matches!(err, Err(IdParseError::TooShort(_))));

        let err: Result<LeafId, _> = "TSKx".parse();
        assert!(matches!(err, Err(IdParseError::MissingDigits(_))));
    }

    #[test]
    fn leaf_rejects_trailing_input_in_from_str() {
        // FromStr for LeafId is strict; use IdInput for tolerant parsing.
        let err: Result<LeafId, _> = "TSK7-label".parse();
        assert!(matches!(err, Err(IdParseError::TrailingInput(_))));
    }

    #[test]
    fn address_parses_full_chain() {
        let raw = "PRJ1-PRD3-EPC7-TSK22-SBT1";
        let addr: AddressId = raw.parse().unwrap();
        assert_eq!(addr.depth(), 5);
        assert_eq!(addr.leaf().to_string(), "SBT1");
        assert_eq!(addr.as_string(), raw);
        let kinds: Vec<TypePrefix> = addr.segments().iter().map(|l| l.prefix()).collect();
        assert_eq!(
            kinds,
            vec![
                TypePrefix::Project,
                TypePrefix::Product,
                TypePrefix::Epic,
                TypePrefix::Task,
                TypePrefix::Subtask,
            ]
        );
    }

    #[test]
    fn address_parses_orphan_task() {
        let addr: AddressId = "TSK15".parse().unwrap();
        assert_eq!(addr.depth(), 1);
        assert_eq!(addr.leaf().to_string(), "TSK15");
    }

    #[test]
    fn idinput_accepts_leaf() {
        let input: IdInput = "TSK7".parse().unwrap();
        assert!(matches!(input, IdInput::Leaf(_)));
        assert_eq!(input.leaf().to_string(), "TSK7");
    }

    #[test]
    fn idinput_accepts_leaf_with_label() {
        let input: IdInput = "TSK7-lock-protocol".parse().unwrap();
        // The label `lock-protocol` does not start with a known prefix so the
        // parser stops after `TSK7` and the result is a bare leaf.
        assert!(matches!(input, IdInput::Leaf(_)));
        assert_eq!(input.leaf().to_string(), "TSK7");
    }

    #[test]
    fn idinput_accepts_address() {
        let input: IdInput = "PRJ1-PRD3-EPC7-TSK22".parse().unwrap();
        match input {
            IdInput::Address(a) => assert_eq!(a.depth(), 4),
            _ => panic!("expected address"),
        }
    }

    #[test]
    fn idinput_accepts_labelled_address() {
        let input: IdInput = "PRJ1-pm/PRD3-core/EPC7-checkouts/TSK22-lock-protocol"
            .parse()
            .unwrap();
        match input {
            IdInput::Address(a) => {
                assert_eq!(a.depth(), 4);
                let leaves: Vec<String> = a.segments().iter().map(|l| l.to_string()).collect();
                assert_eq!(leaves, vec!["PRJ1", "PRD3", "EPC7", "TSK22"]);
            }
            _ => panic!("expected address"),
        }
    }

    #[test]
    fn idinput_distinguishes_address_from_leaf_with_label() {
        // `TSK22-lock-protocol` - residual starts with non-leaf text, so leaf form.
        let a: IdInput = "TSK22-lock-protocol".parse().unwrap();
        assert!(matches!(a, IdInput::Leaf(_)));

        // `TSK22-SBT1` - residual after first leaf starts with another leaf, so address.
        let b: IdInput = "TSK22-SBT1".parse().unwrap();
        assert!(matches!(b, IdInput::Address(_)));
    }

    #[test]
    fn idinput_rejects_empty() {
        let err: Result<IdInput, _> = "".parse();
        assert!(matches!(err, Err(IdParseError::Empty)));
        let err: Result<IdInput, _> = "   ".parse();
        assert!(matches!(err, Err(IdParseError::Empty)));
    }

    #[test]
    fn idinput_rejects_garbage() {
        let err: Result<IdInput, _> = "hello".parse();
        assert!(matches!(err, Err(IdParseError::UnknownPrefix(_))));
    }

    #[test]
    fn leaf_serde_roundtrip() {
        let leaf = LeafId::new(TypePrefix::Task, 42);
        let json = serde_json::to_string(&leaf).unwrap();
        assert_eq!(json, r#""TSK42""#);
        let back: LeafId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, leaf);
    }

    #[test]
    fn type_prefix_serde_roundtrip() {
        let prefix = TypePrefix::Project;
        let json = serde_json::to_string(&prefix).unwrap();
        assert_eq!(json, r#""PRJ""#);
        let back: TypePrefix = serde_json::from_str(&json).unwrap();
        assert_eq!(back, prefix);
    }

    #[test]
    fn type_folder_names() {
        assert_eq!(TypePrefix::Project.type_folder(), "projects");
        assert_eq!(TypePrefix::Product.type_folder(), "products");
        assert_eq!(TypePrefix::Epic.type_folder(), "epics");
        assert_eq!(TypePrefix::Task.type_folder(), "tasks");
        assert_eq!(TypePrefix::Subtask.type_folder(), "subtasks");
        assert_eq!(TypePrefix::Milestone.type_folder(), "milestones");
    }

    #[test]
    fn large_numbers_supported() {
        let leaf: LeafId = "TSK999999".parse().unwrap();
        assert_eq!(leaf.number(), 999_999);
        let leaf: LeafId = "TSK18446744073709551615".parse().unwrap();
        assert_eq!(leaf.number(), u64::MAX);
    }

    #[test]
    fn number_overflow_rejected() {
        // u64::MAX + 1
        let err: Result<LeafId, _> = "TSK18446744073709551616".parse();
        assert!(matches!(err, Err(IdParseError::NumberOverflow(_))));
    }
}
