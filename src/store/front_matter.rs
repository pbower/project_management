//! YAML front-matter for `CLAUDE.md` files.
//!
//! Each ticket's `CLAUDE.md` opens with a YAML block delimited by `---` lines
//! that holds the structured metadata (id, parent, status, tags, etc.). This
//! module owns the typed [`FrontMatter`] struct, its serde representation, and
//! the markdown-aware split helper that extracts the block from a full file.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::fields::{Priority, ProcessStage, Status, Urgency};

use super::id::LeafId;

/// Structured front-matter for a v2 ticket. Fields mirror PM_DESIGN.md
/// Section 5.4. Optional fields are emitted only when set so a brand-new
/// ticket starts with a minimal block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrontMatter {
    /// Durable canonical leaf id. Required.
    pub id: LeafId,

    /// Parent leaf id, if the ticket has a parent. None for orphans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<LeafId>,

    /// Project scope id, when the ticket lives under a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<LeafId>,

    /// Human-readable title.
    pub title: String,

    /// Ticket lifecycle status. Defaults to `open` when omitted.
    #[serde(default = "default_status")]
    pub status: Status,

    /// Priority classification (must-have / nice-to-have / cut-first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,

    /// Urgency-matrix classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgency: Option<Urgency>,

    /// Where the ticket sits in the development process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_stage: Option<ProcessStage>,

    /// Optional due date (ISO 8601 `YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<NaiveDate>,

    /// Free-form tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Direct dependencies that must complete first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<LeafId>,

    /// Linked milestone, when one applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<LeafId>,

    /// Memory references stored as a list of single-key maps so YAML reads
    /// naturally: `- user: feedback-testing`, `- project: auth-stack-conventions`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<MemoryRef>,

    /// Free-form external links (e.g. `github_issue: pbower/...#42`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub links: BTreeMap<String, String>,

    /// Created timestamp (UTC ISO 8601). Set on first scaffold.
    pub created: DateTime<Utc>,

    /// Last-updated timestamp. Bumped on every write.
    pub updated: DateTime<Utc>,
}

/// Default status when the field is omitted from the YAML block.
fn default_status() -> Status {
    Status::Open
}

/// A typed reference to a memory file at a specific tier.
///
/// Serialises as a single-key map: `{ user: feedback-testing }`,
/// `{ project: auth-stack-conventions }`, `{ ticket: lock-design }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryRef {
    /// User-tier memory under `~/.claude/projects/*/memory/`.
    User(String),
    /// Project-tier memory under `.pm/projects/<slug>/memories/`.
    Project(String),
    /// Ticket-tier memory under the ticket's own `memories/` directory.
    Ticket(String),
}

impl FrontMatter {
    /// Build a minimal front-matter for a brand-new ticket. Sets `created`
    /// and `updated` to `Utc::now()`.
    pub fn new(id: LeafId, title: impl Into<String>) -> Self {
        let now = Utc::now();
        FrontMatter {
            id,
            parent: None,
            project: None,
            title: title.into(),
            status: Status::Open,
            priority: None,
            urgency: None,
            process_stage: None,
            due: None,
            tags: Vec::new(),
            deps: Vec::new(),
            milestone: None,
            memories: Vec::new(),
            links: BTreeMap::new(),
            created: now,
            updated: now,
        }
    }

    /// Serialise to a YAML fragment without delimiter lines. Callers wrap in
    /// `---\n...\n---\n` when writing to disk.
    pub fn to_yaml(&self) -> Result<String, FrontMatterError> {
        serde_yml::to_string(self).map_err(FrontMatterError::Yaml)
    }

    /// Parse a YAML fragment (no delimiters).
    pub fn from_yaml(yaml: &str) -> Result<Self, FrontMatterError> {
        let fm: FrontMatter = serde_yml::from_str(yaml).map_err(FrontMatterError::Yaml)?;
        Ok(fm)
    }
}

/// Parsed split of a `CLAUDE.md`-style file into (front-matter, body).
///
/// Body is the raw markdown after the closing `---` line, with the leading
/// newline stripped. Callers that want structured sections feed this into
/// [`crate::store::sections`] (added in a later commit).
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub front_matter: FrontMatter,
    pub body: String,
}

impl Document {
    /// Parse a complete CLAUDE.md-shaped string. The file must open with a
    /// `---` line, contain a YAML block, then a `---` closing line.
    pub fn parse(raw: &str) -> Result<Self, FrontMatterError> {
        let (yaml, body) = split_front_matter(raw)?;
        let fm = FrontMatter::from_yaml(yaml)?;
        Ok(Document {
            front_matter: fm,
            body: body.to_string(),
        })
    }

