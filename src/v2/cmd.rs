//! Dispatcher for `pm v2 <verb>`.
//!
//! Each branch delegates to a small handler that uses the [`crate::store`]
//! modules. Commands deferred to later phases (workflow, memory, tv, log)
//! print a clear "deferred to Phase N" message so the surface is shape-
//! complete even before the underlying machinery exists.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{NaiveDate, Utc};

use crate::store::artifacts::{rename_artifact, sweep_dir, ArtifactsIndex, ARTIFACTS_MD};
use crate::store::front_matter::MemoryRef;
use crate::store::id::IdInput;
use crate::store::sections::ParsedBody;
use crate::store::{
    aliases::Aliases, claude_md::CLAUDE_MD, templates, ArtifactError, FrontMatter, ItemEntry,
    Layout, LeafId, Resolver, State, Ticket, TicketError, TypePrefix,
};
use crate::fields::{Priority, Status};

use super::cli::{
    ArtifactAction, KindArg, MemoryAction, TemplateAction, V2Commands,
};
use super::root;

/// Public entry point invoked from `main.rs`.
pub fn run(cmd: V2Commands, pm_root_arg: Option<PathBuf>) -> Result<(), CommandError> {
    match cmd {
        V2Commands::Init => init(pm_root_arg.as_deref()),
        V2Commands::Add { title, kind, parent, slug } => {
            add(pm_root_arg.as_deref(), &title, kind, parent.as_deref(), slug.as_deref())
        }
        V2Commands::List { kind, tree } => list(pm_root_arg.as_deref(), kind, tree),
        V2Commands::Show { id } => show(pm_root_arg.as_deref(), &id),
        V2Commands::Move { id, dest } => move_ticket(pm_root_arg.as_deref(), &id, &dest),
        V2Commands::Complete { id } => set_status(pm_root_arg.as_deref(), &id, "done"),
        V2Commands::Delete { id, force } => delete(pm_root_arg.as_deref(), &id, force),
        V2Commands::Edit { id, section } => edit(pm_root_arg.as_deref(), &id, section.as_deref()),
        V2Commands::Context { id, no_memories } => context(pm_root_arg.as_deref(), &id, no_memories),
        V2Commands::Materialise { id, output } => {
            materialise(pm_root_arg.as_deref(), &id, output.as_deref())
        }
        V2Commands::Artifact { action } => artifact(pm_root_arg.as_deref(), action),
        V2Commands::Template { action } => template(pm_root_arg.as_deref(), action),
        V2Commands::Status { id, value } => set_status(pm_root_arg.as_deref(), &id, &value),
        V2Commands::Priority { id, value } => set_priority(pm_root_arg.as_deref(), &id, &value),
        V2Commands::Due { id, value } => set_due(pm_root_arg.as_deref(), &id, &value),
        V2Commands::Dep { id, op, other } => dep(pm_root_arg.as_deref(), &id, &op, &other),
        V2Commands::Tag { id, ops } => tag(pm_root_arg.as_deref(), &id, &ops),
        V2Commands::Link { id, key, value } => link(pm_root_arg.as_deref(), &id, &key, &value),
        V2Commands::Milestone { id, value } => milestone(pm_root_arg.as_deref(), &id, &value),
        V2Commands::Memory { action } => memory(action),
        V2Commands::Checkout { id, intent } => stub_workflow("checkout", &id, intent.as_deref()),
        V2Commands::Checkin { id, summary } => stub_workflow("checkin", &id, summary.as_deref()),
        V2Commands::Next { agent, filter } => stub_next(agent.as_deref(), filter.as_deref()),
        V2Commands::Locks => stub_simple("locks", "Phase 6"),
        V2Commands::Tv => stub_simple("tv", "Phase 9"),
        V2Commands::Log { id } => stub_log(&id),
        V2Commands::Search { query } => search(pm_root_arg.as_deref(), &query),
        V2Commands::Doctor => doctor(pm_root_arg.as_deref()),
    }
}

// ---------------------------------------------------------------- lifecycle

fn init(pm_root_arg: Option<&Path>) -> Result<(), CommandError> {
    let layout = root::resolve_for_init(pm_root_arg).map_err(CommandError::Root)?;
    layout.init().map_err(CommandError::Layout)?;
    println!("Initialised .pm/ at {}", layout.root.display());
    Ok(())
}

