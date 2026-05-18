//! Section extraction and splice-back for the ticket editor.
//!
//! When the user picks a section in the editor (Description, User
//! Story, ...) and presses Enter, the editor extracts that section's
//! body to a temp file with no front-matter, no other sections, and no
//! `@artifacts/...` import line. `$EDITOR` opens the temp file. On
//! exit the splice writes the new body back into the right slot in
//! `CLAUDE.md`, leaving every other byte of the file untouched.
//!
//! The model is anchor-based: a section spans from its `# Heading`
//! line up to (but not including) the next `# Heading`, the trailing
//! `@artifacts/...` line, or EOF. Front-matter (between two `---`
//! lines at the top) is always skipped.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One named section in a CLAUDE.md body. `start_line` is the line
/// index of the `# Heading` (0-based); `body_start` is the first body
/// line; `body_end` is the first line that is NOT part of this
/// section's body (one past the last body line, or the file length).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub heading_line: usize,
    pub body_start: usize,
    pub body_end: usize,
}

impl Section {
    pub fn body_line_count(&self) -> usize {
        self.body_end.saturating_sub(self.body_start)
    }
}

/// Parse all level-1 sections out of a CLAUDE.md file. Skips the
/// YAML front-matter and stops a section at the next heading,
/// `@artifacts/...` line, or EOF.
pub fn parse_sections(path: &Path) -> io::Result<Vec<Section>> {
    let raw = fs::read_to_string(path)?;
    Ok(parse_sections_str(&raw))
}

pub fn parse_sections_str(raw: &str) -> Vec<Section> {
    let lines: Vec<&str> = raw.lines().collect();
    let start = body_start_line(&lines);

    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<Section> = None;

    for (idx, line) in lines.iter().enumerate().skip(start) {
        if is_artifact_import(line) {
            // Artifacts import is always the last meaningful line; if
            // we have an open section, close it here.
            if let Some(mut s) = current.take() {
                s.body_end = idx;
                sections.push(s);
            }
            break;
        }
        if let Some(name) = parse_heading(line) {
            if let Some(mut s) = current.take() {
                s.body_end = idx;
                sections.push(s);
            }
            current = Some(Section {
                name,
                heading_line: idx,
                body_start: idx + 1,
                body_end: idx + 1,
            });
            continue;
        }
    }
    if let Some(mut s) = current.take() {
        s.body_end = lines.len();
        sections.push(s);
    }

    // Trim trailing empty lines from each body so the body line count
    // does not double-count the blank that separates sections.
    for s in sections.iter_mut() {
        while s.body_end > s.body_start {
            let last = lines[s.body_end - 1].trim();
            if last.is_empty() {
                s.body_end -= 1;
            } else {
                break;
            }
        }
    }
    sections
}

/// Body lines for `section`, joined with `\n`. The trailing newline is
/// dropped so the temp file rendering decides whether to add one.
pub fn extract_body(path: &Path, section: &Section) -> io::Result<String> {
    let raw = fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().collect();
    let slice = &lines[section.body_start..section.body_end];
    Ok(slice.join("\n"))
}

/// Splice `new_body` into the section's slot, writing the modified
/// file atomically. The heading line is preserved untouched; only the
/// body lines between `body_start` and `body_end` are replaced.
pub fn splice_back(path: &Path, section: &Section, new_body: &str) -> io::Result<()> {
    let raw = fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().collect();

    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());

    // Before the section body: every line up to body_start.
    for line in &lines[..section.body_start] {
        out_lines.push((*line).to_string());
    }

    // New body. Empty body collapses to an empty section.
    let trimmed = new_body.trim_end_matches('\n');
    if !trimmed.is_empty() {
        for body_line in trimmed.lines() {
            out_lines.push(body_line.to_string());
        }
    }

    // Blank line between the new body and the next section keeps the
    // file readable for the next round-trip; only insert when the
    // following content is more body, not EOF.
    let next_idx = section.body_end;
    if next_idx < lines.len() {
        out_lines.push(String::new());
    }

    // After the body: every line from body_end onwards.
    for line in &lines[next_idx..] {
        out_lines.push((*line).to_string());
    }

    let mut serialised = out_lines.join("\n");
    if raw.ends_with('\n') && !serialised.ends_with('\n') {
        serialised.push('\n');
    }

    write_atomic(path, &serialised)
}

