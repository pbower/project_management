//! Tool handlers. Each function in this module backs one tool listed in
//! [`super::tools::tool_catalog`]. Handlers reload the [`Database`] on every
//! call, mutate as needed, and save through the regular atomic write path so
//! two agents calling at the same time cannot tear `state.json`. Lost-update
//! races on the load-mutate-save cycle are not addressed in Phase 11; the
//! exit criterion in PM_BUILD_PLAN.md only commits to no torn writes.
//!
//! Each handler returns a `serde_json::Value` payload. The server wraps that
//! in MCP's `{content: [{type:"text", text}], isError}` envelope.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::db::Database;
use crate::fields::{Kind, Status};
use crate::memory::store::MemoryContext;
use crate::memory::{lookup_by_name, write_memory, MemoryHit, MemoryType, Scope};
use crate::store::claude_md::Ticket;
use crate::store::events::{actor as default_actor, append_event, read_events, Event};
use crate::store::front_matter::MemoryRef;
use crate::store::id::{IdInput, LeafId};
use crate::store::locks::{self, AcquireOutcome, LockFile, LockMode, DEFAULT_TTL_SECONDS};
use crate::task::Task;

/// Per-call working context. Re-loaded on each tool call so a long-running
/// MCP server picks up disk-side changes made by other writers.
pub struct Context {
    pub pm_dir: PathBuf,
    pub db: Database,
    pub home: PathBuf,
    pub cwd: PathBuf,
    /// Launch scope inherited from `THUNDER_SCOPE`, if the MCP server is
    /// running inside a `spacecell agent --window` terminal. `None`
    /// means the server is not scoped and every operation is allowed
    /// without a warning event.
    pub scope: Option<LeafId>,
}

impl Context {
    /// Build a context against `pm_dir`. Loads `Database` once; callers
    /// invoke [`Context::reload`] before each tool call.
    pub fn for_pm_dir(pm_dir: PathBuf) -> Self {
        let db = Database::load(&pm_dir);
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let scope = crate::store::events::scope_from_env();
        Context {
            pm_dir,
            db,
            home,
            cwd,
            scope,
        }
    }

    pub fn reload(&mut self) {
        self.db = Database::load(&self.pm_dir);
    }

    /// Soft-enforce subtree authority: when the MCP server is scoped
    /// to `THUNDER_SCOPE` and the operation's target is outside that
    /// subtree, emit a `warning` event with detail mentioning the
    /// violation. Returns regardless so the operation proceeds; the
    /// audit trail is the enforcement mechanism, not a refusal.
    pub fn record_scope_violation_if_any(&self, verb: &str, target: LeafId) {
        let Some(scope) = self.scope else {
            return;
        };
        if is_descendant_of(&self.db, target, scope) {
            return;
        }
        let detail = format!("out-of-scope: {verb} {target} from scope {scope}");
        let _ = append_event(
            &self.pm_dir,
            &Event {
                ts: Utc::now(),
                actor: default_actor(),
                verb: "warning".to_string(),
                id: Some(target),
                detail: Some(detail),
                scope: Some(scope),
            },
        );
    }
}

/// `true` if `id` is `ancestor` itself or any descendant via parent
/// links. Linear walk capped at 16 levels to guard against pathological
/// data.
fn is_descendant_of(db: &Database, id: LeafId, ancestor: LeafId) -> bool {
    let mut cursor = Some(id);
    let mut guard = 0;
    while let Some(c) = cursor {
        if c == ancestor {
            return true;
        }
        if guard > 16 {
            return false;
        }
        guard += 1;
        cursor = db.get(c).and_then(|t| t.parent);
    }
    false
}

/// Dispatch one tool call. Returns the structured payload for the response;
/// the server wraps it in the MCP envelope.
pub fn dispatch(ctx: &mut Context, tool: &str, args: &Value) -> Result<Value, String> {
    ctx.reload();
    match tool {
        "list" => handle_list(ctx, args),
        "get" => handle_get(ctx, args),
        "read_context" => handle_read_context(ctx, args),
        "read_artifact" => handle_read_artifact(ctx, args),
        "read_memories" => handle_read_memories(ctx, args),
        "write_doc" => handle_write_doc(ctx, args),
        "write_memory" => handle_write_memory(ctx, args),
        "checkout" => handle_checkout(ctx, args),
        "checkin" => handle_checkin(ctx, args),
        "complete" => handle_complete(ctx, args),
        "next" => handle_next(ctx, args),
        "add" => handle_add(ctx, args),
        "link" => handle_link(ctx, args),
        "events" => handle_events(ctx, args),
        _ => Err(format!("unknown tool: {tool}")),
    }
}

