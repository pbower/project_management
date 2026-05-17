# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
