//! Launcher configuration: project-scoped `.pm/.thunder.toml` and
//! user-scoped `~/.config/spacecell/launcher.toml`, plus a built-in
//! fallback so a fresh workspace can spawn terminals without setup.
//!
//! Resolution order, deepest first:
//!
//! 1. `<pm_dir>/.thunder.toml`
//! 2. `~/.config/spacecell/launcher.toml`
//! 3. Built-in default `$SHELL -c '{cmd}'`
//!
//! The per-kind `[terminal] command = "..."` lives in the section
//! template at `.pm/templates/<kind>.toml`; this module only handles
//! the launcher (how to spawn) rather than what to spawn.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fields::Kind;
use crate::store::LeafId;

/// Default inner command. The launcher exec's `spacecell agent
/// --window <uuid>` which then exec's this. Configurable per kind via
/// `.pm/templates/<kind>.toml` once the per-kind template work lands.
pub const DEFAULT_INNER_COMMAND: &str = "claude";

/// The built-in fallback spawn template. Uses `$SHELL` so the user's
/// preferred shell still runs the command line.
pub const BUILTIN_SPAWN: &str = "$SHELL -c '{cmd}'";

/// Substitutions a launcher template may reference. Every field is
/// optional in the user's template; missing values render as empty
/// strings so a minimal `spawn = "$SHELL -c '{cmd}'"` works.
#[derive(Clone, Debug, Default)]
pub struct ScopeSubstitution {
    /// The full command Thunder wants executed. Typically
    /// `spacecell agent --window <uuid>`. Renders as `{cmd}`.
    pub cmd: String,
    /// Window UUID. Renders as `{uuid}` and is exported to the child
    /// process as `THUNDER_WINDOW`.
    pub uuid: String,
    /// Leaf id of the launch scope, e.g. `EPC3`. Renders as
    /// `{scope}` and exports as `THUNDER_SCOPE`.
    pub scope: String,
    /// Human-readable scope label, e.g. `EPC3 checkouts`. Renders as
    /// `{label}`.
    pub label: String,
    /// cwd the terminal should open in. Renders as `{cwd}`.
    pub cwd: String,
}

impl ScopeSubstitution {
    /// Apply substitution to a template string. Unknown placeholders
    /// pass through unchanged so the user's shell can see them and
    /// debug their config.
    pub fn apply(&self, template: &str) -> String {
        template
            .replace("{cmd}", &self.cmd)
            .replace("{uuid}", &self.uuid)
            .replace("{scope}", &self.scope)
            .replace("{label}", &self.label)
            .replace("{cwd}", &self.cwd)
    }
}

/// On-disk launcher configuration. Both project and user files use
/// this shape; missing fields fall through to the next resolution
/// layer.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LauncherConfig {
    #[serde(default)]
    pub launcher: LauncherSection,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LauncherSection {
    /// Spawn command template, with `{cmd}` etc. placeholders.
    pub spawn: Option<String>,
    /// Optional focus command for `spacecell focus <uuid>`. Renders
    /// the same placeholders as `spawn`.
    pub focus: Option<String>,
    /// Optional override for the inner command exec'd inside the
    /// spawned terminal. Defaults to [`DEFAULT_INNER_COMMAND`].
    pub inner_command: Option<String>,
}

/// Load and merge launcher config from project + user files. Either
/// path missing is fine; an empty section gets returned.
pub fn load_config(pm_dir: &Path) -> LauncherConfig {
    let mut merged = LauncherConfig::default();

    if let Some(user) = read_user_config() {
        merge_into(&mut merged, user);
    }
    if let Some(project) = read_project_config(pm_dir) {
        merge_into(&mut merged, project);
    }

    merged
}

