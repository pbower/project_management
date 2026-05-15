# MCP Server

PM exposes a Model Context Protocol server over stdio. Run it with:

```bash
pm mcp
```

The server speaks JSON-RPC 2.0. One line in, one line out. Notifications (requests without an `id`) produce no response. Errors follow the standard JSON-RPC error envelope.

## Wire surface

PM implements the two MCP methods needed for tool use:

- `initialize` - returns the server's name, version, and protocol version.
- `tools/list` - returns the catalogue: 14 entries with `name`, `description`, and JSON Schema `inputSchema`.
- `tools/call` - invokes a tool. The response wraps the handler payload in MCP's `{content: [{type: "text", text}], isError}` envelope.

### Token budget

The full catalogue serialises to under 4.5 k tokens. A unit test (`tools_list_payload_fits_under_45k_tokens`) pins this so adding a verbose description fails CI rather than silently inflating the agent's context.

## Tool catalogue

The catalogue is fixed at 14 entries. Order is stable.

| Tool | Mutates | Purpose |
|---|---|---|
| `list` | no | Scoped query over the workspace. Returns ids and titles only; use `get` or `read_context` for detail. |
| `get` | no | Single ticket's front-matter and child count. |
| `read_context` | no | Composed CLAUDE.md chain from the root ancestor down to the target. Appends a `Linked memories` section by default; pass `no_memories: true` to suppress. |
| `read_artifact` | no | Fetch one artifact file's content from a ticket's `artifacts/` directory. Returns `is_text: false` for binary blobs. |
| `read_memories` | no | Linked memories for a ticket, resolved across the three tiers, each annotated with its tier tag. |
| `write_doc` | yes | Replace one CLAUDE.md section's body. Front-matter and other sections are untouched. Emits an `edit` event. |
| `write_memory` | yes | Write a memory file at the project or ticket tier. User-tier writes are rejected here (PM never writes the user tier directly; see [memory.md](memory.md)). |
| `checkout` | yes | Acquire an advisory lock on a ticket. Soft mode warns on overlap; hard mode rejects. |
| `checkin` | yes | Release a previously-acquired lock. Emits a `checkin` event with the supplied summary. |
| `complete` | yes | Mark a ticket complete. May be gated by a user-approval setting (see Approval gate below). |
| `next` | no | Pick the next ticket ready for work for the given agent. Skips unmet dependencies and locks held by other agents. |
| `add` | yes | Create a ticket. Returns the new leaf id. |
| `link` | yes | Add a dependency edge. `type` is `needs` by default. |
| `events` | no | Tail the activity feed. `since` is either an ISO-8601 timestamp or a positive integer for "last N events". |

## Approval gate for `complete`

If `.pm/config.toml` contains a line `require_complete_approval = true` (under any section, or at the top), `complete` returns a `requires_approval` payload instead of marking the ticket done. Callers re-call with `approved: true` once the human has confirmed. Absence of the file or key disables the gate.

## Database reload and concurrency

Each tool call reloads `Database` from disk before serving. Long-running servers pick up writes by other agents without needing to be restarted.

Side-effecting handlers save via the regular atomic-write path and emit one event per mutation. Concurrent calls do not produce torn reads. Lost-update races on overlapping `load -> mutate -> save` cycles are a known limitation; they require workspace-wide write locking, which is out of scope for v1.0.

## Example session

```jsonrpc
{"jsonrpc":"2.0","id":1,"method":"initialize"}
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"pm","version":"1.0.0"}}}

{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"list", ... }]}}

{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list","arguments":{"status":"open","limit":5}}}
{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"[{\"id\":\"TSK7\",\"title\":\"Lock protocol\"},...]"}],"isError":false}}
```

The tool result is a JSON string nested inside the MCP `content` envelope. Callers parse the inner JSON to get the structured payload.

## Reference

- See [file-layout.md](file-layout.md) for the underlying on-disk model the tools operate on.
- See [memory.md](memory.md) for memory tier semantics surfaced by `read_memories` and `write_memory`.