// ----- list ---------------------------------------------------------------

fn handle_list(ctx: &Context, args: &Value) -> Result<Value, String> {
    let status = args
        .get("status")
        .and_then(Value::as_str)
        .map(parse_status)
        .transpose()?;
    let kind = args
        .get("kind")
        .and_then(Value::as_str)
        .map(parse_kind)
        .transpose()?;
    let parent_arg = args.get("parent").and_then(Value::as_str);
    let parent = parent_arg
        .map(|s| resolve_leaf(&ctx.db, s).ok_or_else(|| format!("parent not found: {s}")))
        .transpose()?;
    let tag = args.get("tag").and_then(Value::as_str).map(str::to_string);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX) as usize;

    let mut rows: Vec<Value> = Vec::new();
    for task in ctx.db.tasks.iter() {
        if let Some(s) = status {
            if task.status != s {
                continue;
            }
        }
        if let Some(k) = kind {
            if task.kind != k {
                continue;
            }
        }
        if let Some(p) = parent {
            if task.parent != Some(p) {
                continue;
            }
        }
        if let Some(t) = &tag {
            if !task.tags.iter().any(|x| x == t) {
                continue;
            }
        }
        rows.push(json!({
            "id": task.id.to_string(),
            "title": task.title,
            "kind": kind_label(task.kind),
            "status": status_label(task.status),
        }));
        if rows.len() >= limit {
            break;
        }
    }
    Ok(json!({"tickets": rows}))
}

// ----- get ----------------------------------------------------------------

fn handle_get(ctx: &Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    let task = ctx.db.get(leaf).ok_or_else(|| "missing task".to_string())?;
    let children = ctx
        .db
        .tasks
        .iter()
        .filter(|t| t.parent == Some(leaf))
        .count();
    Ok(json!({
        "id": task.id.to_string(),
        "title": task.title,
        "kind": kind_label(task.kind),
        "status": status_label(task.status),
        "parent": task.parent.map(|p| p.to_string()),
        "tags": task.tags,
        "due": task.due.map(|d| d.format("%Y-%m-%d").to_string()),
        "children": children,
    }))
}

// ----- read_context -------------------------------------------------------

fn handle_read_context(ctx: &Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    let no_memories = args
        .get("no_memories")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let chain = walk_ancestor_chain(&ctx.db, leaf);
    let mut out = String::new();
    for cid in &chain {
        let Some(entry) = ctx.db.state.items.get(cid) else {
            continue;
        };
        let path = ctx
            .pm_dir
            .join(&entry.path)
            .join(crate::store::claude_md::CLAUDE_MD);
        let Ok(ticket) = Ticket::read(&path) else {
            continue;
        };
        let task = ctx.db.get(*cid).expect("walk returned only known leaves");
        out.push_str(&format!(
            "# {} - {} ({cid})\n",
            kind_label_upper(task.kind),
            task.title
        ));
        for section in &ticket.body.sections {
            out.push_str("\n# ");
            out.push_str(&section.name);
            out.push('\n');
            out.push_str(&section.body);
        }
        out.push_str("\n---\n\n");
    }

    // Per-level artifact summary blocks; mirrors `pm context` output.
    out.push_str(&render_artifact_blocks(&ctx.db, &ctx.pm_dir, &chain));

    if !no_memories {
        if let Some(entry) = ctx.db.state.items.get(&leaf) {
            let path = ctx
                .pm_dir
                .join(&entry.path)
                .join(crate::store::claude_md::CLAUDE_MD);
            if let Ok(ticket) = Ticket::read(&path) {
                if !ticket.front_matter.memories.is_empty() {
                    out.push_str(&render_linked_memories(ctx, leaf, &ticket));
                }
            }
        }
    }

    // Trailing `@`-import line so a Claude Code loader picks up the leaf
    // ticket's ARTIFACTS.md as a generic reference. Cheap and harmless
    // when no `artifacts/` exists.
    out.push_str("@artifacts/ARTIFACTS.md\n");

    Ok(json!({"context": out}))
}