fn add(
    pm_root_arg: Option<&Path>,
    title: &str,
    kind: KindArg,
    parent_arg: Option<&str>,
    slug_arg: Option<&str>,
) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let mut state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;

    let prefix: TypePrefix = kind.into();
    let leaf = state.allocate(prefix);
    let slug = match slug_arg {
        Some(s) => s.to_string(),
        None => crate::v2::cmd::util::slugify(title),
    };

    // Compute the on-disk address chain.
    let (address_path, parent_leaf) = match parent_arg {
        None => (
            layout
                .orphan_directory_for(leaf, &slug)
                .map_err(CommandError::Layout)?,
            None,
        ),
        Some(p_raw) => {
            let resolver = Resolver::new(&layout, &state, &aliases);
            let resolved = resolver.resolve(p_raw).map_err(CommandError::Resolve)?;
            let parent_dir = resolved.relative_path.clone();
            let parent_leaf = resolved.leaf;
            // The new ticket's directory sits under the parent's type-folder
            // (e.g. parent EPC3 lives at `.../epics/checkouts`, so the new
            // task goes to `.../epics/checkouts/tasks/<slug>`).
            let child = parent_dir.join(prefix.type_folder()).join(&slug);
            (child, Some(parent_leaf))
        }
    };

    let abs_dir = layout
        .ensure_node_path(&address_path)
        .map_err(CommandError::Layout)?;

    let mut ticket = Ticket::scaffold(leaf, title, &slug, templates::builtin(prefix));
    ticket.front_matter.parent = parent_leaf;
    ticket.front_matter.project = compute_project_id(&state, &address_path);
    ticket.write_to(&abs_dir).map_err(CommandError::Ticket)?;

    state.insert(leaf, ItemEntry { path: address_path.clone() });
    state.save(&layout.state_path()).map_err(CommandError::State)?;

    println!("Created {} at {}", leaf, address_path.display());
    Ok(())
}

fn list(pm_root_arg: Option<&Path>, kind: Option<KindArg>, tree: bool) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let mut entries: Vec<(LeafId, &ItemEntry)> = state.items.iter().map(|(k, v)| (*k, v)).collect();
    entries.sort_by_key(|(leaf, _)| (leaf.prefix(), leaf.number()));

    let filter = kind.map(TypePrefix::from);
    for (leaf, entry) in &entries {
        if let Some(want) = filter {
            if leaf.prefix() != want { continue; }
        }
        let title = read_title(&layout, &entry.path).unwrap_or_else(|_| "<unreadable>".into());
        if tree {
            // For Phase 4, "tree" mode just indents by depth in the address.
            let depth = entry.path.components().filter(|c| {
                let s = c.as_os_str().to_string_lossy();
                matches!(s.as_ref(), "projects" | "products" | "epics" | "tasks" | "subtasks" | "milestones")
            }).count();
            let indent = "  ".repeat(depth.saturating_sub(1));
            println!("{indent}{} {}", leaf, title);
        } else {
            println!("{} {}", leaf, title);
        }
    }
    Ok(())
}

fn show(pm_root_arg: Option<&Path>, id: &str) -> Result<(), CommandError> {
    let (layout, leaf, rel) = load_ticket_path(pm_root_arg, id)?;
    let _ = layout;
    let path = rel.join(CLAUDE_MD);
    let raw = fs::read_to_string(&path).map_err(CommandError::Io)?;
    println!("== {} ==", leaf);
    print!("{}", raw);
    Ok(())
}

fn delete(pm_root_arg: Option<&Path>, id: &str, force: bool) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let mut state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;

    let leaf = {
        let resolver = Resolver::new(&layout, &state, &aliases);
        let resolved = resolver.resolve(id).map_err(CommandError::Resolve)?;
        resolved.leaf
    };
    let entry = state.lookup(leaf).cloned();
    if !force {
        let answer = prompt(&format!("Delete {} and its directory? [y/N] ", leaf))?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Aborted.");
            return Ok(());
        }
    }
    if let Some(entry) = &entry {
        let abs = layout.root.join(&entry.path);
        if abs.exists() {
            fs::remove_dir_all(&abs).map_err(CommandError::Io)?;
        }
    }
    state.tombstone(leaf);
    state.save(&layout.state_path()).map_err(CommandError::State)?;
    println!("Tombstoned {}", leaf);
    Ok(())
}

// ----------------------------------------------------------- metadata verbs

fn set_status(pm_root_arg: Option<&Path>, id: &str, value: &str) -> Result<(), CommandError> {
    let (layout, _leaf, abs_dir) = load_abs_dir(pm_root_arg, id)?;
    mutate_front_matter(&abs_dir, |fm| {
        fm.status = parse_status(value)?;
        Ok(())
    })?;
    let _ = layout;
    println!("status set");
    Ok(())
}

