# Templates

PM has two related template systems:

1. **Section templates** define the default body shape of a CLAUDE.md per ticket kind. Built into the binary; overridable per workspace.
2. **TaskTemplate presets** are saved field-default bundles - "a Subtask for the auth area, status InProgress, priority MustHave". Created from the TUI quick-entry flow.

Both live under the umbrella term "templates" but answer different questions: section templates shape the prose; presets supply quick-create defaults for the metadata.

## Section templates

When PM scaffolds a new ticket's CLAUDE.md, the body is built from a per-kind template - section headings with empty bodies. The user fills in the sections via the TUI or `$EDITOR`.

| Kind | Default sections |
|---|---|
| `Project` | `Description`, `Goals`, `Stakeholders`, `Notes` |
| `Product` | `Description`, `Vision`, `Roadmap`, `Notes` |
| `Epic` | `Description`, `User Story`, `Acceptance Criteria`, `Notes` |
| `Task` | `Description`, `User Story`, `Requirements`, `Acceptance Criteria`, `Notes` |
| `Subtask` | `Description`, `Acceptance Criteria`, `Notes` |
| `Milestone` | `Description`, `Definition of Done`, `Notes` |

### Override locations

Resolution order, first hit wins:

1. `<.pm-root>/templates/<kind>.md`
2. `~/.pm-templates/<kind>.md`
3. Built-in (compiled into the binary).

`<kind>` is the lowercase kind name: `project`, `product`, `epic`, `task`, `subtask`, `milestone`.

### Template file format

A template file is a CLAUDE.md body fragment - section headings with no front-matter and no body content:

```markdown
# Description

# Implementation Plan

# Risks

# Notes
```

When a ticket is created, this content is dropped into the body verbatim; sections fill in from there.

### Editing templates

```bash
pm template edit task              # opens task.md in $EDITOR
pm template edit project           # opens project.md
```

The first edit creates `<.pm-root>/templates/<kind>.md` if it does not exist, seeded from the built-in template.

### Applying a template to an existing ticket

```bash
pm template apply TSK7
```

Adds any missing sections from the current template to the ticket's CLAUDE.md. Existing content is preserved; sections present in the ticket but absent from the template are kept. Sections present in the template but absent from the ticket are appended with empty bodies.

## TaskTemplate presets (TUI quick-entry)

The retro-style TUI carries a separate "presets" concept used by the quick-create flow. A preset bundles:

- Default kind (`Project`, `Product`, `Epic`, `Task`, `Subtask`, `Milestone`)
- Default status, priority, urgency, process stage
- Default project label, tags, description fragment

Presets are stored in `state.json` under the `templates` field. They survive `pm doctor` rebuilds because they are part of `state.json` itself.

### CLI

```bash
pm template list
pm template save <task-id> <preset-name>
pm template create <preset-name> --kind subtask --priority must-have --tags backend,auth
pm template delete <preset-name>
```

The TUI quick-entry form (`f` key in Mode 1) lists saved presets and applies them to the new ticket as starting defaults.

## See also

- [file-layout.md](file-layout.md) - where templates live on disk.
- [mcp.md](mcp.md) - section templates surface through the `write_doc` tool's section-name argument.