/// Per-level "## Artifacts at <LEVEL> (<id>)" blocks for the chain. Skips
/// levels with no on-disk `artifacts/ARTIFACTS.md` or with an empty entry
/// list. Output mirrors `pm context` so the MCP `read_context` view stays
/// byte-equivalent.
fn render_artifact_blocks(db: &Database, pm_dir: &Path, chain: &[LeafId]) -> String {
    use crate::store::artifacts::{ArtifactsIndex, ARTIFACTS_MD};
    let mut out = String::new();
    for cid in chain {
        let Some(entry) = db.state.items.get(cid) else {
            continue;
        };
        let index_path = pm_dir
            .join(&entry.path)
            .join("artifacts")
            .join(ARTIFACTS_MD);
        if !index_path.is_file() {
            continue;
        }
        let Ok(idx) = ArtifactsIndex::load(&index_path) else {
            continue;
        };
        if idx.entries.is_empty() {
            continue;
        }
        let Some(task) = db.get(*cid) else {
            continue;
        };
        out.push_str(&format!(
            "## Artifacts at {} ({cid})\n",
            kind_label_upper(task.kind)
        ));
        for entry in &idx.entries {
            if entry.desc.is_empty() {
                out.push_str(&format!("- {}\n", entry.file));
            } else {
                out.push_str(&format!("- {} - {}\n", entry.file, entry.desc));
            }
        }
        out.push('\n');
    }
    out
}

// ----- read_artifact ------------------------------------------------------

fn handle_read_artifact(ctx: &Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let filename = require_str(args, "filename")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    let entry = ctx
        .db
        .state
        .items
        .get(&leaf)
        .ok_or_else(|| format!("state.json has no entry for {id}"))?;
    let path = ctx
        .pm_dir
        .join(&entry.path)
        .join("artifacts")
        .join(filename);
    if !path.is_file() {
        return Err(format!("artifact not found: {filename}"));
    }
    let bytes = fs::read(&path).map_err(|e| format!("read: {e}"))?;
    let text_content = String::from_utf8(bytes.clone()).ok();
    Ok(json!({
        "path": path.display().to_string(),
        "size": bytes.len(),
        "is_text": text_content.is_some(),
        "content": text_content,
    }))
}

// ----- read_memories ------------------------------------------------------

fn handle_read_memories(ctx: &Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    let entry = ctx
        .db
        .state
        .items
        .get(&leaf)
        .ok_or_else(|| format!("state.json has no entry for {id}"))?;
    let ticket = Ticket::read(
        &ctx.pm_dir
            .join(&entry.path)
            .join(crate::store::claude_md::CLAUDE_MD),
    )
    .map_err(|e| format!("read ticket: {e}"))?;
    let mctx = memory_context(ctx, Some(leaf));

    let mut out: Vec<Value> = Vec::new();
    for memref in &ticket.front_matter.memories {
        let name = memref_name(memref);
        let tier = memref_tier(memref);
        match lookup_by_name(&mctx, name) {
            Ok(Some(MemoryHit { location, file })) => {
                out.push(json!({
                    "name": file.front_matter.name,
                    "tier": tier,
                    "resolved_at": location.scope.as_str(),
                    "description": file.front_matter.description,
                    "body": file.body,
                    "path": location.file.display().to_string(),
                }));
            }
            Ok(None) => {
                out.push(json!({"name": name, "tier": tier, "missing": true}));
            }
            Err(e) => {
                out.push(json!({"name": name, "tier": tier, "error": e.to_string()}));
            }
        }
    }
    Ok(json!({"memories": out}))
}

// ----- write_doc ----------------------------------------------------------

