//! Section parser and writer for the CLAUDE.md body.
//!
//! A v2 ticket's CLAUDE.md body is structured by level-1 markdown headings
//! (`# Name`). Each heading starts a section that extends to the next level-1
//! heading or to EOF. This module parses a body into [`Section`] values,
//! supports targeted updates that leave other sections (including user-added
//! ones) untouched, and renders back to a markdown string.
//!
//! Behaviours:
//!
//! - **Level 1 only.** `##`, `###`, etc. are body content of the enclosing
//!   section, not new sections.
//! - **Fenced code blocks ignored.** Lines inside ```...``` blocks are never
//!   treated as headings.
//! - **Preamble preserved.** Anything before the first heading lives in
//!   [`ParsedBody::preamble`] and survives all updates.
//! - **Order preserved.** Reordered or user-added sections survive
//!   round-trips. We never re-sort.
//! - **Unknown sections tolerated.** Sections whose names do not match the
//!   ticket-kind template are kept as-is.

/// One markdown section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading text without the leading `# `.
    pub name: String,
    /// Markdown body between this heading and the next (or EOF). Includes the
    /// trailing newline of the last content line; never includes the heading
    /// line itself.
    pub body: String,
}

/// Parsed view of a markdown body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedBody {
    /// Anything before the first `# ` heading. Often empty for ticket bodies
    /// produced by the scaffolder; non-empty when a user has added prose
    /// above the first templated section.
    pub preamble: String,
    /// Sections in source order.
    pub sections: Vec<Section>,
}

impl ParsedBody {
    /// Walk `body` and split it into preamble + sections.
    pub fn parse(body: &str) -> Self {
        let mut sections: Vec<Section> = Vec::new();
        let mut preamble = String::new();
        let mut current: Option<Section> = None;
        let mut in_code_fence = false;

        for line in split_keep_endings(body) {
            // Track fenced code blocks so headings inside them are body content.
            if is_code_fence_line(line) {
                in_code_fence = !in_code_fence;
                push_line(&mut preamble, &mut current, line);
                continue;
            }

            if !in_code_fence {
                if let Some(name) = h1_heading_name(line) {
                    // Close the previous section (if any) before starting a new one.
                    if let Some(prev) = current.take() {
                        sections.push(close_section(prev));
                    }
                    current = Some(Section {
                        name: name.to_string(),
                        body: String::new(),
                    });
                    continue;
                }
            }
            push_line(&mut preamble, &mut current, line);
        }
        if let Some(last) = current.take() {
            sections.push(close_section(last));
        }
        ParsedBody { preamble, sections }
    }