fn set_priority(pm_root_arg: Option<&Path>, id: &str, value: &str) -> Result<(), CommandError> {
    let (_, _, abs_dir) = load_abs_dir(pm_root_arg, id)?;
    mutate_front_matter(&abs_dir, |fm| {
        fm.priority = if value == "none" || value == "clear" { None } else { Some(parse_priority(value)?) };
        Ok(())
    })?;
    println!("priority set");
    Ok(())
}

fn set_due(pm_root_arg: Option<&Path>, id: &str, value: &str) -> Result<(), CommandError> {
    let (_, _, abs_dir) = load_abs_dir(pm_root_arg, id)?;
    mutate_front_matter(&abs_dir, |fm| {
        fm.due = if value == "none" || value == "clear" {
            None
        } else {
            Some(NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|e| CommandError::Parse(format!("invalid date {value:?}: {e}")))?)
        };
        Ok(())
    })?;
    println!("due set");
    Ok(())
}

fn dep(pm_root_arg: Option<&Path>, id: &str, op: &str, other: &str) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;
    let other_leaf = {
        let resolver = Resolver::new(&layout, &state, &aliases);
        resolver.resolve(other).map_err(CommandError::Resolve)?.leaf
    };
    let (_, _, abs_dir) = load_abs_dir(Some(layout.root.as_path()), id)?;
    mutate_front_matter(&abs_dir, |fm| {
        match op {
            "needs" | "add" => {
                if !fm.deps.contains(&other_leaf) {
                    fm.deps.push(other_leaf);
                }
            }
            "drop" | "remove" => {
                fm.deps.retain(|d| *d != other_leaf);
            }
            other => return Err(CommandError::Parse(format!("unknown dep op {other:?}; use `needs` or `drop`"))),
        }
        Ok(())
    })?;
    println!("dep {op} {other_leaf}");
    Ok(())
}

fn tag(pm_root_arg: Option<&Path>, id: &str, ops: &[String]) -> Result<(), CommandError> {
    let (_, _, abs_dir) = load_abs_dir(pm_root_arg, id)?;
    mutate_front_matter(&abs_dir, |fm| {
        for op in ops {
            if let Some(rest) = op.strip_prefix('+') {
                if !fm.tags.iter().any(|t| t == rest) {
                    fm.tags.push(rest.to_string());
                }
            } else if let Some(rest) = op.strip_prefix('-') {
                fm.tags.retain(|t| t != rest);
            } else {
                return Err(CommandError::Parse(format!("tag op {op:?} must start with + or -")));
            }
        }
        Ok(())
    })?;
    println!("tags updated");
    Ok(())
}

fn link(pm_root_arg: Option<&Path>, id: &str, key: &str, value: &str) -> Result<(), CommandError> {
    let (_, _, abs_dir) = load_abs_dir(pm_root_arg, id)?;
    mutate_front_matter(&abs_dir, |fm| {
        if value == "none" || value == "clear" {
            fm.links.remove(key);
        } else {
            fm.links.insert(key.to_string(), value.to_string());
        }
        Ok(())
    })?;
    println!("link {key} set");
    Ok(())
}

fn milestone(pm_root_arg: Option<&Path>, id: &str, value: &str) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;
    let target = if value == "none" || value == "clear" {
        None
    } else {
        let resolver = Resolver::new(&layout, &state, &aliases);
        Some(resolver.resolve(value).map_err(CommandError::Resolve)?.leaf)
    };
    let (_, _, abs_dir) = load_abs_dir(Some(layout.root.as_path()), id)?;
    mutate_front_matter(&abs_dir, |fm| {
        fm.milestone = target;
        Ok(())
    })?;
    println!("milestone set");
    Ok(())
}

// ----------------------------------------------------------------- move

