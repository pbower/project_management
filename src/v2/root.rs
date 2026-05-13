//! Resolve the active `.pm/` root for v2 commands.
//!
//! Order of precedence:
//!
//! 1. An explicit `--pm-root <dir>` flag on the v2 subcommand.
//! 2. The `PM_ROOT` environment variable.
//! 3. A `.pm/` directory found by walking up from the current working
//!    directory.
//!
//! `init` is the only verb that does not require an existing root; every
//! other verb errors out with a friendly "run `pm v2 init` first" message
//! if nothing resolves.

use std::path::{Path, PathBuf};

use crate::store::layout::PM_DIR_NAME;
use crate::store::Layout;

/// Walk up from `start` looking for a `.pm/` directory. Returns the absolute
/// path to the discovered `.pm/`, or `None` if the walk hits the filesystem
/// root without finding one.
pub fn find_existing(start: &Path) -> Option<PathBuf> {
    let mut here: PathBuf = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    loop {
        let candidate = here.join(PM_DIR_NAME);
        if candidate.is_dir() {
            return Some(candidate);
        }
        match here.parent() {
            Some(parent) => here = parent.to_path_buf(),
            None => return None,
        }
    }
}

/// Resolve the active `.pm/` root for a v2 verb.
///
/// `explicit` is the optional `--pm-root` value supplied on the command
/// line. If absent, `PM_ROOT` env var is consulted; if also absent, the
/// cwd is walked upward to find a `.pm/` directory.
pub fn resolve(explicit: Option<&Path>) -> Result<Layout, RootError> {
    if let Some(p) = explicit {
        let pm_dir = if p.file_name() == Some(std::ffi::OsStr::new(PM_DIR_NAME)) {
            p.to_path_buf()
        } else {
            p.join(PM_DIR_NAME)
        };
        if !pm_dir.exists() {
            return Err(RootError::ExplicitNotFound(pm_dir));
        }
        return Ok(Layout::at(pm_dir));
    }
    if let Some(env_val) = std::env::var_os("PM_ROOT") {
        let p = PathBuf::from(env_val);
        let pm_dir = if p.file_name() == Some(std::ffi::OsStr::new(PM_DIR_NAME)) {
            p
        } else {
            p.join(PM_DIR_NAME)
        };
        if !pm_dir.exists() {
            return Err(RootError::EnvNotFound(pm_dir));
        }
        return Ok(Layout::at(pm_dir));
    }
    let cwd = std::env::current_dir().map_err(RootError::Cwd)?;
    match find_existing(&cwd) {
        Some(pm_dir) => Ok(Layout::at(pm_dir)),
        None => Err(RootError::NoPmRoot(cwd)),
    }
}

/// Same as [`resolve`] but for `pm v2 init`, which creates `.pm/` rather
/// than requiring it. If `explicit` is provided, init scaffolds under that
/// directory; otherwise under the current working directory.
pub fn resolve_for_init(explicit: Option<&Path>) -> Result<Layout, RootError> {
    let base = match explicit {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(RootError::Cwd)?,
    };
    // If a `.pm/` is supplied directly, treat its parent as the base.
    let layout = if base.file_name() == Some(std::ffi::OsStr::new(PM_DIR_NAME)) {
        Layout::at(base)
    } else {
        Layout::under(base)
    };
    Ok(layout)
}

#[derive(Debug)]
pub enum RootError {
    Cwd(std::io::Error),
    ExplicitNotFound(PathBuf),
    EnvNotFound(PathBuf),
    NoPmRoot(PathBuf),
}

impl std::fmt::Display for RootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootError::Cwd(e) => write!(f, "cannot read current directory: {e}"),
            RootError::ExplicitNotFound(p) => {
                write!(f, "--pm-root {} does not exist", p.display())
            }
            RootError::EnvNotFound(p) => {
                write!(f, "PM_ROOT points at {} but it does not exist", p.display())
            }
            RootError::NoPmRoot(cwd) => write!(
                f,
                "no .pm/ found walking up from {}. Run `pm v2 init` to create one.",
                cwd.display(),
            ),
        }
    }
}

impl std::error::Error for RootError {}