fn handle_write_doc(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let section = require_str(args, "section")?;
    let content = require_str(args, "content")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    ctx.record_scope_violation_if_any("write_doc", leaf);
    let entry = ctx
        .db
        .state
        .items
        .get(&leaf)
        .ok_or_else(|| format!("state.json has no entry for {id}"))?
        .clone();
    let ticket_dir = ctx.pm_dir.join(&entry.path);
    let claude_path = ticket_dir.join(crate::store::claude_md::CLAUDE_MD);
    let mut ticket = Ticket::read(&claude_path).map_err(|e| format!("read ticket: {e}"))?;
    let replaced = ticket.upsert_section(section, content.to_string());
    ticket
        .write_to(&ticket_dir)
        .map_err(|e| format!("write ticket: {e}"))?;

    let _ = append_event(
        &ctx.pm_dir,
        &Event {
            ts: Utc::now(),
            actor: default_actor(),
            verb: "edit".to_string(),
            id: Some(leaf),
            detail: Some(format!("write_doc section={section}")),
            scope: crate::store::events::scope_from_env(),
        },
    );

    Ok(json!({
        "id": leaf.to_string(),
        "section": section,
        "replaced_existing": replaced,
        "path": claude_path.display().to_string(),
    }))
}

// ----- write_memory -------------------------------------------------------

fn handle_write_memory(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let scope_str = require_str(args, "scope")?;
    let scope = Scope::parse(scope_str).ok_or_else(|| format!("unknown scope: {scope_str}"))?;
    if matches!(scope, Scope::User) {
        return Err("user-tier writes are not exposed via MCP; use promotion".into());
    }
    let kind_str = require_str(args, "type")?;
    let kind =
        MemoryType::parse(kind_str).ok_or_else(|| format!("unknown memory type: {kind_str}"))?;
    let name = require_str(args, "name")?;
    let content = require_str(args, "content")?;
    let description = args
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);

    let ticket_arg = args.get("ticket").and_then(Value::as_str);
    let project_arg = args.get("project").and_then(Value::as_str);
    let mctx = memory_context_with_overrides(ctx, ticket_arg, project_arg)?;

    let loc = write_memory(&mctx, scope, name, kind, description, content)
        .map_err(|e| format!("write_memory: {e}"))?;

    let _ = append_event(
        &ctx.pm_dir,
        &Event {
            ts: Utc::now(),
            actor: default_actor(),
            verb: "memory-write".to_string(),
            id: None,
            detail: Some(format!("{}:{name}", scope.as_str())),
            scope: crate::store::events::scope_from_env(),
        },
    );
    Ok(json!({
        "path": loc.file.display().to_string(),
        "scope": scope.as_str(),
    }))
}

// ----- checkout -----------------------------------------------------------