fn move_ticket(pm_root_arg: Option<&Path>, id: &str, dest: &str) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let mut state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let mut aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;

    let leaf = {
        let resolver = Resolver::new(&layout, &state, &aliases);
        resolver.resolve(id).map_err(CommandError::Resolve)?.leaf
    };
    let entry = state.lookup(leaf).cloned()
        .ok_or_else(|| CommandError::Parse(format!("{} not in state.json", leaf)))?;
    let old_rel = entry.path.clone();
    let slug = old_rel
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| CommandError::Parse("ticket has no slug component".into()))?;

    // Compute the new parent directory.
    let new_parent_rel = if dest == ":orphan" {
        None
    } else {
        let resolver = Resolver::new(&layout, &state, &aliases);
        let resolved = resolver.resolve(dest).map_err(CommandError::Resolve)?;
        Some((resolved.leaf, resolved.relative_path))
    };

    let new_rel = match new_parent_rel.as_ref() {
        Some((_, parent_path)) => parent_path.join(leaf.prefix().type_folder()).join(&slug),
        None => PathBuf::from(leaf.prefix().type_folder()).join(&slug),
    };
    if let Some(parent) = layout.root.join(&new_rel).parent() {
        fs::create_dir_all(parent).map_err(CommandError::Io)?;
    }
    fs::rename(layout.root.join(&old_rel), layout.root.join(&new_rel))
        .map_err(CommandError::Io)?;

    // Update the ticket's front-matter parent field.
    {
        let claude_md = layout.root.join(&new_rel).join(CLAUDE_MD);
        let mut ticket = Ticket::read(&claude_md).map_err(CommandError::Ticket)?;
        ticket.front_matter.parent = new_parent_rel.as_ref().map(|(p, _)| *p);
        ticket.front_matter.project = compute_project_id(&state, &new_rel);
        ticket.write_to(claude_md.parent().unwrap()).map_err(CommandError::Ticket)?;
    }

    // Record an alias from the old address to the new address so old refs
    // keep resolving. Compute both BEFORE updating state.items so the
    // address walker still sees the ticket at its old location.
    let old_addr = compute_address(&state, &old_rel).ok();
    let new_parent_addr = new_parent_rel
        .as_ref()
        .and_then(|(_, parent_path)| compute_address(&state, parent_path).ok());
    let new_addr = match new_parent_addr {
        Some(prefix) => format!("{prefix}-{leaf}"),
        None => leaf.to_string(),
    };
    if let Some(old_addr) = old_addr {
        if old_addr != new_addr {
            aliases.add(old_addr, new_addr);
        }
    }
    state.insert(leaf, ItemEntry { path: new_rel.clone() });
    state.save(&layout.state_path()).map_err(CommandError::State)?;
    aliases.save(&layout.aliases_path()).map_err(CommandError::State)?;

    println!("Moved {} to {}", leaf, new_rel.display());
    Ok(())
}

// ----------------------------------------------------------- content verbs

fn edit(pm_root_arg: Option<&Path>, id: &str, section: Option<&str>) -> Result<(), CommandError> {
    let (_, _, abs_dir) = load_abs_dir(pm_root_arg, id)?;
    let path = abs_dir.join(CLAUDE_MD);
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let mut cmd = Command::new(&editor);
    if let Some(name) = section {
        // nvim/vim: +/<pattern>. Other editors typically ignore unknown args.
        match Path::new(&editor).file_name().and_then(|s| s.to_str()) {
            Some("nvim") | Some("vim") | Some("vi") => {
                cmd.arg(format!("+/^# {name}$"));
            }
            Some("nano") | Some("emacs") => {
                if let Ok(raw) = fs::read_to_string(&path) {
                    if let Some(line) = find_section_line(&raw, name) {
                        cmd.arg(format!("+{line}"));
                    }
                }
            }
            _ => {}
        }
    }
    cmd.arg(&path);
    let status = cmd.status().map_err(CommandError::Io)?;
    if !status.success() {
        return Err(CommandError::Parse(format!("{editor} exited with {status}")));
    }
    Ok(())
}

fn context(pm_root_arg: Option<&Path>, id: &str, _no_memories: bool) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;
    let leaf = {
        let resolver = Resolver::new(&layout, &state, &aliases);
        resolver.resolve(id).map_err(CommandError::Resolve)?.leaf
    };
    let composed = compose_context(&layout, &state, leaf)?;
    print!("{}", composed);
    Ok(())
}

fn materialise(
    pm_root_arg: Option<&Path>,
    id: &str,
    output: Option<&Path>,
) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;
    let leaf = {
        let resolver = Resolver::new(&layout, &state, &aliases);
        resolver.resolve(id).map_err(CommandError::Resolve)?.leaf
    };
    let composed = compose_context(&layout, &state, leaf)?;
    let path: PathBuf = match output {
        Some(p) => p.to_path_buf(),
        None => {
            let abs_dir = layout.root.join(state.lookup(leaf).expect("just resolved").path.clone());
            abs_dir.join(format!("{leaf}.composed.md"))
        }
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(CommandError::Io)?;
    }
    fs::write(&path, composed.as_bytes()).map_err(CommandError::Io)?;
    println!("Wrote {}", path.display());
    Ok(())
}

