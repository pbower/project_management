# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.8] - 2026-05-18

Bug-fix release for v0.3.7. The embedded agent was reachable but
invisible because of two issues that compounded into "press r,
activity feed says spawned, surface stays blank".

### Fixed

- **Agents surface looked up by LHP scope, not the spawned ticket.**
  Pressing `r` on a board card spawned an agent keyed on that card
  (e.g., `TSK7`) but the surface rendered against the LHP's current
  scope (often `PRJ1` if the user hadn't drilled down). The lookup
  missed and the surface showed its empty state. `AgentsState` now
  carries an `active: Option<LeafId>` that the shell pins on spawn;
  `resolved()` prefers the active pin, falls back to LHP scope, and
  falls back to `None` only when neither resolves to a live agent.
- **`portable-pty`'s `take_writer` consumed the writer on first
  call.** `Agent::write` called `take_writer` per keystroke, so every
  byte after the first was silently dropped. Cached the writer at
  spawn time; every subsequent call writes through the same handle.
  Regression test `writing_to_pty_works_across_multiple_keystrokes`
  sends two payloads through `cat` and verifies both round-trip.

### Added

- **Spawn banner.** A fresh agent's parser is seeded with a single
  line - `▌ <leaf>   agent starting: <command>` - so the surface
  never renders an empty 80x24 grid even before the child's first
  byte arrives. The child's own output overwrites the banner.

### Tests

- 303 pass (+1 regression test for the writer cache).

### Notes

- If `claude` is not on `$PATH`, the inner command exits 127 and
  `bash: claude: command not found` lands in the PTY's screen.
  Visible inside the surface; the agent's status flips to `EXITED`.
  To test without `claude`, set `[launcher] inner_command = "bash"`
  in the workspace's `.pm/.thunder.toml`.

## [0.3.7] - 2026-05-18

Embedded agent PTYs. `spacecell` is now an agent host, not just a
driver. Press `r` on a board card and a real PTY-backed Claude
session spawns inside the cockpit, with its stdout rendered in the
Agents surface and your keystrokes piped straight to its stdin. The
v0.3.5 external-launcher path stays available via `spacecell run`
for users who prefer separate OS windows.

### Added

- `src/agents/`: in-process agent management.
  - `Agent` owns a PTY master + vt100 parser + reader thread that
    drains the PTY in the background and pushes bytes through an
    mpsc channel into the parser.
  - `AgentManager` keyed by `LeafId`; one agent per ticket for
    v0.3.7. `poll_all` drains every live agent in a single shell
    tick. `close` kills + reaps.
  - `scope_env` returns `THUNDER_SCOPE` / `THUNDER_LABEL` /
    `PM_AGENT_ID` / `PM_TICKET` so the child sees the same env an
    OS-launcher terminal would.
- `src/tui/workbench/agents.rs` (Agents surface): renders the
  focused ticket's PTY screen by walking the vt100 cells and
  building styled ratatui spans. Resizes the PTY to match the
  visible rect on every render. Empty state when the focused scope
  has no agent.
- `Disposition::SpawnAgent(LeafId)`: zone-level handlers ask the
  shell to spawn an embedded agent + auto-switch to Agents mode.
  Wired to the `r` key on the Board surface.
- `Shell::spawn_agent_and_switch`: resolves the per-kind inner
  command (default `claude`), resolves the scope's cwd from
  `state.json`, spawns through the WorkbenchState's manager, and
  flips `Mode::Agents` + Workbench focus so the user sees the PTY
  immediately.
- New deps: `portable-pty` 0.9 (PTY abstraction from wezterm),
  `shpool_vt100` 0.1 (vt100 terminal emulator fork that resolves
  cleanly against ratatui 0.29's `unicode-width = 0.2.0` pin).

### Changed

- `Mode` rename: `Mode::Terminals` -> `Mode::Agents`. Label updates
  in footer + help + header.
- Workbench's surface enum now has an `Agents` variant; the v0.3.6
  external-terminals registry surface is removed (replaced by the
  embedded PTY view + the CLI `spacecell terminals`).
- `Workbench::handle_key` now takes a full `KeyEvent` rather than a
  `KeyCode` + `KeyModifiers` pair so the Agents surface has the
  fidelity it needs to encode keystrokes for the PTY.
- `Workbench::render` and `Shell::render` are `&mut` so the PTY's
  resize-on-render and reader-thread drain can mutate manager state.
- `Disposition` lost `Copy` (already in v0.3.6 because of
  `EditPath`); v0.3.7 adds `SpawnAgent`.

### Removed

- `src/tui/workbench/terminals.rs` (the v0.3.6 launcher-registry
  view). The Agents surface covers the in-cockpit case; the CLI
  covers external windows.

### Tests

- `tests/phase_v0_3_7_agents.rs`: four end-to-end tests using real
  PTYs.
  - `spawning_echo_renders_output_into_the_pty_screen` confirms
    `echo` output lands in the parsed vt100 screen.
  - `writing_to_pty_round_trips_through_cat` confirms keystrokes
    written to the PTY master come back through stdout.
  - `manager_enforces_one_agent_per_ticket` covers spawn / close /
    re-spawn idempotency.
  - `scope_env_injects_thunder_scope_for_child_processes` confirms
    the env pairs every child inherits.
- Total: 302 tests pass (+4 from v0.3.6 baseline).

### Notes

- Per-kind inner command override (`.pm/templates/<kind>.toml`
  `[terminal] command = "..."`) still pending; v0.3.7 only consults
  the workspace-wide `[launcher] inner_command`.
- Multi-agent per ticket: deferred. The manager is keyed on
  `LeafId`; lifting that to `(LeafId, index)` is a one-line change
  but the surface UX (tabs vs splits) needs design first.

## [0.3.6] - 2026-05-18

Populated Workbench surfaces. The v0.3.1 placeholders are gone; every
mode now shows live content, and a Templates overlay reachable via `t`
opens the per-kind config files in `$EDITOR`.

### Added

- `src/tui/workbench/memories.rs` (Memories surface): three-tier
  browser that enumerates user / project / ticket memories. `Enter`
  hands the focused file to `$EDITOR`; arrow keys move the cursor and
  overflow flips focus back to the LHP.
- `src/tui/workbench/terminals.rs` (Terminals surface): live launcher
  registry view with a header summarising active vs total entries.
  `o` invokes the configured focus command; `K` sends `SIGINT` to the
  recorded pid; `Enter` opens the terminal's scope in the ticket
  editor.
- `src/tui/workbench/templates.rs` (Templates overlay): lists the six
  per-kind template files (`project / product / epic / task / subtask
  / milestone`) plus the workspace launcher config. `Enter` opens the
  selected file in `$EDITOR`. `t` from any other surface toggles the
  overlay on / off.
- `Disposition::EditPath(PathBuf)` and `ShellOutcome::EditPath`. The
  shell suspends the alternate screen and runs `$EDITOR` against any
  path a surface hands it, then resumes and reloads `Database`.

### Changed

- `Mode` variants renamed for clarity (variant names and labels both
  shift):
  - `Mode::Documents` -> `Mode::Memories`
  - `Mode::Activity` -> `Mode::Terminals`
  - `Mode::Board` unchanged.
  The activity feed remains the bottom strip in every mode, so the
  full-screen Activity placeholder is gone; `spacecell tv` keeps the
  kiosk path.
- `Workbench::handle_key` and `Workbench::render` gained `pm_dir` and
  `scope` parameters so the new surfaces can read the launcher
  registry and the memory tier directories.
- `Disposition` lost `Copy`; `PathBuf` inside `EditPath` is not
  `Copy`. Callers move the value, the trait removal is invisible.

### Removed

- `src/tui/workbench/placeholder.rs` and the "coming soon" copy that
  shipped with v0.3.1.

### Tests

- `tests/phase_v0_3_6_surfaces.rs`: five integration tests cover the
  Terminals registry round-trip, terminal-spawn scope tagging, memory
  directory layout invariants, Templates surface paths matching the
  launcher's actual lookups, and confirmation that `terminal-spawn`
  events stay out of the state-change ticker.
- Total: 298 tests pass (+5 from v0.3.5 baseline).

### Notes

- Per-kind form template loader (`.pm/templates/<kind>.toml`)
  remains on the v0.3.7 list. v0.3.6 ships the file-browsing surface
  so users can author and edit those templates today; the consumer
  side (per-kind quick-entry form fields, per-kind inner command)
  follows next.

## [0.3.5] - 2026-05-18

Configured-launcher agent terminals, terminal registry, MCP scope
enforcement. This is the phase where Thunder stops being just a TUI
and starts orchestrating agents.

### Added

- `src/launcher/` module with three submodules:
  - `config`: TOML loader for `<pm_dir>/.thunder.toml` and
    `~/.config/spacecell/launcher.toml`. Resolution order is project
    > user > built-in default (`$SHELL -c '{cmd}'`). Substitutions
    available in the spawn template: `{cmd}`, `{uuid}`, `{scope}`,
    `{label}`, `{cwd}`.
  - `registry`: `.pm/terminals/<uuid>.json` per spawned terminal.
    Atomic writes via the same `store::state::atomic_write` the rest
    of the workspace uses. `TerminalEntry { uuid, scope, agent_id,
    pid, spawned_at, last_heartbeat, label, spawn_command, status }`.
    `HeartbeatThread` refreshes `last_heartbeat` every 30s while the
    agent runs. `purge_dead_terminals` sweeps for stale entries (TTL
    120s) and can either mark them `Dead` or delete the registry
    files outright.
  - `spawn`: glue. Generates a short URL-safe UUID, substitutes the
    template, exec's it through `$SHELL -c`, writes the registry
    entry, and emits a `terminal-spawn` event into `events.log` so
    the cockpit's activity strip surfaces the spawn live.

- New CLI verbs:
  - `spacecell run <id>`: spawn a terminal scoped to a ticket.
  - `spacecell terminals`: list registry entries with status column
    that flags stale-heartbeat actives as `STALE`.
  - `spacecell focus <uuid>`: invoke the configured focus command;
    falls back to printing the entry if no focus is configured.
  - `spacecell agent --window <uuid>`: internal entry point exec'd
    inside spawned terminals. Sets `THUNDER_WINDOW`, `THUNDER_SCOPE`,
    `PM_AGENT_ID`; prints the brand-styled scope header; starts the
    heartbeat thread; exec's the inner command (`claude` by default,
    overridable via `[launcher] inner_command = "..."`).
  - `spacecell doctor --purge-terminals [--delete]`: clean stale
    registry entries. Without `--delete` they get flipped to `Dead`
    so the user can still see what was spawned.

- Optional `scope: Option<LeafId>` field on `store::events::Event`
  plus `store::events::scope_from_env` so events emitted from any
  process inside a scoped terminal get the `THUNDER_SCOPE` tag
  automatically.

- MCP scope enforcement (soft). The MCP server reads `THUNDER_SCOPE`
  on construction; every mutating handler (`write_doc`, `checkout`,
  `checkin`, `complete`, `link`) calls `record_scope_violation_if_any`
  which emits a `warning` event with detail `out-of-scope: <verb>
  <target> from scope <scope>` when the target ticket is not a
  descendant of the launch scope. The operation proceeds regardless;
  the audit trail is the enforcement mechanism rather than a refusal.

### Tests

- 12 unit tests across `launcher::config`, `launcher::registry`,
  `launcher::spawn`.
- 6 integration tests in `tests/phase_v0_3_5_launcher.rs`:
  - registry round-trips an entry written by spawn
  - spawn emits a `terminal-spawn` event with the scope tag
  - purge flips status to `Dead` when not deleting
  - purge deletes when asked
  - `label_for` formats consistently
  - MCP scope enforcement: in-scope checkout passes silently;
    out-of-scope checkout emits exactly one warning event with the
    correct `id` and `scope` fields.

Total: 293 tests pass (+18 from v0.3.4 baseline).

### Notes

- Per-kind inner command (`[terminal] command = "..."` in
  `.pm/templates/<kind>.toml`) lands with the templates work in a
  later phase. v0.3.5 reads only the workspace-wide
  `[launcher] inner_command` override.
- The launcher does not require tmux. Users who want tmux configure
  `spawn = "tmux new-window -n thunder-{scope} -- {cmd}"`. i3 users
  wire `spawn = "i3-msg exec 'alacritty -T {label} --command {cmd}'"`.
  See `docs/launchers.md` (added in a later polish phase) for the
  recipe library.
- Library `Context` in the MCP server gained a public `scope` field
  and `record_scope_violation_if_any` method. Existing callers stay
  source-compatible because the constructor still takes only `pm_dir`.

## [0.3.4] - 2026-05-18

Live activity refresh + state-change ticker.

### Added

- `src/tui/activity/ticker.rs`: state-change ticker on the right side
  of the bottom activity strip. Surfaces the last eight transitions
  with verbs in `{status, priority, complete, move, reopen, checkin}`.
  Most-recent row at the top; verb gets a semantic colour (gold for
  in-progress moves, green for completes, muted for reopens).
- `tests/phase_v0_3_4_live_refresh.rs`: three integration tests that
  exercise the watcher end-to-end - idle poll, external append wakes
  the watcher within 2s, missing `events.log` falls back to throttled
  polling and still picks up later writes.

### Changed

- `ActivityStrip` swapped its 500ms polling for a
  `notify-debouncer-mini` watcher on `events.log`. The strip's new
  `poll()` method drains the watcher channel and re-reads the buffer
  only when something fired (or the 500ms fallback throttle expires
  on a watcher-less filesystem). Returns `true` when the event buffer
  grew, so the shell knows the workspace state may have changed.
- `src/tui/shell/mod.rs` now reloads `Database` whenever `poll()`
  reports a buffer growth. External CLI mutations (`spacecell status
  TSK7 in_progress`, etc.) propagate into the LHP counts and the
  board's content without the user pressing anything.
- Activity strip layout: 2-column inside the bordered band. Events
  feed on the left (variable width), state-change ticker on the
  right (fixed 34 cells).

### Notes

- The watcher silently falls back to polling when `events.log` does
  not yet exist at strip-construction time. The shell's poll loop
  still picks up appends after creation through the throttled path,
  so a fresh workspace is no worse than v0.3.1.
- Debouncer kept alive for the strip's lifetime via an unnamed field
  so the OS watcher thread does not get dropped mid-session.

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