fn handle_checkout(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let intent = require_str(args, "intent")?;
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("soft");
    let mode = match mode {
        "soft" => LockMode::Soft,
        "hard" => LockMode::Hard,
        other => return Err(format!("unknown mode: {other}")),
    };
    let ttl = args
        .get("ttl_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TTL_SECONDS);
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    ctx.record_scope_violation_if_any("checkout", leaf);
    let now = Utc::now();
    let lock = LockFile::new(leaf, Some(intent.to_string()), ttl, mode, None);
    match locks::acquire(&ctx.pm_dir, &lock, now) {
        Ok(AcquireOutcome::Acquired) => Ok(json!({"id": leaf.to_string(), "outcome": "acquired"})),
        Ok(AcquireOutcome::Overlapped { previous }) => Ok(json!({
            "id": leaf.to_string(),
            "outcome": "soft-overlap",
            "previous_agent": previous.agent,
        })),
        Ok(AcquireOutcome::Blocked { holder }) => {
            Err(format!("lock refused: held by {}", holder.agent,))
        }
        Err(e) => Err(format!("checkout: {e}")),
    }
}

// ----- checkin ------------------------------------------------------------

fn handle_checkin(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let summary = require_str(args, "summary")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    ctx.record_scope_violation_if_any("checkin", leaf);
    let removed = locks::release(&ctx.pm_dir, leaf).map_err(|e| format!("checkin: {e}"))?;
    let _ = append_event(
        &ctx.pm_dir,
        &Event {
            ts: Utc::now(),
            actor: default_actor(),
            verb: "checkin".to_string(),
            id: Some(leaf),
            detail: Some(summary.to_string()),
            scope: crate::store::events::scope_from_env(),
        },
    );
    Ok(json!({"id": leaf.to_string(), "released": removed}))
}

// ----- complete -----------------------------------------------------------

fn handle_complete(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    if approval_required(&ctx.pm_dir) {
        return Err(
            "complete: workspace config requires user approval; mark complete via the TUI or CLI"
                .into(),
        );
    }
    let id = require_str(args, "id")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    ctx.record_scope_violation_if_any("complete", leaf);
    {
        let task = ctx
            .db
            .get_mut(leaf)
            .ok_or_else(|| "missing task".to_string())?;
        task.status = Status::Done;
        task.updated_at_utc = Utc::now().timestamp();
    }
    ctx.db.save(&ctx.pm_dir).map_err(|e| format!("save: {e}"))?;
    let _ = append_event(
        &ctx.pm_dir,
        &Event {
            ts: Utc::now(),
            actor: default_actor(),
            verb: "complete".to_string(),
            id: Some(leaf),
            detail: None,
            scope: crate::store::events::scope_from_env(),
        },
    );
    Ok(json!({"id": leaf.to_string(), "status": "done"}))
}

// ----- next ---------------------------------------------------------------

fn handle_next(ctx: &Context, args: &Value) -> Result<Value, String> {
    let agent = require_str(args, "agent")?;
    let kind_filter = args
        .get("kind")
        .and_then(Value::as_str)
        .map(parse_kind)
        .transpose()?;
    let tag_filter = args.get("tag").and_then(Value::as_str).map(str::to_string);

    let active_locks: std::collections::HashSet<LeafId> = locks::list(&ctx.pm_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.agent != agent)
        .map(|l| l.id)
        .collect();

    let mut candidates: Vec<&Task> = ctx
        .db
        .tasks
        .iter()
        .filter(|t| t.status == Status::Open)
        .filter(|t| !active_locks.contains(&t.id))
        .filter(|t| kind_filter.map_or(true, |k| t.kind == k))
        .filter(|t| {
            tag_filter
                .as_ref()
                .map_or(true, |tag| t.tags.iter().any(|x| x == tag))
        })
        .collect();
    candidates.sort_by_key(|t| t.id.number());

    for task in candidates {
        if task
            .deps
            .iter()
            .all(|dep| matches!(ctx.db.get(*dep).map(|d| d.status), Some(Status::Done)))
        {
            return Ok(json!({
                "id": task.id.to_string(),
                "title": task.title,
                "kind": kind_label(task.kind),
                "status": status_label(task.status),
            }));
        }
    }
    Ok(json!({"id": null, "title": null}))
}

// ----- add ----------------------------------------------------------------

fn handle_add(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let title = require_str(args, "title")?;
    let kind_str = require_str(args, "kind")?;
    let kind = parse_kind(kind_str)?;
    let parent = match args.get("parent").and_then(Value::as_str) {
        Some(s) => Some(resolve_leaf(&ctx.db, s).ok_or_else(|| format!("parent not found: {s}"))?),
        None => None,
    };
    if let Some(parent_leaf) = parent {
        let parent_task = ctx
            .db
            .get(parent_leaf)
            .ok_or_else(|| "parent missing".to_string())?;
        if !crate::db::validate_hierarchy(parent_task.kind, kind) {
            return Err(format!(
                "{} cannot be a child of {}",
                kind_label(kind),
                kind_label(parent_task.kind),
            ));
        }
    }
    let id = ctx
        .db
        .allocate_id(crate::store::migrate::kind_to_prefix(kind));
    let now = Utc::now().timestamp();
    let task = Task {
        id,
        title: title.to_string(),
        summary: None,
        description: None,
        user_story: None,
        requirements: None,
        tags: Vec::new(),
        deps: Vec::new(),
        milestone: None,
        memories: Vec::new(),
        due: None,
        parent,
        kind,
        status: Status::Open,
        priority_level: None,
        urgency: None,
        process_stage: None,
        issue_link: None,
        pr_link: None,
        artifacts: Vec::new(),
        created_at_utc: now,
        updated_at_utc: now,
    };
    ctx.db.tasks.push(task);
    ctx.db.save(&ctx.pm_dir).map_err(|e| format!("save: {e}"))?;
    let _ = append_event(
        &ctx.pm_dir,
        &Event {
            ts: Utc::now(),
            actor: default_actor(),
            verb: "add".to_string(),
            id: Some(id),
            detail: Some(title.to_string()),
            scope: crate::store::events::scope_from_env(),
        },
    );
    Ok(json!({"id": id.to_string()}))
}

// ----- link ---------------------------------------------------------------

fn handle_link(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let id = require_str(args, "id")?;
    let dep_id = require_str(args, "dep_id")?;
    let leaf = resolve_leaf(&ctx.db, id).ok_or_else(|| format!("not found: {id}"))?;
    let dep_leaf = resolve_leaf(&ctx.db, dep_id).ok_or_else(|| format!("not found: {dep_id}"))?;
    ctx.record_scope_violation_if_any("link", leaf);
    if leaf == dep_leaf {
        return Err("a ticket cannot depend on itself".into());
    }
    {
        let task = ctx
            .db
            .get_mut(leaf)
            .ok_or_else(|| "missing task".to_string())?;
        if !task.deps.contains(&dep_leaf) {
            task.deps.push(dep_leaf);
            task.updated_at_utc = Utc::now().timestamp();
        }
    }
    ctx.db.save(&ctx.pm_dir).map_err(|e| format!("save: {e}"))?;
    let _ = append_event(
        &ctx.pm_dir,
        &Event {
            ts: Utc::now(),
            actor: default_actor(),
            verb: "link".to_string(),
            id: Some(leaf),
            detail: Some(format!("needs {dep_leaf}")),
            scope: crate::store::events::scope_from_env(),
        },
    );
    Ok(json!({"id": leaf.to_string(), "dep_id": dep_leaf.to_string()}))
}

// ----- events -------------------------------------------------------------

fn handle_events(ctx: &Context, args: &Value) -> Result<Value, String> {
    let since = args.get("since").and_then(Value::as_str);
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;

    let all = read_events(&ctx.pm_dir).map_err(|e| format!("read events: {e}"))?;
    let filtered: Vec<&Event> = match since {
        None => all.iter().collect(),
        Some(s) => {
            if let Ok(ts) = s.parse::<DateTime<Utc>>() {
                all.iter().filter(|e| e.ts > ts).collect()
            } else if let Ok(n) = s.parse::<usize>() {
                let start = all.len().saturating_sub(n);
                all.iter().skip(start).collect()
            } else {
                return Err("since must be an ISO-8601 timestamp or a positive integer".into());
            }
        }
    };
    let payload: Vec<Value> = filtered
        .into_iter()
        .rev()
        .take(limit)
        .rev()
        .map(|e| {
            json!({
                "ts": e.ts.to_rfc3339(),
                "actor": e.actor,
                "verb": e.verb,
                "id": e.id.map(|i| i.to_string()),
                "detail": e.detail,
            })
        })
        .collect();
    Ok(json!({"events": payload}))
}

// ----- helpers ------------------------------------------------------------

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, String> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string field: {field}"))
}