fn artifact(pm_root_arg: Option<&Path>, action: ArtifactAction) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;

    let resolve_id = |raw: &str| -> Result<(LeafId, PathBuf, String), CommandError> {
        let resolver = Resolver::new(&layout, &state, &aliases);
        let resolved = resolver.resolve(raw).map_err(CommandError::Resolve)?;
        let abs = layout.root.join(&resolved.relative_path);
        let slug = resolved.relative_path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok((resolved.leaf, abs, slug))
    };

    match action {
        ArtifactAction::Add { id, path, desc } => {
            let (leaf, abs, slug) = resolve_id(&id)?;
            let artifacts_dir = abs.join("artifacts");
            fs::create_dir_all(&artifacts_dir).map_err(CommandError::Io)?;
            let filename = path.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .ok_or_else(|| CommandError::Parse("source path has no file name".into()))?;
            fs::copy(&path, artifacts_dir.join(&filename)).map_err(CommandError::Io)?;
            sweep_dir(&artifacts_dir, leaf, Some(&slug)).map_err(CommandError::Artifact)?;
            if let Some(d) = desc {
                let index_path = artifacts_dir.join(ARTIFACTS_MD);
                let mut idx = ArtifactsIndex::load(&index_path).map_err(CommandError::Artifact)?;
                if let Some(entry) = idx.find_mut(&filename) {
                    entry.desc = d;
                }
                idx.save(&index_path, Some(&slug)).map_err(CommandError::Artifact)?;
            }
            println!("Added artifact {filename} to {leaf}");
        }
        ArtifactAction::Rename { id, old, new } => {
            let (_leaf, abs, slug) = resolve_id(&id)?;
            let artifacts_dir = abs.join("artifacts");
            rename_artifact(&artifacts_dir, &old, &new, Some(&slug))
                .map_err(CommandError::Artifact)?;
            println!("Renamed {old} -> {new}");
        }
        ArtifactAction::List { id } => {
            let (_leaf, abs, _slug) = resolve_id(&id)?;
            let artifacts_dir = abs.join("artifacts");
            let index_path = artifacts_dir.join(ARTIFACTS_MD);
            if !index_path.exists() {
                println!("(no artifacts)");
                return Ok(());
            }
            let idx = ArtifactsIndex::load(&index_path).map_err(CommandError::Artifact)?;
            for e in &idx.entries {
                let desc = if e.desc.is_empty() { String::from("-") } else { e.desc.clone() };
                let tags = if e.tags.is_empty() { String::from("[]") } else { format!("[{}]", e.tags.join(", ")) };
                println!("{}  {}  {}  {}", e.file, e.added, tags, desc);
            }
        }
    }
    Ok(())
}

fn template(pm_root_arg: Option<&Path>, action: TemplateAction) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    match action {
        TemplateAction::Edit { kind } => {
            let prefix: TypePrefix = kind.into();
            let dir = layout.root.join("templates");
            fs::create_dir_all(&dir).map_err(CommandError::Io)?;
            let stem = templates::template_stem(prefix);
            let path = dir.join(format!("{stem}.md"));
            if !path.exists() {
                fs::write(&path, templates::builtin(prefix)).map_err(CommandError::Io)?;
            }
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let status = Command::new(&editor).arg(&path).status().map_err(CommandError::Io)?;
            if !status.success() {
                return Err(CommandError::Parse(format!("{editor} exited with {status}")));
            }
            println!("Template saved to {}", path.display());
        }
        TemplateAction::Apply { id } => {
            let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
            let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;
            let leaf = {
                let resolver = Resolver::new(&layout, &state, &aliases);
                resolver.resolve(&id).map_err(CommandError::Resolve)?.leaf
            };
            let entry = state.lookup(leaf).cloned()
                .ok_or_else(|| CommandError::Parse(format!("{leaf} not in state.json")))?;
            let abs_dir = layout.root.join(&entry.path);
            let resolved = templates::resolve(leaf.prefix(), &layout.root, std::env::var_os("HOME").map(PathBuf::from).as_deref());
            let mut ticket = Ticket::read(&abs_dir.join(CLAUDE_MD)).map_err(CommandError::Ticket)?;
            ticket.apply_template(&resolved.content);
            ticket.write_to(&abs_dir).map_err(CommandError::Ticket)?;
            println!("Re-applied {} template to {}", templates::template_stem(leaf.prefix()), leaf);
        }
    }
    Ok(())
}

// ------------------------------------------------------------ search/doctor

fn search(pm_root_arg: Option<&Path>, query: &str) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let q = query.to_ascii_lowercase();
    let mut hits = 0usize;
    for (leaf, entry) in &state.items {
        let abs = layout.root.join(&entry.path).join(CLAUDE_MD);
        let Ok(raw) = fs::read_to_string(&abs) else { continue };
        if raw.to_ascii_lowercase().contains(&q) {
            let title = parse_title(&raw).unwrap_or_default();
            println!("{leaf}  {title}");
            hits += 1;
        }
    }
    if hits == 0 {
        println!("(no matches)");
    }
    Ok(())
}

