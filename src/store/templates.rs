//! Per-kind section templates for CLAUDE.md bodies.
//!
//! Each ticket kind has a default set of section headings (PM_DESIGN.md
//! Section 6.5). Templates are resolved in priority order:
//!
//! 1. `.pm/templates/<kind>.md` - project-local override.
//! 2. `~/.pm-templates/<kind>.md` - user-global override.
//! 3. Built-in default compiled into the binary.
//!
//! A template is a plain markdown file containing only level-1 section
//! headings (no front-matter, no body content). [`apply`] uses the template
//! to scaffold or update a [`ParsedBody`] without overwriting existing
//! content in matching sections or dropping user-added sections.

use std::path::{Path, PathBuf};

use super::id::TypePrefix;
use super::sections::ParsedBody;

const PROJECT_TEMPLATE: &str = include_str!("templates/project.md");
const PRODUCT_TEMPLATE: &str = include_str!("templates/product.md");
const EPIC_TEMPLATE: &str = include_str!("templates/epic.md");
const TASK_TEMPLATE: &str = include_str!("templates/task.md");
const SUBTASK_TEMPLATE: &str = include_str!("templates/subtask.md");
const MILESTONE_TEMPLATE: &str = include_str!("templates/milestone.md");

/// Filename stem used for template lookups (e.g. `task` for `Kind::Task`).
pub fn template_stem(prefix: TypePrefix) -> &'static str {
    match prefix {
        TypePrefix::Project => "project",
        TypePrefix::Product => "product",
        TypePrefix::Epic => "epic",
        TypePrefix::Task => "task",
        TypePrefix::Subtask => "subtask",
        TypePrefix::Milestone => "milestone",
    }
}

/// The built-in default template for a ticket kind. Used when no project- or
/// user-level override is found, and also returned as the canonical template
/// when callers want to preview defaults.
pub fn builtin(prefix: TypePrefix) -> &'static str {
    match prefix {
        TypePrefix::Project => PROJECT_TEMPLATE,
        TypePrefix::Product => PRODUCT_TEMPLATE,
        TypePrefix::Epic => EPIC_TEMPLATE,
        TypePrefix::Task => TASK_TEMPLATE,
        TypePrefix::Subtask => SUBTASK_TEMPLATE,
        TypePrefix::Milestone => MILESTONE_TEMPLATE,
    }
}

/// Where a resolved template came from. Useful for diagnostics and for
/// `pm template edit <kind>` so the binary knows whether to copy the built-in
/// out to the override location before launching `$EDITOR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateSource {
    /// Loaded from `.pm/templates/<kind>.md`.
    Project(PathBuf),
    /// Loaded from `~/.pm-templates/<kind>.md`.
    User(PathBuf),
    /// Compiled into the binary; no on-disk file.
    Builtin,
}

/// Result of a template resolution: the markdown content and where it came
/// from. The content is always the full template string, even for built-ins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTemplate {
    pub content: String,
    pub source: TemplateSource,
}

/// Resolve a template by walking the override chain.
///
/// `pm_root` is the `.pm/` directory (i.e. `Layout::root`). `home_dir` is
/// usually `dirs::home_dir()`; passed in so callers without a real HOME (e.g.
/// tests) can substitute a stub.
pub fn resolve(prefix: TypePrefix, pm_root: &Path, home_dir: Option<&Path>) -> ResolvedTemplate {
    let stem = template_stem(prefix);
    let project_path = pm_root.join("templates").join(format!("{stem}.md"));
    if let Ok(content) = std::fs::read_to_string(&project_path) {
        return ResolvedTemplate { content, source: TemplateSource::Project(project_path) };
    }
    if let Some(home) = home_dir {
        let user_path = home.join(".pm-templates").join(format!("{stem}.md"));
        if let Ok(content) = std::fs::read_to_string(&user_path) {
            return ResolvedTemplate { content, source: TemplateSource::User(user_path) };
        }
    }
    ResolvedTemplate {
        content: builtin(prefix).to_string(),
        source: TemplateSource::Builtin,
    }
}

/// Apply a template to a parsed body. For each section in the template:
///
/// - If `body` already has a section with that exact name, keep its content
///   (the template provides only the heading; existing content wins).
/// - Otherwise, append the section with an empty body.
///
/// Sections in `body` that are not in the template are kept (we never drop
/// user-added sections). Order is template-first for newly-added sections,
/// then any leftover body sections in their original order.
pub fn apply(template: &str, body: &mut ParsedBody) {
    let template_pb = ParsedBody::parse(template);
    let template_names: Vec<String> = template_pb
        .sections
        .iter()
        .map(|s| s.name.clone())
        .collect();
    if template_names.is_empty() {
        return;
    }

    // Take the existing sections out so we can rebuild order.
    let mut existing: Vec<super::sections::Section> = std::mem::take(&mut body.sections);

    // First, emit template-named sections in template order; preserve content
    // where the existing body had it, otherwise emit empty.
    for name in &template_names {
        if let Some(pos) = existing.iter().position(|s| &s.name == name) {
            body.sections.push(existing.remove(pos));
        } else {
            body.sections.push(super::sections::Section {
                name: name.clone(),
                body: String::new(),
            });
        }
    }

    // Whatever is left in `existing` was added by the user (or by a previous
    // template that defined sections this template does not). Append in
    // their original order so they are not silently dropped.
    for leftover in existing {
        body.sections.push(leftover);
    }
}