/// Add a new section heading just before the trailing
/// `@artifacts/...` import line (or at EOF if no import line is
/// present). Returns the parsed [`Section`] for the new entry.
pub fn add_section(path: &Path, name: &str) -> io::Result<Section> {
    let raw = fs::read_to_string(path)?;
    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();

    // Find the @artifacts import line, if any.
    let import_idx = lines.iter().position(|l| is_artifact_import(l));

    let heading = format!("# {name}");

    // We insert: blank, heading, blank.
    let insert_at = import_idx.unwrap_or(lines.len());
    let inserts = vec!["".to_string(), heading.clone(), "".to_string()];
    for (offset, line) in inserts.into_iter().enumerate() {
        lines.insert(insert_at + offset, line);
    }

    let mut serialised = lines.join("\n");
    if raw.ends_with('\n') && !serialised.ends_with('\n') {
        serialised.push('\n');
    }
    write_atomic(path, &serialised)?;

    // Re-parse so the returned Section has correct line indices.
    let sections = parse_sections_str(&serialised);
    sections
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "section did not parse back"))
}

/// Write a temp file containing only the section body and return its
/// path. The caller is responsible for cleanup after `$EDITOR` exits.
pub fn write_temp_for_section(
    pm_dir: &Path,
    ticket_id: &str,
    section: &Section,
    body: &str,
) -> io::Result<PathBuf> {
    let dir = pm_dir.join(".thunder").join("tmp");
    fs::create_dir_all(&dir)?;
    let safe_name = section
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase();
    let path = dir.join(format!("{ticket_id}-{safe_name}.md"));
    let mut body = body.to_string();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    write_atomic(&path, &body)?;
    Ok(path)
}

/// Read back a temp file edited in `$EDITOR`.
pub fn read_temp(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Best-effort cleanup of the temp file. Failure here is non-fatal.
pub fn cleanup_temp(path: &Path) {
    let _ = fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("# ") {
        Some(rest.trim().to_string())
    } else if trimmed == "#" {
        Some(String::new())
    } else {
        None
    }
}

fn is_artifact_import(line: &str) -> bool {
    let t = line.trim();
    t == "@artifacts/ARTIFACTS.md"
}

/// Return the first line index that is OUTSIDE the YAML front-matter.
/// Front-matter is the block between the first `---` and the next
/// `---`; if there is no front-matter the body starts at line 0.
fn body_start_line(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return 0;
    }
    for (idx, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            return idx + 1;
        }
    }
    // Unclosed front-matter is treated as no body. Returning lines.len()
    // means parse_sections will produce zero sections, which is the
    // right defensive behaviour for a malformed file.
    lines.len()
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nid: TSK7\ntitle: Foo\n---\n\n# Description\n\nFirst section\nbody.\n\n# User Story\n\nAs an agent\nI want X.\n\n@artifacts/ARTIFACTS.md\n";

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thunder-sections-{label}-{}-{}",
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
    fn parse_sections_finds_two() {
        let sections = parse_sections_str(SAMPLE);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "Description");
        assert_eq!(sections[1].name, "User Story");
    }

    #[test]
    fn body_extraction_drops_heading_and_trailing_blanks() {
        let sections = parse_sections_str(SAMPLE);
        let lines: Vec<&str> = SAMPLE.lines().collect();
        let desc = &sections[0];
        let body: Vec<&str> = lines[desc.body_start..desc.body_end].to_vec();
        assert!(body.iter().all(|l| !l.starts_with('#')));
        assert!(body.contains(&"First section"));
        assert!(body.contains(&"body."));
    }

    #[test]
    fn parse_handles_no_front_matter() {
        let raw = "# Description\nbody\n# Notes\nmore\n";
        let sections = parse_sections_str(raw);
        assert_eq!(sections.len(), 2);
    }

    #[test]
    fn parse_handles_no_artifact_import() {
        let raw = "---\nid: TSK1\n---\n\n# Description\nbody\n";
        let sections = parse_sections_str(raw);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "Description");
    }

    #[test]
    fn splice_replaces_only_the_named_section_body() {
        let dir = scratch_dir("splice");
        let p = dir.join("CLAUDE.md");
        fs::write(&p, SAMPLE).unwrap();
        let sections = parse_sections_str(SAMPLE);
        let desc = sections[0].clone();
        splice_back(&p, &desc, "Replaced body\nwith two lines").unwrap();
        let after = fs::read_to_string(&p).unwrap();
        assert!(after.contains("# Description"));
        assert!(after.contains("Replaced body"));
        assert!(!after.contains("First section"));
        // User Story untouched.
        assert!(after.contains("As an agent"));
        // Artifacts trailer untouched.
        assert!(after.contains("@artifacts/ARTIFACTS.md"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_section_inserts_before_artifact_import() {
        let dir = scratch_dir("add");
        let p = dir.join("CLAUDE.md");
        fs::write(&p, SAMPLE).unwrap();
        let new_section = add_section(&p, "Risk").unwrap();
        assert_eq!(new_section.name, "Risk");
        let after = fs::read_to_string(&p).unwrap();
        assert!(after.contains("# Risk"));
        let risk_pos = after.find("# Risk").unwrap();
        let artifact_pos = after.find("@artifacts/ARTIFACTS.md").unwrap();
        assert!(risk_pos < artifact_pos);
        fs::remove_dir_all(&dir).ok();
    }
}