    /// Read a file path and parse it.
    pub fn read(path: &Path) -> Result<Self, FrontMatterError> {
        let raw = std::fs::read_to_string(path).map_err(FrontMatterError::Io)?;
        Self::parse(&raw)
    }

    /// Render to a complete CLAUDE.md-shaped string with delimiter lines.
    pub fn render(&self) -> Result<String, FrontMatterError> {
        let yaml = self.front_matter.to_yaml()?;
        let mut out = String::with_capacity(yaml.len() + self.body.len() + 16);
        out.push_str("---\n");
        out.push_str(yaml.trim_end_matches('\n'));
        out.push_str("\n---\n");
        if !self.body.is_empty() {
            out.push_str(&self.body);
            if !self.body.ends_with('\n') {
                out.push('\n');
            }
        }
        Ok(out)
    }
}

/// Split a string at the YAML front-matter delimiters. Returns the YAML body
/// (without the `---` lines) and the trailing markdown body.
///
/// Accepts an optional leading `\u{FEFF}` BOM. Requires the very first line to
/// be `---`; otherwise reports [`FrontMatterError::MissingOpenDelimiter`].
pub fn split_front_matter(raw: &str) -> Result<(&str, &str), FrontMatterError> {
    let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    let raw = raw
        .strip_prefix("\r\n")
        .or_else(|| raw.strip_prefix('\n'))
        .unwrap_or(raw);

    let first_line_end = raw
        .find('\n')
        .ok_or(FrontMatterError::MissingOpenDelimiter)?;
    let first_line = raw[..first_line_end].trim_end_matches('\r').trim_end();
    if first_line != "---" {
        return Err(FrontMatterError::MissingOpenDelimiter);
    }
    let rest = &raw[first_line_end + 1..];

    let mut search = rest;
    let mut consumed = 0usize;
    loop {
        let line_end = search
            .find('\n')
            .ok_or(FrontMatterError::MissingCloseDelimiter)?;
        let line = search[..line_end].trim_end_matches('\r');
        if line.trim_end() == "---" {
            let yaml = &rest[..consumed];
            let body_start = consumed + line_end + 1;
            let body = if body_start <= rest.len() {
                &rest[body_start..]
            } else {
                ""
            };
            // Strip one leading newline after the closing delimiter so callers
            // see a clean body rather than a stray blank line.
            let body = body.strip_prefix('\n').unwrap_or(body);
            return Ok((yaml, body));
        }
        consumed += line_end + 1;
        search = &search[line_end + 1..];
    }
}

/// Errors emitted by front-matter parsing / serialisation / IO.
#[derive(Debug)]
pub enum FrontMatterError {
    Io(std::io::Error),
    Yaml(serde_yml::Error),
    MissingOpenDelimiter,
    MissingCloseDelimiter,
}

impl std::fmt::Display for FrontMatterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrontMatterError::Io(e) => write!(f, "front-matter io: {e}"),
            FrontMatterError::Yaml(e) => write!(f, "front-matter yaml: {e}"),
            FrontMatterError::MissingOpenDelimiter => {
                write!(f, "front-matter must start with a `---` line")
            }
            FrontMatterError::MissingCloseDelimiter => {
                write!(f, "front-matter block is not terminated by a `---` line")
            }
        }
    }
}

