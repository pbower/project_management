# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.3] - 2026-05-18

The full-screen ticket editor. Pressing Enter on a board card opens a
new editor that owns the screen until the user saves or cancels;
short fields are typed in directly, enum fields cycle with Left/Right,
and the long-form section bodies are still edited externally in
`$EDITOR`. The user never sees the YAML front-matter or the other
sections while editing one; on save, the body splices back into
`CLAUDE.md` at the same anchor.

### Added

- `src/tui/ticket_editor/`: new module split into `mod.rs` (the
  full-screen editor), `form.rs` (13-field form model, enum cycling,
  text-input typing mode, diff-against-task), and `sections.rs`
  (anchor-based section parsing, body extraction, atomic splice
  back, temp-file lifecycle).
- 13 form fields: title, summary, kind, status, priority, urgency,
  process, tags, due, parent, milestone, issue_link, pr_link. Text
  fields type straight in. Enum fields cycle with Left/Right.
- Section list shows every level-1 heading present in CLAUDE.md plus
  a line count. Enter on a section opens it externally in nvim with
  only that section's body in the temp file - no YAML, no other
  sections, no `@artifacts` import line. On save, splice back to the
  same anchor; everything else in CLAUDE.md is untouched.
- Save & Return and Cancel buttons at the bottom of the form. Ctrl+S
  also saves from anywhere; Esc cancels.
- New `Disposition` variants `OpenTicketEditor(LeafId)` and
  `EditRaw(LeafId)`. The shell routes Enter from a board card to the
  editor, `e` from a board card to raw `$EDITOR` on the whole
  CLAUDE.md (escape hatch for power users).
- Section splice tests: parse round-trip, body extraction with
  trailing-blank trim, splice replaces only the named section body,
  add-section inserts before the `@artifacts` import.
- Form tests: enum cycling in both directions, priority cycle
  includes None, typing appends and backspaces.

### Changed

- `cmd_edit` default `$EDITOR` fallback changed from `nano` to
  `nvim`. `EDITOR=...` still wins; the fallback is now nvim.
- Shell hosts `Option<TicketEditor>` and renders it full-screen when
  open. While the editor owns the screen, all keys route to it; the
  shell's mode-switch keys are inert until the editor closes.
- Save path applies each dirty form field through a direct task
  mutation + `db.save()` + `commit_workspace()` + `append_event()`
  cycle, so every change in the editor produces a git commit and an
  events.log entry. Future agents reading the event log see what the
  user changed and when.

### Robustness

- Ticket re-read from disk on editor open; missing fields default to
  empty; missing sections do not appear in the list.
- Section parse handles missing front-matter, missing `@artifacts`
  import, unclosed front-matter (treats as no body), and trailing
  blank-line normalisation across round-trips.
- Splice writes are atomic (temp file + rename). Tests pin the
  byte-equivalence of unmodified sections after a splice.
- Database reload after every save / cancel / external section edit
  so the form and the board reflect the latest disk state.

### Deferred

- Markdown rendering inside the editor (preview pane). `tui-markdown`
  is the obvious crate; deferring until the editor settles.
- Add Section flow (prompts the user for a heading and inserts it).
  v0.3.3 ships an "Add Section" row that surfaces a status note
  pointing at v0.3.4; the splice helper that backs it
  (`sections::add_section`) is already wired and tested.
- Artifact add / rename UI: still CLI-only via `spacecell artifact
  add`. v0.3.4 (launcher work) brings the in-form affordance.

## [0.3.2] - 2026-05-16

Lightning palette refresh and arrow-key-only navigation. The cockpit
canvas drops navy for true black; cyan-blue is reserved for the bolt
glyph on the wordmark; eyebrows render in paper bold and ids in muted
grey so the palette reads as restrained instead of saturated. Gold is
retained for active rows, focused selections, the THUNDER half of the
wordmark, and the focused-pane border accent.

Arrow keys handle navigation everywhere. `H/L/J/K` and `[/]` are gone;
Left from the leftmost board column hands focus back to the LHP, and
Right from the deepest LHP level (Subtask) hands focus into the board,
so a single cursor flows across the whole cockpit.

Note: the program name stays SpaceCell Thunder. The Lightning palette
is borrowed from the sibling SpaceCell Lightning product because the
black + cyan + gold combination reads better in a terminal than the
navy + gold did.

### Added

- `LIGHTNING_BLUE`, `LIGHTNING_BLUE_BRIGHT`, `LIGHTNING_BLUE_DEEP`
  palette constants in `src/style/mod.rs`.
- `wordmark_accent()` and `bolt()` style helpers so the wordmark renders
  in three coordinated colours (bolt in Lightning blue, "SPACECELL" in
  paper white, "THUNDER" in gold).
