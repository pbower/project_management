# PM - Project Management CLI

Command-line project management tool with hierarchical task organisation and an optional terminal user interface (TUI).

The key use case is rapid low-effort personal project planning for an individual, avoiding web-tool overhead.

It creates a simple local file ~/.pm/tasks.json which it uses as the database, with basic CSV export,
and no external integrations, so you retain complete control.

**Change Log**:
V0.9.3: *Added Workflow Ticket Manager*
V0.9.0-2: Initial Public Release

## Features

- **Hierarchical Task Organisation**: Organise work using a four-level hierarchy:
  - **Product** → **Epic** → **Task** → **Subtask**
  - **Milestone** (can be attached to any level)

- **Rich Task Metadata**:
  - Title, summary, description, and user stories
  - Requirements specification and artifacts tracking
  - Priority levels (Must Have, Nice to Have, Cut First)
  - Urgency matrix (Urgent/Important combinations)
  - Process stages (Ideation → Design → Prototyping → Implementation → Testing → Refinement → Release)
  - Project grouping and tag-based categorisation
  - Due dates with flexible input formats
  - Issue and PR links for development workflow integration

- **Multiple Interfaces**:
  - Full-featured command-line interface for scripting and automation
  - Interactive terminal user interface (TUI) for visual task management
  - Web interface *(PLANNED)* for board status movements.

- **Flexible Querying and Filtering**:
  - Filter by status, kind, project, tags, due dates
  - Sort by due date, priority, or ID
  - Tree view for hierarchical relationships
  - Ancestor and descendant navigation

## Screenshots

### Main Menu
<img width="2560" height="1595" alt="image" src="https://github.com/user-attachments/assets/5cfbf035-3206-4c56-bce0-373b1fb58680" />

### Manage Workflow Queue
<img width="2556" height="1589" alt="image" src="https://github.com/user-attachments/assets/21756bd4-d027-4a73-bd22-d07968ee33ac" />

### Product-View
<img width="2560" height="1571" alt="image" src="https://github.com/user-attachments/assets/1e884f8d-ee99-403b-95b8-8610502bfeae" />

### Add Item
<img width="2559" height="1599" alt="image" src="https://github.com/user-attachments/assets/819062fb-376e-4079-870e-8aa0d82e23f6" />

### Hierarchical Views with Per-Level Colours
<img width="2560" height="1599" alt="image" src="https://github.com/user-attachments/assets/16c6cc32-2dc5-4203-b595-bbe73905e8ac" />


## Installation

### Prerequisites

- Rust 1.70+ (for building from source)

### Building from Source

```bash
git clone <repository-url>
cd pm
cargo build --release
```

The binary will be available at `target/release/pm`.

Alternatively, install it directly to ~/.cargo/bin with:
```bash
cargo install --path .
```

## Quick Start

### Basic Task Management

```bash
# Add a simple task
pm add "Implement user authentication"

# Add a task with metadata
pm add "Design login flow" \
  --desc "Create wireframes and user flow for login process" \
  --project "auth-system" \
  --tag "design,ux" \
  --due "2024-12-31" \
  --kind epic \
  --priority must-have \
  --urgency urgent-important

# List all open tasks
pm list

# List tasks in tree view
pm list --tree

# View detailed task information
pm view 1

# Update a task
pm update 1 --status in-progress --add-tags "frontend"

# Complete a task
pm complete 1

# Delete a task
pm delete 1
```

### Special Features

#### Smart Date Parsing
```bash
# Natural language dates
pm add "Weekend Task" --due "this weekend"
pm add "Sprint Review" --due "next friday"
pm add "Month End Report" --due "end of month"
pm add "Next Week Task" --due "in 1w"
pm add "Tomorrow's Meeting" --due "tomorrow"
```

#### Task Templates
```bash
# Create templates for reusable configurations
pm template create "Bug Fix" \
  --description "Standard bug fix template" \
  --priority must-have \
  --tags "bug,fix" \
  --process-stage implementation

# Use templates when creating tasks
pm add "Authentication Issue" --template "Bug Fix"

# Save existing tasks as templates
pm template save 42 "My Custom Template"

# Manage templates
pm template list
pm template delete "Old Template"
```

#### Bulk Operations
```bash
# Complete all tasks with specific tag
pm complete --tag "sprint-1"

# Delete all tasks in a project
pm delete --project "old-project"

# Complete all open tasks
pm complete --status open
```

#### CSV Export
```bash
# Export all tasks to CSV
pm export --all

# Export filtered tasks
pm export --tag "bug" --output "bug_report.csv"
pm export --project "web-app" --output "project_tasks.csv"
```

#### Shell Completions
```bash
# Generate completion scripts
pm completions bash > ~/.bash_completion.d/pm
pm completions zsh > ~/.zfunc/_pm
pm completions fish > ~/.config/fish/completions/pm.fish
```