    /// Render back to a markdown string. Headings are emitted in `# Name`
    /// form, separated from the preceding content by a single blank line for
    /// readability. Bodies are emitted verbatim; empty bodies do not produce
    /// a phantom blank line.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.preamble);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for (idx, section) in self.sections.iter().enumerate() {
            let need_gap = idx > 0 || !self.preamble.is_empty();
            if need_gap {
                // Exactly one blank line before the heading.
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                if !out.ends_with("\n\n") {
                    out.push('\n');
                }
            }
            out.push_str("# ");
            out.push_str(&section.name);
            out.push('\n');
            if !section.body.is_empty() {
                out.push_str(&section.body);
                if !section.body.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Find a section by exact name. Case-sensitive.
    pub fn find(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Mutable lookup by exact name.
    pub fn find_mut(&mut self, name: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.name == name)
    }

    /// Replace the body of `name` if it exists; otherwise append a new
    /// section with that name. Returns `true` if an existing section was
    /// updated, `false` if a new one was appended.
    pub fn upsert(&mut self, name: &str, body: impl Into<String>) -> bool {
        let body = body.into();
        if let Some(existing) = self.find_mut(name) {
            existing.body = body;
            true
        } else {
            self.sections.push(Section {
                name: name.to_string(),
                body,
            });
            false
        }
    }

    /// Remove a section by name. Returns the removed section if present.
    pub fn remove(&mut self, name: &str) -> Option<Section> {
        let idx = self.sections.iter().position(|s| s.name == name)?;
        Some(self.sections.remove(idx))
    }

    /// All section names in source order.
    pub fn names(&self) -> Vec<&str> {
        self.sections.iter().map(|s| s.name.as_str()).collect()
    }
}

/// Push a line to either the current section's body or the preamble buffer.
fn push_line(preamble: &mut String, current: &mut Option<Section>, line: &str) {
    match current {
        Some(section) => section.body.push_str(line),
        None => preamble.push_str(line),
    }
}

/// Strip trailing blank lines from a section's body so the inter-section gap
/// does not become part of the preceding section's content. Idempotent on
/// further calls; preserves a single trailing newline when there is content.
fn close_section(mut section: Section) -> Section {
    while section.body.ends_with("\n\n") {
        section.body.pop();
    }
    if section.body == "\n" {
        section.body.clear();
    }
    section
}

/// Split `s` into lines, keeping the trailing newline on each line. The final
/// line may or may not end in a newline.
fn split_keep_endings(s: &str) -> impl Iterator<Item = &str> {
    let mut start = 0usize;
    let bytes = s.as_bytes();
    std::iter::from_fn(move || {
        if start >= bytes.len() {
            return None;
        }
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'\n' {
            end += 1;
        }
        // Include the newline byte if present.
        let stop = if end < bytes.len() { end + 1 } else { end };
        let slice = &s[start..stop];
        start = stop;
        Some(slice)
    })
}

/// If `line` is an ATX level-1 heading, return the heading text (without the
/// leading `# `). Returns `None` for level >= 2 or non-headings.
fn h1_heading_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_end_matches(['\n', '\r']);
    let rest = trimmed.strip_prefix('#')?;
    // Reject `##...` by requiring the next char to be a space (or end-of-line
    // for empty headings, though we treat those as not-a-heading).
    if rest.starts_with('#') {
        return None;
    }
    let name = rest
        .strip_prefix(' ')
        .or_else(|| if rest.is_empty() { Some("") } else { None })?;
    if name.is_empty() {
        return None;
    }
    // ATX headings allow an optional trailing run of #s; strip those.
    let cleaned = name.trim_end_matches([' ', '\t']);
    let cleaned = cleaned.trim_end_matches('#');
    let cleaned = cleaned.trim_end_matches([' ', '\t']);
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned)
}