- `SURFACE_RAISED` / `SURFACE_RAISED_2` neutrals for nested-pane fills
  when a panel needs to read as slightly raised above the canvas.

### Changed

- Canvas (`body()`, all backgrounds) now `BLACK` instead of
  `NAVY_DEEP`. Every existing helper composes the new black canvas
  automatically.
- `eyebrow()` is paper bold (clean white section labels); cyan-blue is
  held back for the brand bolt only.
- `id_code()` is muted grey so ticket ids and timestamps read as data
  rather than competing with the active-row gold for attention.
- `border()` is muted grey for unfocused panels. `border_focused()`
  is the new gold accent for the pane that owns input; the user
  always knows where the cursor is.
- Semantic green / crimson tokens brightened so they pop on the new
  black canvas without losing their meaning.
- `src/tui/shell/header.rs` renders the wordmark as three styled spans
  to mirror the SpaceCell Lightning logo treatment.
- `src/tui/shell/help.rs` and `src/tui/lhp/mod.rs` align with the new
  wordmark treatment.

### Navigation

- Arrow keys flow seamlessly across the LHP and the Workbench. The
  `H/L/J/K` and `[/]` bindings from v0.3.1 are gone.
- LHP key handler returns a new `Disposition` enum. `OverflowRight`
  from the deepest level (Subtask) tells the shell to flip focus to
  the Workbench; `OverflowLeft` at Project is a no-op until v0.3.4
  adds a left-of-LHP surface.
- Board key handler does the symmetric thing: `OverflowLeft` from the
  leftmost column flips focus back to the LHP.
- Footer hints update with focus so the visible bindings always match
  what the cursor will actually do.

### Renumbered

- The next live-refresh phase moves from v0.3.2 to v0.3.3 to make room
  for this palette refresh. PM_BUILD_PLAN.md will renumber subsequent
  phases when v0.3.3 lands.

## [0.3.1] - 2026-05-16

The main shell composition. Wraps the v0.3.0 kept-pieces (workflow
board, hierarchy navigation, activity feed) in the LHP + Workbench +
Activity layout described in PM_DESIGN section 8.3. `spacecell` invoked
without a subcommand now opens the new shell; the standalone kanban at
`spacecell wf` stays for users who prefer it.

### Added

- `src/tui/shell/`: main shell with header (brand wordmark + scope
  breadcrumb + mode badge), three-zone body layout, activity strip,
  and context-sensitive footer.
- `src/tui/lhp/`: left-hand panel rendering five hierarchy levels
  (`PROJECTS`/`PRODUCTS`/`EPICS`/`TASKS`/`SUBTASKS`) plus a totals
  block. Up/Down moves the cursor within the focused level, Left/Right
  shifts focus between levels, Enter drills.
- `src/tui/workbench/`: `Surface` enum + `WorkbenchState`. Board
  surface is populated (9-stage kanban filtered to the LHP scope, H/L
  column nav, J/K card nav, read-only for v0.3.1). Documents and
  Activity surfaces render placeholder cards.
- `src/tui/activity/`: 3-line `events.log` tail strip. Reuses
  `views::events_view::ActivityView` for buffer parsing. Polls every
  500ms; v0.3.2 swaps the poll for a `notify` watcher.
- `src/tui/input/`: mode router. Tab/Shift-Tab cycle modes, 1/2/3
  jump, `[`/`]` switch focus between LHP and Workbench, `?`/F1 toggle
  help overlay, `q`/Ctrl-C quit.
- `src/tui/shell/help.rs`: modal help overlay reachable from every
  mode. v0.3.1 ships a single-page reference; v0.3.x phases extend it.
- Subcommand made optional in `cli.rs`: `spacecell` with no args opens
  the shell. Every existing CLI verb continues to work.
- `Commands` and the per-action enums under it now derive `Clone` so
  the main dispatcher can pass the subcommand through both the
  database-loaded and database-free branches.

### Changed

- `Cargo.toml` version bumped to 0.3.1.

### Tests

- `tui::input::tests` covers mode cycling, quit-key recognition, help
  overlay swallowing, focus-bracket routing, and key forwarding to
  the focused zone.
- `tui::workbench::board::tests` covers stage-index mapping and the
  truncation helper.
- 270 tests pass (210 lib + 60 integration; +7 from new modules).

## [0.3.0] - 2026-05-16

The v0.3.0 demolition phase. Strips the v0.9 TUI substrate down to the
pieces worth keeping (workflow kanban board, hierarchy navigation
primitives, `$EDITOR` handoff, activity-feed view) and rebuilds the rest
on the SpaceCell Thunder visual language across subsequent v0.3.x
releases.

### Added

- New `src/style/` module: the SpaceCell palette as ratatui `Style` and
  `Color` constants, plus a glyph table (`style::glyphs`). Single source
  of truth for the brand tokens from PM_DESIGN section 8.7.