fn parse_status(s: &str) -> Result<Status, String> {
    match s.to_lowercase().as_str() {
        "open" => Ok(Status::Open),
        "in-progress" | "in_progress" => Ok(Status::InProgress),
        "done" => Ok(Status::Done),
        _ => Err(format!("unknown status: {s}")),
    }
}

fn parse_kind(s: &str) -> Result<Kind, String> {
    match s.to_lowercase().as_str() {
        "project" => Ok(Kind::Project),
        "product" => Ok(Kind::Product),
        "epic" => Ok(Kind::Epic),
        "task" => Ok(Kind::Task),
        "subtask" => Ok(Kind::Subtask),
        "milestone" => Ok(Kind::Milestone),
        _ => Err(format!("unknown kind: {s}")),
    }
}

fn status_label(s: Status) -> &'static str {
    match s {
        Status::Open => "open",
        Status::InProgress => "in-progress",
        Status::Done => "done",
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Project => "project",
        Kind::Product => "product",
        Kind::Epic => "epic",
        Kind::Task => "task",
        Kind::Subtask => "subtask",
        Kind::Milestone => "milestone",
    }
}

fn kind_label_upper(k: Kind) -> &'static str {
    match k {
        Kind::Project => "PROJECT",
        Kind::Product => "PRODUCT",
        Kind::Epic => "EPIC",
        Kind::Task => "TASK",
        Kind::Subtask => "SUBTASK",
        Kind::Milestone => "MILESTONE",
    }
}

fn resolve_leaf(db: &Database, input: &str) -> Option<LeafId> {
    let parsed: IdInput = input.parse().ok()?;
    let leaf = parsed.leaf();
    if db.get(leaf).is_some() {
        Some(leaf)
    } else {
        None
    }
}