### Hierarchical Organisation

```bash
# Create a product
pm add "E-commerce Platform" --kind product

# Create an epic under the product
pm add "User Management System" --kind epic --parent "E-commerce Platform"

# Create tasks under the epic
pm add "User Registration" --kind task --parent "User Management System"
pm add "User Login" --kind task --parent "User Management System"

# Create subtasks
pm add "Form Validation" --kind subtask --parent "User Registration"
pm add "Email Verification" --kind subtask --parent "User Registration"
```

### Terminal User Interface

Launch the interactive TUI:

```bash
pm ui
```

The Terminal UI (TUI) provides:
- Visual task browsing and editing
- Hierarchical navigation
- Form-based task creation and editing
- Help system with keyboard shortcuts

## Command Reference

### Global Options

- `--db <PATH>`: Specify custom database file location (default: `~/.pm/tasks.json`)

### Commands

#### `add` - Add a new task
```bash
pm add <TITLE> [OPTIONS]
```

**Options:**
- `--desc <TEXT>`: Longer description
- `--project <NAME>`: Project name
- `--tag <TAG>`: Tags (comma-separated, repeatable)
- `--due <DATE>`: Due date with smart parsing ("next friday", "end of week", "in 2w", "YYYY-MM-DD")
- `--parent <NAME>`: Parent task name (or ID for legacy compatibility)
- `--template <NAME>`: Use template for default values
- `--kind <KIND>`: Task kind (product, epic, task, subtask, milestone) [default: task]
- `--priority-level <LEVEL>`: Priority (must-have, nice-to-have, cut-first)
- `--urgency <LEVEL>`: Urgency (urgent-important, urgent-not-important, not-urgent-important, not-urgent-not-important)
- `--process-stage <STAGE>`: Process stage (ideation, design, prototyping, implementation, testing, refinement, release)
- `--status <STATUS>`: Initial status (open, in-progress, done) [default: open]
- `--summary <TEXT>`: One-line summary
- `--user-story <TEXT>`: User story
- `--requirements <TEXT>`: Requirements specification
- `--artifacts <FILES>`: Artifact file paths (comma-separated)
- `--issue-link <URL>`: Issue tracker URL
- `--pr-link <URL>`: Pull request URL

#### `list` - List tasks with filtering
```bash
pm list [OPTIONS]
```

**Options:**
- `--all`: Include completed tasks
- `--status <STATUS>`: Filter by status
- `--kind <KIND>`: Filter by kind
- `--project <PROJECT>`: Filter by project
- `--tag <TAG>`: Filter by tags (repeatable)
- `--due <FILTER>`: Due date filter (today, this-week, overdue, none)
- `--tree`: Display as hierarchical tree
- `--sort <KEY>`: Sort by (due, priority, id)
- `--limit <N>`: Limit number of results

#### `view` - View task details
```bash
pm view <ID_OR_NAME> [OPTIONS]
```

**Options:**
- `--children`: Show child tasks
- `--parents`: Show parent chain

#### `update` - Update task fields
```bash
pm update <ID_OR_NAME> [OPTIONS]
```

**Options:** (Same as `add` command, plus)
- `--add-tags <TAGS>`: Add tags
- `--rm-tags <TAGS>`: Remove tags
- `--clear-due`: Clear due date
- `--clear-parent`: Remove parent relationship

#### `complete` - Mark task as done
```bash
pm complete <ID_OR_NAME> [OPTIONS]
# OR bulk operations:
pm complete --tag <TAG>
pm complete --project <PROJECT>
pm complete --status <STATUS>
```

**Options:**
- `--recurse`: Also complete all descendant tasks

#### `reopen` - Reopen completed task
```bash
pm reopen <ID_OR_NAME>
```

#### `delete` - Delete task
```bash
pm delete <ID_OR_NAME> [OPTIONS]
# OR bulk operations:
pm delete --tag <TAG>
pm delete --project <PROJECT>
pm delete --status <STATUS>
```

**Options:**
- `--cascade`: Also delete all descendant tasks

#### `projects` - List all projects
```bash
pm projects
```

#### `tags` - List all tags with usage counts
```bash
pm tags
```

#### `ui` - Launch terminal user interface
```bash
pm ui
```

#### `template` - Manage task templates
```bash
pm template <ACTION>
```

**Actions:**
- `create <NAME> [OPTIONS]`: Create new template
- `save <TASK_ID> <NAME>`: Save existing task as template
- `list`: List all templates
- `delete <NAME>`: Delete a template

#### `export` - Export tasks to CSV
```bash
pm export [OPTIONS]
```

