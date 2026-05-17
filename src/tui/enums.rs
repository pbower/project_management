//! Enumerations for TUI state management.

use crate::store::{LeafId, MemoryRef};

/// Top-level TUI mode. Mode 1 (Tickets) hosts the existing per-screen
/// [`AppState`] flow; Modes 2 and 3 are their own surfaces, stubbed until
/// Phases 8 and 9 land.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Mode 1 - the hierarchical Ticket View.
    Tickets,
    /// Mode 2 - the Document Workspace (Phase 8).
    Documents,
    /// Mode 3 - the Activity View (Phase 9).
    Activity,
}

impl Mode {
    /// The next mode in the `Tab` cycle.
    pub fn next(self) -> Mode {
        match self {
            Mode::Tickets => Mode::Documents,
            Mode::Documents => Mode::Activity,
            Mode::Activity => Mode::Tickets,
        }
    }

    /// The previous mode in the `Shift+Tab` cycle.
    pub fn prev(self) -> Mode {
        match self {
            Mode::Tickets => Mode::Activity,
            Mode::Documents => Mode::Tickets,
            Mode::Activity => Mode::Documents,
        }
    }

    /// Short label shown in the header.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Tickets => "Mode 1 Tickets",
            Mode::Documents => "Mode 2 Documents",
            Mode::Activity => "Mode 3 Activity",
        }
    }
}

/// Application state for the terminal user interface.
#[derive(Clone, Copy, PartialEq)]
pub enum AppState {
    TaskList,
    TaskDetail,
    AddTask,
    EditTask,
    UserStoryDialog,
    RequirementsDialog,
    Confirm,
}

/// Input mode for text entry fields.
#[derive(Clone)]
pub enum InputMode {
    None,
    Text,
}

/// What an active single-line input prompt is collecting.
pub enum PromptType {
    /// A path to a file to copy into the given ticket's `artifacts/` dir.
    ArtifactPath(LeafId),
    /// A new title for the ticket, or a `move <ADDRESS>` instruction to
    /// reparent it.
    RenameTicket(LeafId),
}

/// An active single-line input prompt overlaid on the current mode.
pub struct PromptState {
    pub prompt_type: PromptType,
    pub buffer: String,
}

/// A transient surface layered over the current mode. At most one is active
/// at a time, so a single enum is the source of truth - deliberately not a
/// scatter of boolean flags that could fall out of sync.
pub enum Overlay {
    /// Nothing layered; the mode owns the screen.
    None,
    /// The modal help overlay, carrying its vertical scroll offset.
    Help { scroll: u16 },
    /// The memory side-panel for the selected ticket.
    MemoryPanel,
    /// A single-line input prompt.
    Prompt(PromptState),
    /// The Mode 2 modal for linking and unlinking memories.
    MemoryLink(MemoryLinkState),
}

/// One row in the [`MemoryLinkState`] modal.
pub struct MemoryLinkRow {
    pub reference: MemoryRef,
    pub linked: bool,
}

/// State for the memory link/unlink modal in Mode 2.
pub struct MemoryLinkState {
    pub ticket: LeafId,
    pub rows: Vec<MemoryLinkRow>,
    pub cursor: usize,
    pub dirty: bool,
}

/// A deferred operation picked up by the run loop after the input phase,
/// because it suspends the terminal and so cannot run mid-render. Distinct
/// from [`Overlay`]: an overlay is a visible surface that input is routed to,
/// this is a one-shot action consumed on the next loop tick.
pub enum PendingAction {
    /// Open the given ticket's `CLAUDE.md` in `$EDITOR`.
    EditTicket(LeafId),
    /// Open an arbitrary file in `$EDITOR`, optionally jumping the cursor
    /// to the given section heading. The owning ticket is tracked so the
    /// post-edit hooks (event emission, artifact sweep, task refresh) target
    /// the right node.
    EditDoc {
        ticket: LeafId,
        path: std::path::PathBuf,
        section: Option<String>,
    },
}

/// State for Mode 2 - the Document Workspace.
///
/// Owns the breadcrumb chain of leaf ids that anchors the doc list and the
/// preview pane. `active_level` is the cursor position inside the breadcrumb
/// (Left / Right moves it); `doc_cursor` is the cursor position inside the
/// flat LHS list at the focused level.
#[derive(Clone, Debug, Default)]
pub struct DocumentsState {
    /// Address chain from root to focused leaf. Empty when no ticket has
    /// been selected via Mode 1 yet.
    pub crumb: Vec<LeafId>,
    /// Index into `crumb` of the active level. The renderer treats
    /// `crumb[active_level]` as the focused ticket.
    pub active_level: usize,
    /// Selection index within the flattened LHS list of docs, memories, and
    /// sections at the focused ticket.
    pub doc_cursor: usize,
}

/// Hierarchy levels for navigation context.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HierarchyLevel {
    Project,
    Product,
    Epic,
    Task,
    Subtask,
    Milestone,
}

/// Context for hierarchical navigation in the TUI.
#[derive(Clone, PartialEq, Debug)]
pub struct NavigationContext {
    pub level: HierarchyLevel,
    pub parent_id: Option<LeafId>,
    pub parent_title: Option<String>,
}

impl NavigationContext {
    /// Create a context for viewing all projects - the top of the v2
    /// hierarchy and the default landing view for a workspace.
    pub fn new_all_projects() -> Self {
        NavigationContext {
            level: HierarchyLevel::Project,
            parent_id: None,
            parent_title: None,
        }
    }

    /// Create a context for viewing all items at a specific hierarchy level.
    pub fn new_all_level(level: HierarchyLevel) -> Self {
        NavigationContext {
            level,
            parent_id: None,
            parent_title: None,
        }
    }

    /// Create a context for viewing items filtered by a specific parent.
    pub fn new_filtered(level: HierarchyLevel, parent_id: LeafId, parent_title: String) -> Self {
        NavigationContext {
            level,
            parent_id: Some(parent_id),
            parent_title: Some(parent_title),
        }
    }

    /// Get a human-readable display name for this navigation context.
    pub fn get_display_name(&self) -> String {
        match (&self.parent_id, &self.parent_title) {
            (Some(id), Some(title)) => {
                let parent_type = match self.level {
                    HierarchyLevel::Product => "Project",
                    HierarchyLevel::Epic => "Product",
                    HierarchyLevel::Task => "Epic",
                    HierarchyLevel::Subtask => "Task",
                    HierarchyLevel::Milestone => "Parent", // Special case
                    HierarchyLevel::Project => "Parent",   // Top of the hierarchy
                };
                format!(
                    "All {}s for {} {} {}",
                    format!("{:?}", self.level),
                    parent_type,
                    id,
                    title
                )
            }
            _ => format!("All {}s", format!("{:?}", self.level)),
        }
    }
}
