//! In-cockpit agent PTYs.
//!
//! Each ticket can host one agent session at a time. The session is a
//! child process (`claude` by default) spawned through `portable-pty`;
//! Thunder reads the PTY output in a background thread and feeds the
//! bytes into a [`shpool_vt100::Parser`]. The active Workbench surface
//! renders the parsed screen, and keystrokes typed while the Agents
//! surface owns focus write straight to the PTY master.
//!
//! v0.3.7 ships one PTY per ticket. Multi-PTY support (parallel
//! agents on the same scope) is a later phase; the surface UX would
//! become a tab strip in that mode and the manager would key on
//! `(LeafId, agent_index)` instead of `LeafId` alone.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use shpool_vt100::Parser;

use crate::store::LeafId;

/// Default rows/cols used when the surface has not yet computed its
/// rect. Resized on the first render.
const INITIAL_ROWS: u16 = 24;
const INITIAL_COLS: u16 = 80;

/// Status of a hosted agent. Reflects what the manager observed last;
/// the surface uses it to pick a status badge and decide whether to
/// route keystrokes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Exited,
}

/// One hosted agent. Wraps the PTY master, the killer handle, the
/// vt100 parser, and the channels that ferry PTY bytes from the
/// background reader thread into the parser.
pub struct Agent {
    pub leaf: LeafId,
    pub command: String,
    pub status: AgentStatus,
    parser: Arc<Mutex<Parser>>,
    master: Box<dyn MasterPty + Send>,
    /// Cached writer for the PTY master. `portable-pty`'s
    /// `take_writer` succeeds once; calling it on every keystroke
    /// returns `Err` after the first call and silently drops input.
    /// We grab it at spawn time and reuse it for the agent's lifetime.
    writer: Option<Box<dyn std::io::Write + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    rx: Receiver<Vec<u8>>,
    rows: u16,
    cols: u16,
}

impl Agent {
    /// Spawn `command` inside a freshly created PTY scoped to `leaf`.
    /// `cwd` defaults to the workspace root; the caller resolves the
    /// scope path before calling.
    pub fn spawn(
        leaf: LeafId,
        command: &str,
        cwd: Option<&Path>,
        extra_env: &[(String, String)],
    ) -> Result<Self, AgentError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: INITIAL_ROWS,
                cols: INITIAL_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| AgentError::Spawn(format!("openpty: {e}")))?;

        // Build the child process command. `$SHELL -c <command>` so
        // the user's per-kind template can be a full command line.
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-c");
        cmd.arg(command);
        if let Some(dir) = cwd {
            cmd.cwd(dir.as_os_str());
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| AgentError::Spawn(format!("spawn: {e}")))?;
        let killer = child.clone_killer();

        let parser = Arc::new(Mutex::new(Parser::new(INITIAL_ROWS, INITIAL_COLS, 1000)));

        // Seed the parser with a banner so a freshly spawned agent
        // never looks empty before its first byte arrives. The
        // child's own output will overwrite the top-left cells the
        // first time it draws.
        if let Ok(mut p) = parser.lock() {
            let banner = format!(
                "\x1b[1;33m\u{258c} {leaf}\x1b[0m   \x1b[2magent starting: {command}\x1b[0m\r\n"
            );
            p.process(banner.as_bytes());
        }

        // Background reader: blocking read from the PTY master into
        // an mpsc channel the main shell drains every tick.
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| AgentError::Spawn(format!("clone_reader: {e}")))?;
        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Best-effort: reap the child on exit so the OS does not
        // accumulate zombies if the user spawns and closes many
        // agents in a single session. The killer handle still works
        // while this thread blocks on `wait`.
        thread::spawn(move || {
            let _ = child.wait();
        });

        // Grab the writer up front. `take_writer` only succeeds on
        // the first call; caching it here lets every subsequent
        // `Agent::write` succeed for the agent's lifetime.
        let writer = pair.master.take_writer().ok();

        Ok(Agent {
            leaf,
            command: command.to_string(),
            status: AgentStatus::Running,
            parser,
            master: pair.master,
            writer,
            killer,
            rx,
            rows: INITIAL_ROWS,
            cols: INITIAL_COLS,
        })
    }

    /// Drain any bytes the reader thread has queued and feed them
    /// into the parser. Returns whether the screen content changed.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        loop {
            match self.rx.try_recv() {
                Ok(bytes) => {
                    if let Ok(mut p) = self.parser.lock() {
                        p.process(&bytes);
                    }
                    any = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Reader thread is gone; the child has exited or
                    // crashed. Flip status so the surface can show it.
                    self.status = AgentStatus::Exited;
                    break;
                }
            }
        }
        any
    }

    /// Write keystrokes to the PTY master. Bytes go to the child
    /// process's stdin. Errors are silenced because a dead child has
    /// nowhere useful to surface the failure beyond the next status
    /// poll.
    pub fn write(&mut self, bytes: &[u8]) {
        if self.status != AgentStatus::Running {
            return;
        }
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        let _ = writer.write_all(bytes);
        let _ = writer.flush();
    }

    /// Resize the PTY when the surface's allocated area changes.
    /// Cheap; the v0.3.7 surface calls this on every render.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == 0 || cols == 0 {
            return;
        }
        if rows == self.rows && cols == self.cols {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.screen_mut().set_size(rows, cols);
        }
        self.rows = rows;
        self.cols = cols;
    }

    /// Hand out a clone of the parser Arc so the renderer can read
    /// the screen without holding the manager's lock.
    pub fn parser(&self) -> Arc<Mutex<Parser>> {
        Arc::clone(&self.parser)
    }

    /// Send SIGINT to the child. The reader thread will see EOF and
    /// the status will flip to `Exited` on the next poll.
    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