**Options:**
- `-o, --output <FILE>`: Output file (default: tasks.csv)
- `--all`: Include completed tasks
- `--project <PROJECT>`: Filter by project
- `--tag <TAG>`: Filter by tag

#### `completions` - Generate shell completions
```bash
pm completions <SHELL>
```

**Shells:** bash, zsh, fish

## Data Storage

Tasks are stored in a JSON file at `~/.pm/tasks.json` by default. You can specify a custom location using the `--db` option.

The database format is human-readable JSON, making it easy to backup, sync, or integrate with other tools.

## Hierarchy Rules

The tool enforces these hierarchical relationships:
- **Product** can contain **Epic**
- **Epic** can contain **Task**
- **Task** can contain **Subtask**
- **Subtask** can contain **Subtask** (nested subtasks)
- **Milestone** can be attached to any level

Reason:
At a personal level I have found this structure supports effective management
of large-scale software projects, without being confronted with a sea of low-level task details prematurely.

## Development Process Integration

PM is designed to integrate with modern development workflows:

- **Process Stages**: Track items through ideation → design → implementation → release
- **Issue/PR Links**: Connect tasks to external tracking systems
- **Artifacts**: Track associated files and deliverables
- **Requirements**: Document formal specifications
- **User Stories**: Capture user-focused requirements

## Examples

### Software Development Project

```bash
# Create product
pm add "Task Management App" --kind product

# Create epics
pm add "Core Task System" --kind epic --parent "Task Management App"
pm add "User Interface" --kind epic --parent "Task Management App"

# Create tasks
pm add "Task CRUD Operations" --kind task --parent "Core Task System" --process-stage implementation
pm add "CLI Interface" --kind task --parent "User Interface" --process-stage design

# Create subtasks with development metadata
pm add "Add Task Command" --kind subtask --parent "Task CRUD Operations" \
  --desc "Implement the add command with validation" \
  --process-stage implementation \
  --issue-link "https://github.com/user/repo/issues/42" \
  --priority must-have
```

### Project Planning with Dates

```bash
# Sprint planning
pm add "Sprint 1 Planning" --kind milestone --due "today"
pm add "User Registration" --kind task --due "in 7d" --priority must-have
pm add "User Login" --kind task --due "in 14d" --priority must-have

# View sprint work
pm list --due this-week --tree
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass: `cargo test`
5. Submit a pull request

## Getting Started - UI

### Understanding the Hierarchy

The TUI is built around a four-level hierarchy designed to manage complex projects from high-level vision down to implementation details:

- **Product**: The overall deliverable or system (e.g., "E-commerce Platform")
- **Epic**: Major features or components (e.g., "User Management System", "Payment Processing")
- **Task**: Specific work items (e.g., "User Registration Form", "Payment Gateway Integration")
- **Subtask**: Implementation details (e.g., "Email Validation", "API Error Handling")

This structure prevents cognitive overload by allowing you to focus on the appropriate level of abstraction for your current context.
Additionally, it enables farming low-level (the low-complexity, time-consuming) work out to LLM's such as Claude Code, within manageable system boundaries.

### Navigation Controls

**Basic Navigation:**
- `LEFT/RIGHT`: Navigate between hierarchy levels (Product ↔ Epic ↔ Task ↔ Subtask)
- `UP/DOWN`: Move within the current list
- `ENTER`: Select/drill down into items
- `ESC`: Go back/up one level

**Advanced Navigation:**
- `SHIFT+LEFT/SHIFT+RIGHT`: Move items between hierarchy levels (promote/demote)
- `TAB`: Switch between different views (List, Tree, Calendar, etc.)
- `?`: Help system with full keyboard shortcuts

The LEFT/RIGHT movement lets you "zoom out" to see the big picture or "zoom in" to focus on specific implementation details, while SHIFT+LEFT/RIGHT actually restructures your project hierarchy.

## Reason This Exists

Most terminal-based project management tools fall into two categories:
1. **Simple todo lists** that lack hierarchical organisation, or time-based task-only pomodoro counters
2. **Team collaboration platforms** (Jira, Asana, etc.) that are web-based and designed for multiple stakeholders

There are few high-quality options for **individual developers** who need:
- **Terminal-native workflow** that integrates with their development environment
- **Hierarchical thinking** to manage complex projects from vision to implementation
- **Local control** without web tool overhead or forced collaboration features
- **Rapid capture** of ideas without context switching to browser tabs

PM fills this gap by providing a personal project management system that scales from quick task capture to complex multi-epic software projects, all while staying in your terminal where development work happens.

## Roadmap

The following are potential future enhancements:
1. GIT issue integration (likely)
2. Webbased view - simply for rapid status changing on a simple board.

All free, and user-local, and development will likely stop there, rather than overcooking things,
so that setup remains zero-config and ready-to-go.

## License

MIT Licensed. See LICENSE for details.

---
