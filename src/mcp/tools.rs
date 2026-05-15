//! Tool catalog with JSON Schemas.
//!
//! Each tool the MCP server exposes appears here once: a stable name, a
//! short description, and the JSON Schema describing its input arguments.
//! The catalog is built once and shared between the `tools/list` response
//! and the `tools/call` dispatcher.

use serde_json::{json, Value};

/// One tool definition surfaced over MCP.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

impl ToolDef {
    /// Render as the JSON shape MCP's `tools/list` expects.
    pub fn to_listing(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}

/// The 14 tools the Phase 11 server exposes. Returned in a stable order so
/// agents that cache the listing see the same shape each session.
pub fn tool_catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list",
            description: "Scoped query over the workspace's tickets. Returns ids and titles only; use `get` or `read_context` for detail.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": {"type": "string", "description": "Filter by status: open, in-progress, done"},
                    "kind": {"type": "string", "description": "Filter by kind: project, product, epic, task, subtask, milestone"},
                    "parent": {"type": "string", "description": "Filter to children of this ticket id"},
                    "tag": {"type": "string", "description": "Filter to tickets carrying this tag"},
                    "limit": {"type": "integer", "minimum": 1, "description": "Cap on rows returned"}
                },
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "get",
            description: "Single ticket's front-matter and child count. Does not return CLAUDE.md body; use `read_context` for that.",
            input_schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string", "description": "Ticket id (e.g. TSK7, PRJ1-PRD1-EPC3-TSK7)"}},
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "read_context",
            description: "Composed CLAUDE.md chain from the root ancestor down to the target ticket. Appends a `Linked memories` section by default; pass `no_memories: true` to suppress.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "no_memories": {"type": "boolean", "default": false}
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "read_artifact",
            description: "Fetch one artifact file's content from a ticket's artifacts/ directory.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "filename": {"type": "string"}
                },
                "required": ["id", "filename"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "read_memories",
            description: "Linked memories for a ticket, each resolved across the user / project / ticket tiers and returned with its tier tag.",
            input_schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "write_doc",
            description: "Replace a CLAUDE.md section's body. Front-matter is preserved; other sections untouched.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "section": {"type": "string", "description": "Section heading (e.g. Description, Requirements)"},
                    "content": {"type": "string"}
                },
                "required": ["id", "section", "content"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "write_memory",
            description: "Write a memory file at the project or ticket tier. The user tier is read-only from PM and is rejected here.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["project", "ticket"]},
                    "type": {"type": "string", "enum": ["user", "feedback", "project", "reference"]},
                    "name": {"type": "string"},
                    "content": {"type": "string"},
                    "description": {"type": "string"},
                    "ticket": {"type": "string", "description": "Required when scope=ticket"},
                    "project": {"type": "string", "description": "PRJ leaf; required when scope=project and the workspace has more than one project"}
                },
                "required": ["scope", "type", "name", "content"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "checkout",
            description: "Acquire an advisory lock on a ticket. Soft mode warns on overlap; hard mode rejects.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "intent": {"type": "string", "description": "One-line description of why this checkout exists"},
                    "mode": {"type": "string", "enum": ["soft", "hard"], "default": "soft"},
                    "ttl_seconds": {"type": "integer", "minimum": 60}
                },
                "required": ["id", "intent"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "checkin",
            description: "Release a previously-acquired lock on a ticket. Emits a checkin event with the supplied summary.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "summary": {"type": "string"}
                },
                "required": ["id", "summary"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "complete",
            description: "Mark a ticket complete. May be gated by a user-approval setting in .pm/config.toml.",
            input_schema: json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "next",
            description: "Pick the next ticket ready for work for the given agent. Skips tickets with unmet dependencies or active locks held by other agents.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent": {"type": "string", "description": "Agent identifier; matches PM_AGENT_ID in lock files"},
                    "kind": {"type": "string", "description": "Optional filter by ticket kind"},
                    "tag": {"type": "string", "description": "Optional filter by tag"}
                },
                "required": ["agent"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "add",
            description: "Create a new ticket. Returns the new leaf id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "kind": {"type": "string", "enum": ["project", "product", "epic", "task", "subtask", "milestone"]},
                    "parent": {"type": "string", "description": "Optional parent ticket id"}
                },
                "required": ["title", "kind"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "link",
            description: "Add a dependency edge: the source ticket waits on the target ticket. Type is `needs` by default.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Ticket that gains the dependency"},
                    "dep_id": {"type": "string", "description": "Ticket the source depends on"},
                    "type": {"type": "string", "default": "needs"}
                },
                "required": ["id", "dep_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "events",
            description: "Tail the activity feed. `since` is either an ISO-8601 timestamp or a positive integer for the last N events.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "since": {"type": "string", "description": "ISO timestamp or `N` for the last N events"},
                    "limit": {"type": "integer", "minimum": 1, "default": 50}
                },
                "additionalProperties": false
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_exactly_fourteen_tools() {
        assert_eq!(tool_catalog().len(), 14);
    }

    #[test]
    fn catalog_tool_names_are_unique() {
        let names: Vec<&str> = tool_catalog().iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate tool name");
    }

    #[test]
    fn each_tool_has_an_object_schema() {
        for tool in tool_catalog() {
            assert_eq!(
                tool.input_schema["type"].as_str(),
                Some("object"),
                "tool {} has non-object inputSchema",
                tool.name,
            );
        }
    }

    #[test]
    fn write_memory_excludes_user_scope() {
        let tool = tool_catalog()
            .into_iter()
            .find(|t| t.name == "write_memory")
            .unwrap();
        let enum_values = &tool.input_schema["properties"]["scope"]["enum"];
        let scopes: Vec<&str> = enum_values
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(scopes, vec!["project", "ticket"]);
    }

    #[test]
    fn to_listing_matches_mcp_shape() {
        let tool = &tool_catalog()[0];
        let v = tool.to_listing();
        assert_eq!(v["name"].as_str(), Some(tool.name));
        assert_eq!(v["description"].as_str(), Some(tool.description));
        assert!(v["inputSchema"].is_object());
    }

    /// PM_BUILD_PLAN.md Phase 11 exit criterion: the full tool-set fits in
    /// roughly 4.5k tokens when serialised for `tools/list`. The estimate
    /// is `chars / 4`, a widely-used token rule of thumb for English-and-JSON
    /// payloads. The check is conservative on purpose - the real tokeniser
    /// would treat short JSON punctuation as a single token, which our
    /// chars/4 rule overestimates.
    #[test]
    fn tools_list_payload_fits_under_45k_tokens() {
        let listings: Vec<_> = tool_catalog().into_iter().map(|t| t.to_listing()).collect();
        let payload = serde_json::json!({"tools": listings});
        let serialised = serde_json::to_string(&payload).expect("serialise listings");
        let est_tokens = (serialised.chars().count() + 3) / 4;
        assert!(
            est_tokens <= 4500,
            "tools/list payload uses ~{est_tokens} tokens (cap is 4500). Trim descriptions or schemas.",
        );
    }
}