fn walk_ancestor_chain(db: &Database, start: LeafId) -> Vec<LeafId> {
    let mut chain = Vec::new();
    let mut cursor = Some(start);
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

fn memref_name(m: &MemoryRef) -> &str {
    match m {
        MemoryRef::User(s) | MemoryRef::Project(s) | MemoryRef::Ticket(s) => s.as_str(),
    }
}

fn memref_tier(m: &MemoryRef) -> &'static str {
    match m {
        MemoryRef::User(_) => "user",
        MemoryRef::Project(_) => "project",
        MemoryRef::Ticket(_) => "ticket",
    }
}

fn memory_context(ctx: &Context, ticket: Option<LeafId>) -> MemoryContext {
    let active_ticket_dir = ticket.and_then(|leaf| {
        ctx.db
            .state
            .items
            .get(&leaf)
            .map(|e| ctx.pm_dir.join(&e.path))
    });
    let active_project = ticket.and_then(|leaf| project_ancestor(&ctx.db, leaf));
    MemoryContext {
        home: ctx.home.clone(),
        cwd: ctx.cwd.clone(),
        pm_root: ctx.pm_dir.clone(),
        active_project,
        active_ticket_dir,
    }
}

fn memory_context_with_overrides(
    ctx: &Context,
    ticket_arg: Option<&str>,
    project_arg: Option<&str>,
) -> Result<MemoryContext, String> {
    let ticket_leaf = ticket_arg
        .map(|s| resolve_leaf(&ctx.db, s).ok_or_else(|| format!("ticket not found: {s}")))
        .transpose()?;
    let mut mctx = memory_context(ctx, ticket_leaf);
    if let Some(p) = project_arg {
        let leaf = resolve_leaf(&ctx.db, p).ok_or_else(|| format!("project not found: {p}"))?;
        if !matches!(ctx.db.get(leaf).map(|t| t.kind), Some(Kind::Project)) {
            return Err(format!("--project must be a PRJ leaf; got {p}"));
        }
        mctx.active_project = Some(leaf);
    } else if mctx.active_project.is_none() {
        // Fall back to the workspace's only project if there is exactly one.
        mctx.active_project = solo_project(&ctx.db);
    }
    Ok(mctx)
}

fn project_ancestor(db: &Database, leaf: LeafId) -> Option<LeafId> {
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(id) = cursor {
        if guard > 16 {
            break;
        }
        guard += 1;
        let task = db.get(id)?;
        if matches!(task.kind, Kind::Project) {
            return Some(task.id);
        }
        cursor = task.parent;
    }
    None
}

fn solo_project(db: &Database) -> Option<LeafId> {
    let mut found: Option<LeafId> = None;
    for task in db.tasks.iter() {
        if matches!(task.kind, Kind::Project) {
            if found.is_some() {
                return None;
            }
            found = Some(task.id);
        }
    }
    found
}

fn render_linked_memories(ctx: &Context, leaf: LeafId, ticket: &Ticket) -> String {
    let mctx = memory_context(ctx, Some(leaf));
    let mut out = String::new();
    out.push_str(&format!("## Linked memories ({leaf})\n\n"));
    for memref in &ticket.front_matter.memories {
        let name = memref_name(memref);
        let tier = memref_tier(memref);
        out.push_str(&format!("### {name} [{tier}]\n"));
        match lookup_by_name(&mctx, name) {
            Ok(Some(hit)) => {
                if let Some(desc) = &hit.file.front_matter.description {
                    out.push_str(&format!("\n> {desc}\n"));
                }
                out.push_str(&format!("\n{}\n", hit.file.body.trim_end()));
                out.push_str(&format!("\n@{}\n", hit.location.file.display()));
            }
            Ok(None) => out.push_str("\n(missing from disk)\n"),
            Err(e) => out.push_str(&format!("\n(resolution error: {e})\n")),
        }
        out.push_str("\n---\n\n");
    }
    out
}

fn approval_required(pm_dir: &std::path::Path) -> bool {
    // Phase 11 config-toml gate. The setting is a single boolean under
    // `[mcp] require_complete_approval = true`. Absence of the file or the
    // key means the gate is off.
    let path = pm_dir.join("config.toml");
    if !path.is_file() {
        return false;
    }
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    raw.lines().any(|l| {
        let t = l.trim();
        t.starts_with("require_complete_approval") && t.contains("true")
    })
}
