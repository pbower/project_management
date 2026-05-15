//! On-disk format for a memory file.
//!
//! A memory file is markdown with a YAML front-matter block compatible with
//! Claude Code's auto-memory schema. The body is free-form prose. PM neither
//! parses sections inside the body nor enforces a structure; the body is
//! preserved verbatim through read/write.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::store::front_matter::{split_front_matter, FrontMatterError};
use crate::store::state::atomic_write;

use super::scope::MemoryType;

/// Front-matter for a memory file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFrontMatter {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Nested `metadata` map. Claude Code stores `type` (and optionally
    /// other fields) here. PM exposes the type field directly; any other
    /// keys round-trip through `extra`.
    pub metadata: MemoryMetadata,
}

/// `metadata` block in the memory file's front-matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMetadata {
    #[serde(rename = "type")]
    pub kind: MemoryType,
}

/// One memory file: front-matter + body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFile {
    pub front_matter: MemoryFrontMatter,
    pub body: String,
}

impl MemoryFile {
    pub fn new(name: impl Into<String>, kind: MemoryType, body: impl Into<String>) -> Self {
        MemoryFile {
            front_matter: MemoryFrontMatter {
                name: name.into(),
                description: None,
                metadata: MemoryMetadata { kind },
            },
            body: body.into(),
        }
    }

    /// Parse the contents of a memory `.md` file.
    pub fn parse(raw: &str) -> Result<Self, MemoryFileError> {
        let (yaml, body) = split_front_matter(raw).map_err(MemoryFileError::FrontMatter)?;
        let fm: MemoryFrontMatter = serde_yml::from_str(yaml).map_err(MemoryFileError::Yaml)?;
        Ok(MemoryFile { front_matter: fm, body: body.to_string() })
    }

    /// Load and parse a memory file from disk.
    pub fn read(path: &Path) -> Result<Self, MemoryFileError> {
        let raw = fs::read_to_string(path).map_err(MemoryFileError::Io)?;
        Self::parse(&raw)
    }

    /// Render to a complete markdown string with the YAML delimiter lines.
    pub fn render(&self) -> Result<String, MemoryFileError> {
        let yaml = serde_yml::to_string(&self.front_matter).map_err(MemoryFileError::Yaml)?;
        let mut out = String::with_capacity(yaml.len() + self.body.len() + 8);
        out.push_str("---\n");
        out.push_str(yaml.trim_end_matches('\n'));
        out.push_str("\n---\n");
        if !self.body.is_empty() {
            // A blank line between the closing delimiter and the body keeps
            // the file readable when opened in an editor.
            if !self.body.starts_with('\n') {
                out.push('\n');
            }
            out.push_str(&self.body);
            if !self.body.ends_with('\n') {
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// Atomic write to `path`. Creates the parent directory if missing.
    pub fn write(&self, path: &Path) -> Result<(), MemoryFileError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(MemoryFileError::Io)?;
        }
        let rendered = self.render()?;
        atomic_write(path, rendered.as_bytes()).map_err(MemoryFileError::Io)
    }
}

/// Errors emitted by memory-file IO and parsing.
#[derive(Debug)]
pub enum MemoryFileError {
    Io(io::Error),
    Yaml(serde_yml::Error),
    FrontMatter(FrontMatterError),
}

impl std::fmt::Display for MemoryFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryFileError::Io(e) => write!(f, "memory file io: {e}"),
            MemoryFileError::Yaml(e) => write!(f, "memory file yaml: {e}"),
            MemoryFileError::FrontMatter(e) => write!(f, "memory file front-matter: {e}"),
        }
    }
}

impl std::error::Error for MemoryFileError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-memory-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn render_round_trip() {
        let mut mf = MemoryFile::new("feedback-testing", MemoryType::Feedback, "");
        mf.front_matter.description = Some("Real DB in tests".into());
        mf.body = "Integration tests must hit a real database.\n".into();

        let rendered = mf.render().unwrap();
        let back = MemoryFile::parse(&rendered).unwrap();
        assert_eq!(back, mf);
    }

    #[test]
    fn render_includes_yaml_delimiters() {
        let mf = MemoryFile::new("a", MemoryType::User, "body\n");
        let rendered = mf.render().unwrap();
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("\n---\n"));
        assert!(rendered.contains("name: a"));
        assert!(rendered.contains("metadata:"));
        assert!(rendered.ends_with("body\n"));
    }

    #[test]
    fn parse_rejects_malformed_yaml() {
        let raw = "---\nname: [broken\n---\nbody\n";
        let err = MemoryFile::parse(raw).unwrap_err();
        assert!(matches!(err, MemoryFileError::Yaml(_)));
    }

    #[test]
    fn parse_rejects_missing_delimiters() {
        let raw = "name: x\nbody\n";
        let err = MemoryFile::parse(raw).unwrap_err();
        assert!(matches!(err, MemoryFileError::FrontMatter(_)));
    }

    #[test]
    fn write_then_read_round_trip_on_disk() {
        let dir = tmp_dir();
        let path = dir.join("test.md");
        let mut mf = MemoryFile::new("auth", MemoryType::Reference, "Reference body.\n");
        mf.front_matter.description = Some("Auth conventions".into());
        mf.write(&path).unwrap();
        let back = MemoryFile::read(&path).unwrap();
        assert_eq!(back, mf);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_creates_missing_parent_directories() {
        let dir = tmp_dir();
        let path = dir.join("a/b/c/inside.md");
        let mf = MemoryFile::new("x", MemoryType::Project, "body\n");
        mf.write(&path).unwrap();
        assert!(path.is_file());
        std::fs::remove_dir_all(&dir).ok();
    }
}
