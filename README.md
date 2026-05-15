# pm - Local-first project management for humans and agents

`pm` is a single-binary project manager that runs in your terminal, talks to AI coding agents through MCP, and keeps every ticket, artifact, and memory as plain files on disk. Hand-edit in your editor of choice, version with `git`, and pick up where you left off in another machine or another decade.

> **v1.0.0** ships with a redesigned data model, an MCP server, a three-tier memory system, and a refreshed TUI. The legacy v0.9.x JSON store is gone; the new format is documented in [docs/file-layout.md](docs/file-layout.md). Migration tooling is part of `pm doctor --migrate`.

[![CI](https://github.com/pbower/project_management/actions/workflows/ci.yml/badge.svg)](https://github.com/pbower/project_management/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/project_management.svg)](https://crates.io/crates/project_management)

## The 90-second story

```bash
# Install (Rust toolchain required)
cargo install project_management

# Set up the workspace in your repo (creates .pm/)
pm init

# Make a project, a task under it, and a subtask under that
pm add --kind project "PM tool"               # -> PRJ1
pm add --kind task "Lock protocol" --parent PRJ1   # -> TSK1
pm add --kind subtask "TTL heartbeat" --parent TSK1 # -> SBT1

# Drop a reference file under the task and watch ARTIFACTS.md update
echo '...' > .pm/projects/PRJ1/tasks/TSK1/artifacts/schema.png
pm artifact list TSK1

# Open the TUI for visual triage
pm ui

# Or start the MCP server so an agent can drive the same workspace
pm mcp
```

Every ticket is its own directory with a `CLAUDE.md` describing it. Agents read those files via the cwd cascade or via the MCP server; humans edit them in `$EDITOR`. The two ways stay in sync because they're the same files on disk.

> *Demo GIF coming - until then, the example commands above are the complete getting-started sequence.*

## Why it's different

| | Web-based PM tools | Other CLI PMs | `pm` |
|---|---|---|---|
| Runs offline | rarely | usually | always |
| Stores tickets as files you can `grep` and `git diff` | no | sometimes | yes (one CLAUDE.md per ticket) |
| Agent-ready out of the box | no | rarely | yes (MCP server, three-tier memory) |
| Visual TUI for triage | no | rarely | yes |
| Artifacts attached per-ticket | rarely | rarely | yes (per-ticket `artifacts/` folder + auto-curated index) |
| Cross-platform single binary | n/a | yes | yes |

## What you get

- **Hierarchical model**: `Project -> Product -> Epic -> Task -> Subtask`, plus `Milestone` as a cross-cutting tag. Each level is its own ticket type with its own id prefix (`PRJ1`, `PRD2`, `EPC3`, `TSK4`, `SBT5`, `MLS6`). Leaf ids are monotonic per type and never reused.
- **CLAUDE.md per ticket**: front-matter for the structured metadata (status, priority, due, tags, deps, links, memories) and a markdown body with named sections (Description, Acceptance Criteria, Notes, ...). Each kind has a default body template; templates are user-overrideable.
- **TUI**: three modes - ticket browser, activity feed, and a redesigned workflow board. Vim-style keybindings.
- **MCP server**: `pm mcp` opens a stdio JSON-RPC server with 14 tools (list, get, read_context, read_artifact, read_memories, write_doc, write_memory, checkout, checkin, complete, next, add, link, events). Under 4.5 k tokens of tool descriptions; spec'd to fit a single agent context window.
- **Three-tier memory**: user-scope, project-scope, ticket-scope. Front-matter `memories:` references resolve across tiers. PM does not write the user tier directly; that stays under Claude Code's auto-memory control.
- **Per-ticket artifacts**: drop files in `artifacts/`, `ARTIFACTS.md` regenerates with file-name and timestamp entries. Hand-edited `desc:` and `tags:` fields survive sweeps. Rename via `pm artifact rename` preserves metadata.
- **Activity feed**: every state-changing action appends one line to `.pm/events.log`. Tail it from another terminal with `pm tv`.
- **Locks**: optional checkouts (`pm checkout TSK7`) so a busy agent can claim work without stepping on the human. Soft mode warns; hard mode rejects.

## Installation

```bash
cargo install project_management
```

Prebuilt binaries for Linux, macOS, and Windows are attached to each [GitHub release](https://github.com/pbower/project_management/releases).

## Quick reference

```bash
# Workspace
pm init                            # initialise .pm/ in the current directory
pm doctor                          # rebuild state.json from disk truth
pm doctor --migrate                # migrate a legacy v0.9.x ~/.pm/tasks.json

# Tickets
pm add --kind task "Title" --parent EPC3
pm list --kind task --status open
pm view TSK7                       # inline view of front-matter + body
pm complete TSK7
pm delete TSK7                     # tombstones the id; no reuse

# Context, artifacts, memory
pm context TSK7                    # composed CLAUDE.md chain to TSK7
pm artifact list TSK7
pm artifact rename TSK7 old.png new.png
pm memory list TSK7
pm memory write --scope project --type feedback --name <name> "..."
pm memory promote <name>           # user <-> project promotion

# Templates
pm template list                   # TaskTemplate presets
pm template edit task              # section template for the Task kind

# Workflow
pm checkout TSK7 --intent "..."
pm checkin TSK7 --summary "..."
pm next --agent claude-be          # ready-for-work pick

# UI and feeds
pm ui                              # TUI
pm tv                              # tail .pm/events.log
pm mcp                             # JSON-RPC server on stdio

# Help
pm help
pm help <verb>
```

## Hierarchy and identity

Six ticket types. The first letter you write is which one:

| Prefix | Type | Lives under |
|---|---|---|
| `PRJ` | Project | `.pm/projects/<PRJ>/` |
| `PRD` | Product | `<project>/products/<PRD>/` |
| `EPC` | Epic | `<product>/epics/<EPC>/` |
| `TSK` | Task | `<epic>/tasks/<TSK>/` |
| `SBT` | Subtask | `<task>/subtasks/<SBT>/` |
| `MLS` | Milestone | `<project>/milestones/<MLS>/` (or top-level for cross-project) |

A ticket's leaf id (e.g. `TSK7`) is durable for its lifetime. Address forms like `PRJ1-PRD1-EPC3-TSK7` derive from the parent chain on disk and regenerate when a ticket moves. Stale address-form references continue to resolve through `aliases.json`.

See [docs/file-layout.md](docs/file-layout.md) for the on-disk model in full.

## Editor integration

`pm edit TSK7` opens the ticket's CLAUDE.md in `$EDITOR`. The default sections (Description, Acceptance Criteria, Notes, etc.) come from the per-kind template; agents can recall them via the cwd cascade or the `read_context` MCP tool. The cascade picks up every `CLAUDE.md` on the path from the ticket directory up to the project root.

Section editing is targeted: `pm write_doc TSK7 --section Description "..."` replaces one named section while leaving every other section and the front-matter untouched.

See [docs/templates.md](docs/templates.md) for template details and customisation.

## Agents (MCP)

```bash
pm mcp
```

Starts an MCP server on stdio. Add this to your agent's MCP config:

```json
{
  "mcpServers": {
    "pm": {
      "command": "pm",
      "args": ["mcp"]
    }
  }
}
```

The 14-tool surface is documented in [docs/mcp.md](docs/mcp.md). Notable bits:

- Tools are mutate-or-read; mutating tools save atomically and emit a feed event.
- `complete` can be gated behind a human-approval step via `.pm/config.toml`.
- The user-scope memory tier is read-only from PM. Agents cannot write there through MCP; the schema reflects that.

## Memory

Three tiers cover the spectrum from "what I personally remember" to "what this ticket needs":

- **User**: Claude Code's auto-memory store. PM reads; PM writes only through promote demotions.
- **Project**: per-project, committed with the project.
- **Ticket**: per-ticket, committed with the ticket.

`pm memory link TSK7 feedback-testing` records a typed reference in TSK7's front-matter. The composed view annotates each reference with its tier tag so agents see which tier provided each note.

Full semantics in [docs/memory.md](docs/memory.md).

## Migration from v0.9.x

The on-disk format has changed completely. The good news:

```bash
pm doctor --migrate
```

reads your legacy `~/.pm/tasks.json` (or per-project JSON files) and writes the v1 `.pm/` tree alongside. Original files are preserved under `.legacy-backup/`; no data is lost. Once you're satisfied, you can delete the backup yourself.

If you're starting fresh, just `pm init` in a new repo.

## Configuration

`.pm/config.toml` (optional) carries workspace-level toggles. Currently recognised keys:

```toml
# Gate the MCP `complete` tool behind explicit human approval.
require_complete_approval = true
```

## Storage and portability

Everything PM writes lives under one workspace-local `.pm/` directory. Drop the workspace into a git repo and version it with your code. Multiple agents can drive the same workspace concurrently; per-ticket locks and the activity feed keep activity visible.

PM stores paths in `state.json` using the local OS's separators. If you rsync a `.pm/` tree from one OS to another and the index ends up with mixed separators, run `pm doctor` on the target OS to rebuild a clean index. The on-disk tree itself is portable.

## Project structure

```
pm/
├── src/                     # crate source
├── tests/                   # phase acceptance tests
├── examples/                # scaffolds demonstrating the building blocks
├── docs/                    # reference docs for each major surface
├── .github/workflows/       # CI matrix (Linux/macOS/Windows)
├── README.md                # you are here
├── CHANGELOG.md             # release notes
└── Cargo.toml
```

## Contributing

PRs welcome. CI runs `cargo fmt --check`, `cargo clippy` (advisory at v1.0), `cargo build`, and `cargo test --all` on all three operating systems. A v1.0 stable release lives on `main`; phase branches carry larger reworks.

## License

MPL-2.0. See [LICENSE](LICENSE).

## See also

- [docs/file-layout.md](docs/file-layout.md) - the on-disk model.
- [docs/mcp.md](docs/mcp.md) - the JSON-RPC tool catalogue.
- [docs/memory.md](docs/memory.md) - the three-tier memory model.
- [docs/templates.md](docs/templates.md) - per-kind section templates and TaskTemplate presets.
