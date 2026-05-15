# File Layout

Every PM workspace lives under a single `.pm/` directory. The shape on disk is the canonical source of truth; `state.json` is a derived index that `pm doctor` can rebuild at any time.

## Top-level

```
.pm/
├── state.json          # id index, counters, alias targets, templates
├── aliases.json        # address-form redirect entries for moved tickets
├── events.log          # JSONL activity feed (one event per line, append-only)
├── locks/              # active checkouts; <leaf>.lock files
├── projects/           # PRJ tickets and their subtrees
├── products/           # orphan products (no project parent)
├── epics/              # orphan epics
├── tasks/              # orphan tasks
├── subtasks/           # orphan subtasks
└── milestones/         # cross-project milestones
```

`projects/`, `products/`, `epics/`, `tasks/`, `subtasks/`, `milestones/` are the six type-folder roots. Their plural form is the only place the type appears in the path - the directory below them is named after the ticket's `LeafId` (e.g. `tasks/TSK7/`), not a title-derived label.

## Per-ticket layout

A complete five-level chain on disk:

```
.pm/
└── projects/PRJ1/
    ├── CLAUDE.md
    ├── artifacts/
    │   ├── ARTIFACTS.md
    │   └── ...
    ├── milestones/MLS1/
    │   ├── CLAUDE.md
    │   └── artifacts/
    └── products/PRD1/
        ├── CLAUDE.md
        ├── artifacts/
        └── epics/EPC3/
            ├── CLAUDE.md
            ├── artifacts/
            └── tasks/TSK7/
                ├── CLAUDE.md
                ├── memories/      # ticket-tier memory files
                ├── artifacts/
                │   ├── ARTIFACTS.md
                │   ├── schema.png
                │   └── bench.csv
                └── subtasks/SBT2/
                    ├── CLAUDE.md
                    └── artifacts/
```

Each ticket directory contains:

- `CLAUDE.md` - authored YAML front-matter plus markdown body. The single source of truth for the ticket's metadata and prose. See [templates.md](templates.md) for the body shape.
- `artifacts/` - drop-folder for related files. Auto-swept; the resulting `artifacts/ARTIFACTS.md` is a hand-curatable index. Hand-edits to `desc:`/`tags:` fields survive sweeps.
- `memories/` (optional) - ticket-tier memory files (see [memory.md](memory.md)).
- Type-named subfolders for the next level down, when present.

## `state.json`

`state.json` is the id index. Its shape:

```json
{
  "next":       { "PRJ": 2, "PRD": 4, "EPC": 18, "TSK": 263, "SBT": 91, "MLS": 7 },
  "tombstones": { "PRJ": [], "PRD": [], "EPC": [], "TSK": [42], "SBT": [], "MLS": [] },
  "items": {
    "PRJ1":  { "path": "projects/PRJ1/" },
    "PRD1":  { "path": "projects/PRJ1/products/PRD1/" },
    "TSK7":  { "path": "projects/PRJ1/products/PRD1/epics/EPC3/tasks/TSK7/" },
    "TSK15": { "path": "tasks/TSK15/" },
    "MLS1":  { "path": "projects/PRJ1/milestones/MLS1/" }
  },
  "templates": [ ... ]
}
```

- `next` carries one monotonic counter per type. New ids are allocated as `<prefix><next>`, then `next` advances.
- `tombstones` records numbers that have been allocated and deleted. They are never reused.
- `items` maps each live leaf id to its directory under `.pm/`.
- `templates` carries the `TaskTemplate` presets used by the TUI quick-entry flow (see [templates.md](templates.md)).

`state.json` writes go through a temp-file + atomic rename. `pm doctor` walks the on-disk tree and rebuilds `state.json` from observed reality; use it if anything ever drifts.

## `aliases.json`

When a ticket moves under a different parent chain, the address form of its id changes (the leaf id never does). Aliases let stale address-form references keep resolving:

```json
{
  "PRJ1-PRD1-EPC3-TSK7":      "PRJ1-PRD1-EPC5-TSK7",
  "PRJ1-PRD1-EPC3-TSK7-SBT2": "PRJ1-PRD1-EPC5-TSK7-SBT2"
}
```

Leaf-form lookups never need aliases. The resolver checks aliases only when an address-form lookup misses the live index.

## `events.log`

Append-only JSONL of every state-changing action. One line per event:

```jsonl
{"ts":"2026-05-12T11:33:00Z","actor":"claude-be","verb":"checkout","id":"TSK7","intent":"implement TTL"}
{"ts":"2026-05-12T11:38:14Z","actor":"claude-be","verb":"edit","id":"TSK7","summary":"added heartbeat field"}
{"ts":"2026-05-12T11:52:08Z","actor":"claude-be","verb":"checkin","id":"TSK7","summary":"tests passing"}
```

Writes use `OpenOptions::append`, which gives atomic appends for short lines on both POSIX and Windows.

## Locks

`locks/` holds per-ticket JSON files representing active checkouts:

```
locks/TSK7.lock
locks/TSK44.lock
```

Each file carries the actor, intent, heartbeat timestamp, TTL, and mode (soft or hard). PM uses file-level conventions rather than OS-level file locking for portability and to make ownership visible on disk. Stale locks past their TTL are released on the next `pm doctor` or `pm locks` invocation.

## Cross-platform notes

- `PathBuf` in `state.json.items[].path` serialises with the local OS's separators. If you sync a `.pm/` tree across operating systems and your `state.json` ends up with mixed-separator paths, run `pm doctor` on the target OS to rebuild a clean index. The on-disk tree itself is portable.
- Line endings in `CLAUDE.md` round-trip cleanly under either `\n` or `\r\n`; the front-matter parser tolerates both.
- The lock-file format is plain JSON, so a Windows agent and a Linux agent can read each other's locks if they share a workspace via a network mount.

## See also

- [mcp.md](mcp.md) - JSON-RPC tool catalogue served over stdio.
- [memory.md](memory.md) - three-tier memory model.
- [templates.md](templates.md) - per-kind section templates and customisation.