fn doctor(pm_root_arg: Option<&Path>) -> Result<(), CommandError> {
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let original = State::load(&layout.state_path()).map_err(CommandError::State)?;

    let mut rebuilt = State {
        next: original.next.clone(),
        tombstones: original.tombstones.clone(),
        items: Default::default(),
    };

    let mut drift = 0usize;
    let mut visit = |path: PathBuf| -> io::Result<()> {
        let claude = path.join(CLAUDE_MD);
        if !claude.exists() { return Ok(()); }
        if let Ok(t) = Ticket::read(&claude) {
            let rel = path.strip_prefix(&layout.root).unwrap_or(&path).to_path_buf();
            let leaf = t.front_matter.id;
            if rebuilt.items.insert(leaf, ItemEntry { path: rel.clone() }).is_some() {
                println!("Duplicate id on disk: {leaf} at {}", rel.display());
                drift += 1;
            }
            // Bump counter if disk holds an id beyond the recorded counter.
            let next = rebuilt.next.entry(leaf.prefix()).or_insert(1);
            if leaf.number() >= *next { *next = leaf.number() + 1; }
        }
        Ok(())
    };

    walk(&layout.root, &mut visit).map_err(CommandError::Io)?;

    // Detect drift: items in original not in rebuilt, and vice versa.
    for (k, v) in &original.items {
        match rebuilt.items.get(k) {
            None => {
                println!("Missing on disk: {k} (state.json had {})", v.path.display());
                drift += 1;
            }
            Some(r) if r.path != v.path => {
                println!("Path drift: {k} state.json={} disk={}", v.path.display(), r.path.display());
                drift += 1;
            }
            _ => {}
        }
    }
    for k in rebuilt.items.keys() {
        if !original.items.contains_key(k) {
            println!("On disk but not in state.json: {k}");
            drift += 1;
        }
    }

    rebuilt.save(&layout.state_path()).map_err(CommandError::State)?;
    if drift == 0 {
        println!("state.json clean.");
    } else {
        println!("Reconciled {drift} drift entries; state.json rewritten.");
    }
    Ok(())
}

fn walk<F: FnMut(PathBuf) -> io::Result<()>>(root: &Path, f: &mut F) -> io::Result<()> {
    for dirent in fs::read_dir(root)? {
        let dirent = dirent?;
        let path = dirent.path();
        if path.is_dir() {
            f(path.clone())?;
            walk(&path, f)?;
        }
    }
    Ok(())
}

// ----------------------------------------------------------------- memory

fn memory(action: MemoryAction) -> Result<(), CommandError> {
    match action {
        MemoryAction::Link { id, name, scope } => {
            println!("memory link {id} {name} (scope: {:?}) deferred to Phase 10.", scope);
        }
        MemoryAction::Unlink { id, name } => {
            println!("memory unlink {id} {name} deferred to Phase 10.");
        }
        MemoryAction::List { id } => {
            println!("memory list {id} deferred to Phase 10.");
        }
        MemoryAction::Promote { name, to } => {
            println!("memory promote {name} --to {:?} deferred to Phase 10.", to);
        }
        MemoryAction::Write { name, body, scope } => {
            println!("memory write {name} (scope: {:?}): {body:?} deferred to Phase 10.", scope);
        }
    }
    let _ = MemoryRef::User(String::new()); // touch import so it isn't dead
    Ok(())
}

// -------------------------------------------------------------- workflow stubs

fn stub_workflow(verb: &str, id: &str, hint: Option<&str>) -> Result<(), CommandError> {
    let hint = hint.unwrap_or("(no message)");
    println!("{verb} {id}: {hint} - deferred to Phase 6 (locks + events).");
    Ok(())
}

fn stub_next(agent: Option<&str>, filter: Option<&str>) -> Result<(), CommandError> {
    println!("next agent={:?} filter={:?} deferred to Phase 6.", agent, filter);
    Ok(())
}

fn stub_simple(verb: &str, phase: &str) -> Result<(), CommandError> {
    println!("{verb} deferred to {phase}.");
    Ok(())
}

fn stub_log(id: &str) -> Result<(), CommandError> {
    println!("log {id} deferred to Phase 5 (git integration).");
    Ok(())
}

// ----------------------------------------------------------------- helpers

