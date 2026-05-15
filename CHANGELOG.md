# Changelog

All notable changes to this project will be documented here. Format roughly follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html) from v1.0.0 onward.

## [1.0.0] - 2026-05-15

The v1.0 release marks the new on-disk model and the agent-ready surfaces. The legacy v0.9.x storage is replaced; migration is automatic via `pm doctor --migrate`.

### Added

- Five-level hierarchy: `Project -> Product -> Epic -> Task -> Subtask`, plus `Milestone` as a cross-cutting tag. Each level has its own typed-prefix leaf id (PRJ/PRD/EPC/TSK/SBT/MLS) with a monotonic per-type counter.
- One CLAUDE.md per ticket carrying YAML front-matter and named markdown sections. Per-kind section templates with workspace and user-level overrides.
- `pm init` scaffolds a `.pm/` workspace in any directory.
- `pm doctor` rebuilds `state.json` from the on-disk tree. `pm doctor --migrate` reads a legacy v0.9.x JSON store and writes the new layout alongside it, preserving the originals under `.legacy-backup/`.
- Per-ticket `artifacts/` directories with an auto-curated `ARTIFACTS.md`. Hand-edited `desc:` and `tags:` survive sweeps. `pm artifact rename` preserves metadata.
- Three-tier memory system (user / project / ticket) with `pm memory link/list/write/promote/show/unlink`. The user tier is read-only from PM. Front-matter `memories:` carries typed references; the composed view annotates each with its tier tag.
- TUI with three modes - ticket browser, activity feed, redesigned workflow board - sharing a single layer over `Database`. Mode switching via Tab, 1, 2, 3.
- Workflow board (`pm wf`) updated for the v1 model.
- Per-ticket section editing (`pm edit --section <name>`) targets a named CLAUDE.md section in `$EDITOR` while leaving the rest untouched.
- Composed context view (`pm context <id>`) walks the parent chain and emits a single CLAUDE.md-shaped blob. Toggleable inclusion of a `Linked memories` section.
- Soft-lock checkouts (`pm checkout`, `pm checkin`, `pm heartbeat`, `pm locks`) so agents can claim work without races.
- Activity feed at `.pm/events.log` (append-only JSONL). Tail it from a second terminal with `pm tv`.
- Atomic git commits per state change (`pm log <id>` filters history to a ticket's slice of the tree).
- MCP server (`pm mcp`): stdio JSON-RPC 2.0 with 14 tools. Under 4.5 k tokens of tool descriptions. See `docs/mcp.md`.
- `docs/file-layout.md`, `docs/mcp.md`, `docs/memory.md`, `docs/templates.md`.
- Cross-platform CI matrix on `ubuntu-latest`, `macos-latest`, and `windows-latest`.
- `Cargo.toml` polished for crates.io publish: SPDX `MPL-2.0` license, refreshed description, `homepage`, `documentation` fields.

### Changed

- Storage layout is now a directory tree under `.pm/` instead of a flat `~/.pm/tasks.json`. See [docs/file-layout.md](docs/file-layout.md).
- `Task.id` is a typed `LeafId` rather than a bare `u64`. Front-matter and disk paths use the typed form (`TSK7`, `PRJ1-PRD1-EPC3-TSK7`, etc.).
- `Database` is loaded from and saved to the v2 tree via the Task <-> Document bridge in `src/store/task_bridge.rs`. The on-disk JSON format from v0.9.x is no longer supported directly; migrate with `pm doctor --migrate`.

### Removed

- The flat `tasks.json` storage format (migrate via `pm doctor --migrate`).
- The `pm tv` cwd-fallback to the legacy directory; pass an explicit path if you want to monitor a different workspace.

### Deferred to v1.1+

- GSD coexistence. The GSD spec and template shape have moved since this build began; an audit-and-import will land in a follow-on release.

### Known limitations

- `state.json` carries paths in the local OS's separator form. Sharing a `.pm/` tree across operating systems works via the on-disk tree; rebuild `state.json` with `pm doctor` on the target OS if the index ends up with mixed separators.
- Lost-update races on overlapping MCP load-mutate-save cycles are possible. The atomic save guarantees no torn reads. Workspace-wide write locking is a v1.1 candidate.

## [0.9.3] - 2025-08-17

- Workflow Ticket Manager added.

## [0.9.0] - [0.9.2] - 2025-08-15

- Initial public releases of the v0.9 line.