/// True if `line` is a fenced code-block delimiter (``` or ~~~ on its own
/// line, possibly with a language tag).
fn is_code_fence_line(line: &str) -> bool {
    let trimmed = line.trim_end_matches(['\n', '\r']).trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_body() {
        let pb = ParsedBody::parse("");
        assert!(pb.preamble.is_empty());
        assert!(pb.sections.is_empty());
    }

    #[test]
    fn parses_preamble_only() {
        let pb = ParsedBody::parse("Just some text.\n");
        assert_eq!(pb.preamble, "Just some text.\n");
        assert!(pb.sections.is_empty());
    }

    #[test]
    fn parses_single_section() {
        let body = "# Description\n\nWe need a heartbeat lock.\n";
        let pb = ParsedBody::parse(body);
        assert!(pb.preamble.is_empty());
        assert_eq!(pb.sections.len(), 1);
        assert_eq!(pb.sections[0].name, "Description");
        assert_eq!(pb.sections[0].body, "\nWe need a heartbeat lock.\n");
    }

    #[test]
    fn parses_multiple_sections_in_order() {
        let body = "# Description\nFoo.\n\n# User Story\nBar.\n\n# Requirements\nBaz.\n";
        let pb = ParsedBody::parse(body);
        let names: Vec<&str> = pb.names();
        assert_eq!(names, vec!["Description", "User Story", "Requirements"]);
    }

    #[test]
    fn ignores_level_2_plus_headings() {
        let body = "# Description\n## Subhead\nText.\n### Deeper\nMore.\n# Requirements\nReq.\n";
        let pb = ParsedBody::parse(body);
        assert_eq!(pb.names(), vec!["Description", "Requirements"]);
        // The Description section's body retains its level-2 / level-3 lines.
        assert!(pb.sections[0].body.contains("## Subhead"));
        assert!(pb.sections[0].body.contains("### Deeper"));
    }

    #[test]
    fn ignores_headings_inside_fenced_code() {
        let body = "# Real\nIntro.\n\n```\n# Not a heading\n```\n\n# Also Real\n";
        let pb = ParsedBody::parse(body);
        assert_eq!(pb.names(), vec!["Real", "Also Real"]);
    }

    #[test]
    fn preserves_user_added_section_on_round_trip() {
        let body = "# Description\nA.\n\n# Performance Notes\nMine.\n";
        let pb = ParsedBody::parse(body);
        let rendered = pb.render();
        let back = ParsedBody::parse(&rendered);
        assert_eq!(back, pb);
    }

    #[test]
    fn upsert_updates_existing_section() {
        let mut pb = ParsedBody::parse("# Description\nOld text.\n");
        let was_existing = pb.upsert("Description", "New text.\n");
        assert!(was_existing);
        assert_eq!(pb.find("Description").unwrap().body, "New text.\n");
    }

    #[test]
    fn upsert_appends_new_section() {
        let mut pb = ParsedBody::parse("# Description\nA.\n");
        let was_existing = pb.upsert("Notes", "B.\n");
        assert!(!was_existing);
        assert_eq!(pb.names(), vec!["Description", "Notes"]);
    }

    #[test]
    fn upsert_preserves_other_sections() {
        let body = "# Description\nA.\n\n# User Story\nB.\n\n# Requirements\nC.\n";
        let mut pb = ParsedBody::parse(body);
        pb.upsert("User Story", "Replaced.\n");
        assert_eq!(pb.find("Description").unwrap().body, "A.\n");
        assert_eq!(pb.find("User Story").unwrap().body, "Replaced.\n");
        assert_eq!(pb.find("Requirements").unwrap().body, "C.\n");
    }

    #[test]
    fn remove_drops_named_section() {
        let mut pb = ParsedBody::parse("# A\n1.\n\n# B\n2.\n\n# C\n3.\n");
        let removed = pb.remove("B").unwrap();
        assert_eq!(removed.name, "B");
        assert_eq!(pb.names(), vec!["A", "C"]);
    }

    #[test]
    fn render_round_trip_preserves_order() {
        // Sections deliberately not in alphabetical order.
        let body = "# Notes\nN.\n\n# Description\nD.\n\n# User Story\nU.\n";
        let pb = ParsedBody::parse(body);
        assert_eq!(pb.names(), vec!["Notes", "Description", "User Story"]);
        let rendered = pb.render();
        let again = ParsedBody::parse(&rendered);
        assert_eq!(again.names(), vec!["Notes", "Description", "User Story"]);
    }

    #[test]
    fn render_handles_section_with_no_body() {
        let body = "# Description\n# Requirements\n";
        let pb = ParsedBody::parse(body);
        assert_eq!(pb.names(), vec!["Description", "Requirements"]);
        assert!(pb.sections[0].body.is_empty());
        let rendered = pb.render();
        let again = ParsedBody::parse(&rendered);
        assert_eq!(again, pb);
    }

    #[test]
    fn preamble_survives_round_trip() {
        let body = "Some intro prose.\n\n# Description\nDesc.\n";
        let pb = ParsedBody::parse(body);
        assert_eq!(pb.preamble, "Some intro prose.\n\n");
        assert_eq!(pb.names(), vec!["Description"]);
        let rendered = pb.render();
        let again = ParsedBody::parse(&rendered);
        assert_eq!(again, pb);
    }

    #[test]
    fn h1_heading_name_rejects_level_2() {
        assert_eq!(h1_heading_name("## Subhead\n"), None);
        assert_eq!(h1_heading_name("# Real\n"), Some("Real"));
        assert_eq!(h1_heading_name("not a heading\n"), None);
        assert_eq!(h1_heading_name("#NoSpace\n"), None);
    }

    #[test]
    fn h1_heading_strips_optional_trailing_hashes() {
        // ATX allows `# Title #` style; we drop the trailing hashes.
        assert_eq!(h1_heading_name("# Title #\n"), Some("Title"));
        assert_eq!(h1_heading_name("# Title  ##\n"), Some("Title"));
    }

    #[test]
    fn code_fence_with_tilde_also_ignored() {
        let body = "# A\n~~~\n# fake\n~~~\n# B\n";
        let pb = ParsedBody::parse(body);
        assert_eq!(pb.names(), vec!["A", "B"]);
    }
}