/// Manager owning every live agent. Keyed by ticket leaf id; one
/// agent per ticket for v0.3.7.
pub struct AgentManager {
    agents: HashMap<LeafId, Agent>,
}

impl AgentManager {
    pub fn new() -> Self {
        AgentManager {
            agents: HashMap::new(),
        }
    }

    /// Get the agent for `leaf`, if any.
    pub fn get(&self, leaf: LeafId) -> Option<&Agent> {
        self.agents.get(&leaf)
    }

    pub fn get_mut(&mut self, leaf: LeafId) -> Option<&mut Agent> {
        self.agents.get_mut(&leaf)
    }

    /// Spawn an agent for `leaf`. Returns `Err` when one already
    /// exists or the spawn itself fails; the caller is expected to
    /// stop the existing one first if it wants to recycle.
    pub fn spawn(
        &mut self,
        leaf: LeafId,
        command: &str,
        cwd: Option<&Path>,
        extra_env: &[(String, String)],
    ) -> Result<(), AgentError> {
        if self.agents.contains_key(&leaf) {
            return Err(AgentError::AlreadyExists);
        }
        let agent = Agent::spawn(leaf, command, cwd, extra_env)?;
        self.agents.insert(leaf, agent);
        Ok(())
    }

    /// Drain every agent's reader queue. Returns the set of leaves
    /// whose screen changed so the shell can decide whether to
    /// re-render.
    pub fn poll_all(&mut self) -> Vec<LeafId> {
        let mut changed: Vec<LeafId> = Vec::new();
        for (leaf, agent) in self.agents.iter_mut() {
            if agent.poll() {
                changed.push(*leaf);
            }
        }
        changed
    }

    /// Stop and remove the agent for `leaf`. Idempotent.
    pub fn close(&mut self, leaf: LeafId) {
        if let Some(mut agent) = self.agents.remove(&leaf) {
            agent.kill();
        }
    }

    pub fn live_leaves(&self) -> Vec<LeafId> {
        self.agents.keys().copied().collect()
    }
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors the spawn path can surface.
#[derive(Debug)]
pub enum AgentError {
    AlreadyExists,
    Spawn(String),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::AlreadyExists => write!(f, "an agent is already running for this scope"),
            AgentError::Spawn(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AgentError {}

/// Build the env pairs the spawned agent inherits from Thunder.
/// Mirrors what `spacecell agent --window` would have set, minus the
/// window UUID (the embedded agent has no registry entry by design).
pub fn scope_env(leaf: LeafId, label: &str) -> Vec<(String, String)> {
    let agent_id = std::env::var("PM_AGENT_ID").unwrap_or_else(|_| {
        let pid = std::process::id();
        format!("claude-embedded-{pid}")
    });
    vec![
        ("THUNDER_SCOPE".to_string(), leaf.to_string()),
        ("THUNDER_LABEL".to_string(), label.to_string()),
        ("PM_AGENT_ID".to_string(), agent_id),
        ("PM_TICKET".to_string(), leaf.to_string()),
    ]
}

/// Resolve the inner command for a ticket: per-kind override from
/// `.pm/templates/<kind>.toml` if present, falling back to the
/// workspace-wide launcher config, then the built-in default. v0.3.7
/// only consults the workspace-wide override; the per-kind loader
/// lands as part of the same release.
pub fn resolve_inner_command(pm_dir: &Path, _leaf: LeafId) -> String {
    let cfg = crate::launcher::load_config(pm_dir);
    crate::launcher::resolve_inner_command(&cfg, crate::fields::Kind::Task)
}

/// Best-effort helper: where should the agent's cwd be?  Uses the
/// resolved ticket path if the ticket lives in `state.json`; falls
/// back to the workspace root otherwise.
pub fn cwd_for_scope(pm_dir: &Path, db: &crate::db::Database, leaf: LeafId) -> PathBuf {
    db.state
        .items
        .get(&leaf)
        .map(|entry| pm_dir.join(&entry.path))
        .unwrap_or_else(|| pm_dir.to_path_buf())
}
