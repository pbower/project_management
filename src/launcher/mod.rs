//! Configured-launcher infrastructure.
//!
//! Thunder spawns OS-level terminals to host agents through a user
//! configured `spawn` command. The launcher resolves config in
//! project > user > built-in order, performs `{cmd}` / `{uuid}` /
//! `{scope}` / `{label}` / `{cwd}` substitution, exec's the result,
//! and writes a registry entry under `.pm/terminals/<uuid>.json` so
//! the cockpit and other tools can track live agents.
//!
//! See PM_DESIGN section 8.6 for the architecture.

pub mod config;
pub mod registry;
pub mod spawn;

pub use config::{
    load_config, resolve_focus_command, resolve_inner_command, resolve_spawn_command,
    LauncherConfig, ScopeSubstitution,
};
pub use registry::{
    list_terminals, load_terminal, mark_terminal_closed, purge_dead_terminals, write_terminal,
    HeartbeatThread, TerminalEntry, TerminalStatus,
};
pub use spawn::{spawn_terminal, SpawnError};