- New `src/tui/nav/` module: free-standing hierarchy traversal
  primitives (`Level::child` / `Level::parent` / `Level::label_upper`,
  `ancestor_chain`, `tickets_at`). Salvaged from the deleted
  `app/navigation.rs`, decoupled from the v0.9 `App` state.

### Changed

- Workflow kanban board (`src/tui/workflow.rs`) renderer now resolves
  colour tokens through `src/style/`. The board's structure (9-stage
  columns, drill-down, card movement) is unchanged.
- `cmd_wf`: when the user picks a card to edit, the board exits and the
  ticket's `CLAUDE.md` opens via `$EDITOR` (through `cmd_edit`) instead
  of the deleted edit-form TUI. Reopens the board on the next
  invocation.
- Library crate identifier renamed: `project_management` is retired and
  the library now publishes as `spacecell_thunder`. Internal `use`
  statements were not affected by this change because no v0.3.0 code
  references the library by its old identifier.

### Removed

- The v0.9 per-project `App` (`src/tui/app/`, 2856 lines) and every
  module under it (`confirm`, `dialog`, `filter`, `help`, `prompt`,
  `ticket_detail`, `mod`).
- `MenuApp` (`src/tui/menu.rs`, 680 lines).
- The v0.9 task form (`src/tui/task_form.rs`, 500 lines).
- `src/tui/colors.rs` (replaced by `src/style/`).
- `src/tui/input.rs` (App-specific input router).
- The `pm` deprecation shim binary and `src/pm_shim.rs`. Users on
  v0.9.x install `spacecell` directly.
- CLI verbs: `ui`, `menu`, `legacy-tui`. Use `spacecell wf` for the
  board, or the per-verb CLI for everything else.
- Total TUI surface area: 6925 -> 1449 lines.

### What survived

- `src/tui/workflow.rs` + `workflow_run.rs` (the kanban board)
- `src/tui/utils.rs` (small layout helper)
- `src/tui/enums.rs` (slimmed: only the two types the board uses)
- The CLI (`src/cli.rs`, `src/cmd.rs`)
- The store substrate (`src/store/`)
- The MCP server (`src/mcp/`)
- The memory tiers (`src/memory/`)
- The activity feed renderer (`src/views/events_view.rs`)
- The `$EDITOR` handoff (`src/editor.rs`)

The v0.3.1 phase wraps these pieces in the LHP + Workbench + Activity
composition described in PM_DESIGN section 8.3.

## [0.2.0] - 2026-05-16

### Renamed

- **Crate**: `project_management` -> `spacecell-thunder` (new identity on
  crates.io; `project_management` retains the v0.9.x release line for
  pre-rename users).
- **Binary**: `pm` -> `spacecell`. A `pm` deprecation shim ships in this
  release that prints a notice on stderr and forwards arguments to
  `spacecell`. The shim is removed in v0.3.0.
- **MCP server identifier**: `pm` -> `spacecell-thunder` in the
  `initialize` response's `serverInfo.name`.

### Added

- New `legacy-tui` CLI subcommand. Same surface as `menu`; the new name is
  the stable entry point users should adopt now. In v0.3.0 the default
  `spacecell` invocation stops launching the v0.9 TUI and `legacy-tui`
  becomes the only path to the v1 surface, until the new cockpit ships.
- Composed view (`spacecell context <id>` and MCP `read_context`) now
  emits a `## Artifacts at <LEVEL> (<id>)` block for each chain node that
  has artifacts, followed by a trailing `@artifacts/ARTIFACTS.md` import
  line. Closes the audit gap against PM_DESIGN section 6.2.
- Two new integration tests in `tests/phase10_memory.rs` covering the
  composed-view artifact blocks and the trailing import.

### Changed

- `Cargo.toml`: license fixed from `MPL` to `MPL-2.0`. Description
  rewritten. Added `homepage` and `documentation` fields for crates.io
  presentation.

### Internal

- Library crate identifier stays `project_management` for v0.2.0 (the
  `[lib] name` override keeps all internal `use project_management::...`
  paths working). v0.3.0 may rename to `spacecell_thunder` as part of the
  TUI demolition phase.
- Substrate (`src/store/`, `src/mcp/`, `src/memory/`, `src/views/`) is
  unchanged in shape; only string identifiers shifted.

### Removed

Nothing in this release. The TUI demolition (`src/tui/app/`,
`src/tui/menu.rs`, `src/tui/task_form.rs`, etc.) lands in v0.3.0.

## [0.9.3] - 2026-04 (pre-rename, `project_management` crate)

Last release under the `project_management` name. See git history for
v0.9.x changes; this changelog starts fresh from the v0.2.0 rename.