fn merge_into(into: &mut LauncherConfig, from: LauncherConfig) {
    if from.launcher.spawn.is_some() {
        into.launcher.spawn = from.launcher.spawn;
    }
    if from.launcher.focus.is_some() {
        into.launcher.focus = from.launcher.focus;
    }
    if from.launcher.inner_command.is_some() {
        into.launcher.inner_command = from.launcher.inner_command;
    }
}

fn read_project_config(pm_dir: &Path) -> Option<LauncherConfig> {
    let path = pm_dir.join(".thunder.toml");
    let raw = fs::read_to_string(&path).ok()?;
    toml::from_str(&raw).ok()
}

fn read_user_config() -> Option<LauncherConfig> {
    let path = user_config_path()?;
    let raw = fs::read_to_string(&path).ok()?;
    toml::from_str(&raw).ok()
}

fn user_config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/spacecell/launcher.toml"))
}

/// The spawn template that should be exec'd for this workspace,
/// falling back to the built-in default when no config sets it.
pub fn resolve_spawn_command(cfg: &LauncherConfig) -> String {
    cfg.launcher
        .spawn
        .clone()
        .unwrap_or_else(|| BUILTIN_SPAWN.to_string())
}

/// The focus command if one is configured; `None` means the user did
/// not wire focus-on-card and Thunder should print a UUID rather than
/// try to focus.
pub fn resolve_focus_command(cfg: &LauncherConfig) -> Option<String> {
    cfg.launcher.focus.clone()
}

/// The inner command this kind's terminal should exec. v0.3.5 ignores
/// `kind` and returns the workspace-wide default (per-kind override
/// lands with the v0.3.6 templates work); the signature is in place
/// now so call sites do not change later.
pub fn resolve_inner_command(cfg: &LauncherConfig, _kind: Kind) -> String {
    cfg.launcher
        .inner_command
        .clone()
        .unwrap_or_else(|| DEFAULT_INNER_COMMAND.to_string())
}

/// Helper to build the label substitution for a scope. Pure string
/// formatting; lives here so the spawn path and any future "preview
/// the spawn line before running" surface share one source of truth.
pub fn label_for(scope: LeafId, title: Option<&str>) -> String {
    match title {
        Some(t) => format!("{scope} {t}"),
        None => scope.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_spawn_resolves_when_nothing_is_configured() {
        let cfg = LauncherConfig::default();
        assert_eq!(resolve_spawn_command(&cfg), BUILTIN_SPAWN);
        assert!(resolve_focus_command(&cfg).is_none());
        assert_eq!(
            resolve_inner_command(&cfg, Kind::Task),
            DEFAULT_INNER_COMMAND
        );
    }

    #[test]
    fn substitution_replaces_all_placeholders() {
        let sub = ScopeSubstitution {
            cmd: "spacecell agent --window u".into(),
            uuid: "u".into(),
            scope: "TSK7".into(),
            label: "TSK7 lock".into(),
            cwd: "/work".into(),
        };
        let line = sub.apply("tmux new-window -n thunder-{scope} -- {cmd}");
        assert_eq!(
            line,
            "tmux new-window -n thunder-TSK7 -- spacecell agent --window u"
        );

        let with_label = sub.apply("alacritty -T {label} --command {cmd}");
        assert_eq!(
            with_label,
            "alacritty -T TSK7 lock --command spacecell agent --window u"
        );
    }

    #[test]
    fn unknown_placeholders_pass_through_unchanged() {
        let sub = ScopeSubstitution::default();
        let line = sub.apply("echo {nope}");
        assert_eq!(line, "echo {nope}");
    }

    #[test]
    fn project_config_overrides_user_then_builtin() {
        let mut user = LauncherConfig::default();
        user.launcher.spawn = Some("user-spawn".into());
        let mut project = LauncherConfig::default();
        project.launcher.spawn = Some("project-spawn".into());
        let mut merged = LauncherConfig::default();
        merge_into(&mut merged, user);
        merge_into(&mut merged, project);
        assert_eq!(resolve_spawn_command(&merged), "project-spawn");
    }
}