impl std::error::Error for FrontMatterError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::id::TypePrefix;

    fn sample_id() -> LeafId {
        LeafId::new(TypePrefix::Task, 7)
    }

    #[test]
    fn new_initialises_required_fields() {
        let fm = FrontMatter::new(sample_id(), "Lock protocol");
        assert_eq!(fm.id, sample_id());
        assert_eq!(fm.title, "Lock protocol");
        assert_eq!(fm.status, Status::Open);
        assert!(fm.parent.is_none());
        assert!(fm.tags.is_empty());
        assert!(fm.memories.is_empty());
    }

    #[test]
    fn yaml_roundtrip_minimum_fields() {
        let fm = FrontMatter::new(sample_id(), "Lock protocol");
        let yaml = fm.to_yaml().unwrap();
        let back = FrontMatter::from_yaml(&yaml).unwrap();
        assert_eq!(back, fm);
    }

    #[test]
    fn yaml_roundtrip_full_fields() {
        let mut fm = FrontMatter::new(sample_id(), "Lock protocol");
        fm.parent = Some(LeafId::new(TypePrefix::Epic, 3));
        fm.project = Some(LeafId::new(TypePrefix::Project, 1));
        fm.priority = Some(Priority::MustHave);
        fm.urgency = Some(Urgency::UrgentImportant);
        fm.process_stage = Some(ProcessStage::Design);
        fm.due = NaiveDate::from_ymd_opt(2026, 5, 25);
        fm.tags = vec!["infra".into(), "locking".into()];
        fm.deps = vec![
            LeafId::new(TypePrefix::Task, 6),
            LeafId::new(TypePrefix::Task, 11),
        ];
        fm.milestone = Some(LeafId::new(TypePrefix::Milestone, 1));
        fm.memories = vec![
            MemoryRef::User("feedback-testing".into()),
            MemoryRef::Project("auth-stack-conventions".into()),
        ];
        fm.links
            .insert("github_issue".into(), "pbower/project_management#42".into());

        let yaml = fm.to_yaml().unwrap();
        let back = FrontMatter::from_yaml(&yaml).unwrap();
        assert_eq!(back, fm);
    }

    #[test]
    fn omitted_optional_fields_default_correctly() {
        let yaml = r#"
id: TSK7
title: Lock
created: 2026-05-12T11:33:00Z
updated: 2026-05-12T11:33:00Z
"#;
        let fm = FrontMatter::from_yaml(yaml).unwrap();
        assert_eq!(fm.status, Status::Open);
        assert!(fm.parent.is_none());
        assert!(fm.priority.is_none());
        assert!(fm.tags.is_empty());
        assert!(fm.deps.is_empty());
        assert!(fm.memories.is_empty());
        assert!(fm.links.is_empty());
    }

    #[test]
    fn missing_id_rejected() {
        let yaml = r#"
title: No id here
created: 2026-05-12T11:33:00Z
updated: 2026-05-12T11:33:00Z
"#;
        let err = FrontMatter::from_yaml(yaml).unwrap_err();
        assert!(matches!(err, FrontMatterError::Yaml(_)));
    }

    #[test]
    fn malformed_yaml_rejected() {
        let yaml = "id: [not, closed";
        let err = FrontMatter::from_yaml(yaml).unwrap_err();
        assert!(matches!(err, FrontMatterError::Yaml(_)));
    }

    #[test]
    fn document_split_strips_delimiters() {
        let raw = "---\nid: TSK7\ntitle: x\ncreated: 2026-05-12T11:33:00Z\nupdated: 2026-05-12T11:33:00Z\n---\n# Body\nhello\n";
        let (yaml, body) = split_front_matter(raw).unwrap();
        assert!(yaml.contains("id: TSK7"));
        assert_eq!(body, "# Body\nhello\n");
    }

    #[test]
    fn document_parse_round_trip() {
        let raw = "---\nid: TSK7\ntitle: x\ncreated: 2026-05-12T11:33:00Z\nupdated: 2026-05-12T11:33:00Z\n---\n# Body\n\nThe body.\n";
        let doc = Document::parse(raw).unwrap();
        assert_eq!(doc.front_matter.id, sample_id());
        assert_eq!(doc.body, "# Body\n\nThe body.\n");
        let back = doc.render().unwrap();
        let doc2 = Document::parse(&back).unwrap();
        assert_eq!(doc2.front_matter, doc.front_matter);
        assert_eq!(doc2.body, doc.body);
    }

    #[test]
    fn missing_open_delimiter_rejected() {
        let raw = "id: TSK7\ntitle: x\n---\nbody\n";
        let err = split_front_matter(raw).unwrap_err();
        assert!(matches!(err, FrontMatterError::MissingOpenDelimiter));
    }

    #[test]
    fn missing_close_delimiter_rejected() {
        let raw = "---\nid: TSK7\ntitle: x\ncreated: 2026-05-12T11:33:00Z\nupdated: 2026-05-12T11:33:00Z\n";
        let err = split_front_matter(raw).unwrap_err();
        assert!(matches!(err, FrontMatterError::MissingCloseDelimiter));
    }

    #[test]
    fn document_renders_without_body() {
        let mut doc = Document {
            front_matter: FrontMatter::new(sample_id(), "x"),
            body: String::new(),
        };
        // Pin timestamps for determinism.
        doc.front_matter.created = "2026-05-12T11:33:00Z".parse().unwrap();
        doc.front_matter.updated = "2026-05-12T11:33:00Z".parse().unwrap();
        let rendered = doc.render().unwrap();
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("id: TSK7"));
        assert!(rendered.ends_with("---\n"));
    }
}