/// Convenience: scaffold an empty body from the template. Equivalent to
/// `apply(template, &mut ParsedBody::default())`.
pub fn scaffold(template: &str) -> ParsedBody {
    let mut pb = ParsedBody::default();
    apply(template, &mut pb);
    pb
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-store-templates-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builtin_templates_have_expected_sections() {
        let task = ParsedBody::parse(builtin(TypePrefix::Task));
        assert_eq!(
            task.names(),
            vec!["Description", "User Story", "Requirements", "Acceptance Criteria", "Notes"],
        );

        let project = ParsedBody::parse(builtin(TypePrefix::Project));
        assert_eq!(
            project.names(),
            vec!["Vision", "Scope", "Goals", "Stakeholders", "Notes"],
        );

        let milestone = ParsedBody::parse(builtin(TypePrefix::Milestone));
        assert_eq!(
            milestone.names(),
            vec!["Goal", "Tasks Included", "Definition of Done", "Notes"],
        );
    }

    #[test]
    fn scaffold_produces_empty_body_in_template_order() {
        let pb = scaffold(builtin(TypePrefix::Subtask));
        assert_eq!(pb.names(), vec!["Description", "Notes"]);
        for s in &pb.sections {
            assert!(s.body.is_empty(), "fresh scaffold section should have empty body");
        }
    }

    #[test]
    fn apply_preserves_existing_content_in_matching_sections() {
        let mut pb = ParsedBody::default();
        pb.upsert("Description", "We need a heartbeat lock.\n");
        pb.upsert("User Story", "As an agent, I want my lock to release...\n");

        apply(builtin(TypePrefix::Task), &mut pb);

        // Description and User Story content survives.
        assert_eq!(
            pb.find("Description").unwrap().body,
            "We need a heartbeat lock.\n",
        );
        assert_eq!(
            pb.find("User Story").unwrap().body,
            "As an agent, I want my lock to release...\n",
        );
        // Missing template sections are added with empty bodies.
        assert!(pb.find("Requirements").unwrap().body.is_empty());
        assert!(pb.find("Acceptance Criteria").unwrap().body.is_empty());
        assert!(pb.find("Notes").unwrap().body.is_empty());
    }

    #[test]
    fn apply_keeps_user_added_sections() {
        let mut pb = ParsedBody::default();
        pb.upsert("Description", "Desc.\n");
        pb.upsert("Performance Notes", "User-added.\n");

        apply(builtin(TypePrefix::Task), &mut pb);

        // Template sections come first, then leftover user-added.
        let names = pb.names();
        assert_eq!(&names[..5], &[
            "Description", "User Story", "Requirements", "Acceptance Criteria", "Notes",
        ]);
        assert_eq!(names[5], "Performance Notes");
        assert_eq!(pb.find("Performance Notes").unwrap().body, "User-added.\n");
    }

    #[test]
    fn apply_is_idempotent_on_already_templated_body() {
        let mut pb = scaffold(builtin(TypePrefix::Task));
        pb.upsert("Description", "Some content.\n");
        let before = pb.clone();

        apply(builtin(TypePrefix::Task), &mut pb);
        assert_eq!(pb, before);
    }

    #[test]
    fn resolve_returns_builtin_when_no_override() {
        let dir = tmp_dir();
        let pm_root = dir.join(".pm");
        fs::create_dir_all(&pm_root).unwrap();
        let resolved = resolve(TypePrefix::Task, &pm_root, Some(&dir.join("nope")));
        assert!(matches!(resolved.source, TemplateSource::Builtin));
        assert_eq!(resolved.content, builtin(TypePrefix::Task));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_prefers_project_override() {
        let dir = tmp_dir();
        let pm_root = dir.join(".pm");
        let templates_dir = pm_root.join("templates");
        fs::create_dir_all(&templates_dir).unwrap();
        let override_content = "# Custom\n\n# Sections\n";
        fs::write(templates_dir.join("task.md"), override_content).unwrap();

        let resolved = resolve(TypePrefix::Task, &pm_root, None);
        match &resolved.source {
            TemplateSource::Project(p) => assert_eq!(p, &templates_dir.join("task.md")),
            other => panic!("expected Project source, got {other:?}"),
        }
        assert_eq!(resolved.content, override_content);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_falls_back_to_user_when_no_project_override() {
        let dir = tmp_dir();
        let pm_root = dir.join(".pm");
        fs::create_dir_all(&pm_root).unwrap();
        let home = dir.join("home");
        let user_dir = home.join(".pm-templates");
        fs::create_dir_all(&user_dir).unwrap();
        let user_content = "# User Override\n";
        fs::write(user_dir.join("subtask.md"), user_content).unwrap();

        let resolved = resolve(TypePrefix::Subtask, &pm_root, Some(&home));
        match &resolved.source {
            TemplateSource::User(p) => assert_eq!(p, &user_dir.join("subtask.md")),
            other => panic!("expected User source, got {other:?}"),
        }
        assert_eq!(resolved.content, user_content);
        fs::remove_dir_all(&dir).ok();
    }
}