mod util {
    pub fn slugify(title: &str) -> String {
        let mut out = String::with_capacity(title.len());
        let mut last_hyphen = true;
        for c in title.chars() {
            let mapped = if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c.is_whitespace() || matches!(c, '-' | '_' | '/' | '\\' | '.') {
                Some('-')
            } else {
                None
            };
            if let Some(ch) = mapped {
                if ch == '-' {
                    if !last_hyphen { out.push('-'); last_hyphen = true; }
                } else {
                    out.push(ch); last_hyphen = false;
                }
            }
        }
        while out.ends_with('-') { out.pop(); }
        if out.is_empty() { return "untitled".to_string(); }
        if out.len() > 63 { out.truncate(63); }
        while out.ends_with('-') { out.pop(); }
        out
    }
}

fn load_ticket_path(pm_root_arg: Option<&Path>, id: &str)
    -> Result<(Layout, LeafId, PathBuf), CommandError>
{
    let layout = root::resolve(pm_root_arg).map_err(CommandError::Root)?;
    let state = State::load(&layout.state_path()).map_err(CommandError::State)?;
    let aliases = Aliases::load(&layout.aliases_path()).map_err(CommandError::State)?;
    let resolved = {
        let resolver = Resolver::new(&layout, &state, &aliases);
        resolver.resolve(id).map_err(CommandError::Resolve)?
    };
    let rel = layout.root.join(&resolved.relative_path);
    Ok((layout, resolved.leaf, rel))
}

fn load_abs_dir(pm_root_arg: Option<&Path>, id: &str)
    -> Result<(Layout, LeafId, PathBuf), CommandError>
{
    load_ticket_path(pm_root_arg, id)
}

fn mutate_front_matter<F>(abs_dir: &Path, f: F) -> Result<(), CommandError>
where
    F: FnOnce(&mut FrontMatter) -> Result<(), CommandError>,
{
    let claude = abs_dir.join(CLAUDE_MD);
    let mut ticket = Ticket::read(&claude).map_err(CommandError::Ticket)?;
    f(&mut ticket.front_matter)?;
    ticket.front_matter.updated = Utc::now();
    ticket.write_to(abs_dir).map_err(CommandError::Ticket)?;
    Ok(())
}

fn parse_status(value: &str) -> Result<Status, CommandError> {
    match value.replace('_', "-").to_ascii_lowercase().as_str() {
        "open" => Ok(Status::Open),
        "in-progress" | "in_progress" | "progress" => Ok(Status::InProgress),
        "done" | "complete" | "completed" => Ok(Status::Done),
        other => Err(CommandError::Parse(format!("unknown status {other:?}; use open/in-progress/done"))),
    }
}

fn parse_priority(value: &str) -> Result<Priority, CommandError> {
    match value.replace('_', "-").to_ascii_lowercase().as_str() {
        "must-have" | "high" => Ok(Priority::MustHave),
        "nice-to-have" | "medium" | "med" => Ok(Priority::NiceToHave),
        "cut-first" | "low" => Ok(Priority::CutFirst),
        other => Err(CommandError::Parse(format!("unknown priority {other:?}; use must-have/nice-to-have/cut-first"))),
    }
}

fn read_title(layout: &Layout, rel: &Path) -> Result<String, CommandError> {
    let claude = layout.root.join(rel).join(CLAUDE_MD);
    let raw = fs::read_to_string(&claude).map_err(CommandError::Io)?;
    Ok(parse_title(&raw).unwrap_or_default())
}

fn parse_title(raw: &str) -> Option<String> {
    let (yaml, _) = crate::store::split_front_matter(raw).ok()?;
    for line in yaml.lines() {
        if let Some(rest) = line.strip_prefix("title:") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn find_section_line(raw: &str, name: &str) -> Option<usize> {
    let (_, body) = crate::store::split_front_matter(raw).ok()?;
    let prefix_skip = raw.len() - body.len();
    let mut line_no = 1;
    for c in raw[..prefix_skip].chars() {
        if c == '\n' { line_no += 1; }
    }
    let target = format!("# {name}");
    for (idx, line) in body.lines().enumerate() {
        if line.trim() == target {
            return Some(line_no + idx);
        }
    }
    None
}

fn prompt(msg: &str) -> Result<String, CommandError> {
    print!("{msg}");
    io::stdout().flush().map_err(CommandError::Io)?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).map_err(CommandError::Io)?;
    Ok(buf)
}

fn compose_context(
    layout: &Layout,
    state: &State,
    leaf: LeafId,
) -> Result<String, CommandError> {
    let chain = walk_ancestors(state, leaf, &layout.root);
    let mut out = String::new();
    for leaf in chain {
        let entry = state.lookup(leaf).ok_or_else(||
            CommandError::Parse(format!("{leaf} not in state.json")))?;
        let abs = layout.root.join(&entry.path).join(CLAUDE_MD);
        let raw = fs::read_to_string(&abs).map_err(CommandError::Io)?;
        let (_, body) = crate::store::split_front_matter(&raw).map_err(|e|
            CommandError::Parse(format!("front-matter parse: {e}")))?;
        let parsed = ParsedBody::parse(strip_artifacts_trailer(body));
        let label = match leaf.prefix() {
            TypePrefix::Project => "PROJECT",
            TypePrefix::Product => "PRODUCT",
            TypePrefix::Epic => "EPIC",
            TypePrefix::Task => "TASK",
            TypePrefix::Subtask => "SUBTASK",
            TypePrefix::Milestone => "MILESTONE",
        };
        let slug = entry.path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !out.is_empty() {
            out.push_str("\n---\n\n");
        }
        out.push_str(&format!("# {} - {} ({})\n\n", label, slug, leaf));
        out.push_str(&parsed.render());
    }
    Ok(out)
}

fn walk_ancestors(state: &State, leaf: LeafId, _pm_root: &Path) -> Vec<LeafId> {
    let mut chain: Vec<LeafId> = Vec::new();
    let mut cursor: Option<LeafId> = Some(leaf);
    let mut guard = 0;
    while let Some(c) = cursor {
        if guard > 16 { break; }
        guard += 1;
        chain.push(c);
        // Walk by reading the ticket's parent field from disk.
        let Some(entry) = state.lookup(c) else { break };
        let claude = entry.path.join(CLAUDE_MD);
        let Ok(raw) = fs::read_to_string(&claude) else { break };
        cursor = parse_parent_id(&raw);
    }
    chain.reverse();
    chain
}

fn parse_parent_id(raw: &str) -> Option<LeafId> {
    let (yaml, _) = crate::store::split_front_matter(raw).ok()?;
    for line in yaml.lines() {
        if let Some(rest) = line.strip_prefix("parent:") {
            let val = rest.trim();
            if val.is_empty() || val == "null" { return None; }
            let cleaned = val.trim_matches('"').trim_matches('\'');
            let input: IdInput = cleaned.parse().ok()?;
            return Some(input.leaf());
        }
    }
    None
}

fn strip_artifacts_trailer(body: &str) -> &str {
    let needle = crate::store::ARTIFACTS_IMPORT;
    let trimmed = body.trim_end_matches('\n');
    trimmed.strip_suffix(needle).unwrap_or(body)
}

fn compute_project_id(state: &State, rel: &Path) -> Option<LeafId> {
    // The project leaf, if any, is the first segment of an address chain
    // beginning with `projects/<slug>`. Walk the relative path; the first
    // ticket whose recorded path starts with `projects/<slug>` wins.
    let components: Vec<_> = rel.components().collect();
    if components.first().map(|c| c.as_os_str().to_string_lossy().into_owned()).as_deref() != Some("projects") {
        return None;
    }
    let project_slug = components.get(1)?.as_os_str().to_string_lossy().into_owned();
    let project_path = Path::new("projects").join(&project_slug);
    for (k, v) in &state.items {
        if k.prefix() == TypePrefix::Project && v.path == project_path {
            return Some(*k);
        }
    }
    None
}

fn compute_address(state: &State, rel: &Path) -> Result<String, CommandError> {
    let mut chain: Vec<String> = Vec::new();
    let mut cumulative = PathBuf::new();
    for comp in rel.components() {
        cumulative.push(comp);
        for (leaf, entry) in &state.items {
            if entry.path == cumulative {
                chain.push(leaf.to_string());
                break;
            }
        }
    }
    if chain.is_empty() {
        return Err(CommandError::Parse("could not compute address".into()));
    }
    Ok(chain.join("-"))
}

// ------------------------------------------------------------ error type

#[derive(Debug)]
pub enum CommandError {
    Root(super::root::RootError),
    Layout(crate::store::LayoutError),
    State(crate::store::StateError),
    Ticket(TicketError),
    Artifact(ArtifactError),
    Resolve(crate::store::ResolveError),
    Io(io::Error),
    Parse(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Root(e) => write!(f, "{e}"),
            CommandError::Layout(e) => write!(f, "{e}"),
            CommandError::State(e) => write!(f, "{e}"),
            CommandError::Ticket(e) => write!(f, "{e}"),
            CommandError::Artifact(e) => write!(f, "{e}"),
            CommandError::Resolve(e) => write!(f, "{e}"),
            CommandError::Io(e) => write!(f, "{e}"),
            CommandError::Parse(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for CommandError {}
