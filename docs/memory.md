# Memory

PM organises remembered context into three tiers. The same memory entry can live at exactly one tier at a time; promotion and demotion move it between them.

## Tiers

| Tier | Location | Owner |
|---|---|---|
| User | `~/.claude/projects/<encoded-cwd>/memory/` | Claude Code's auto-memory store. PM reads but does not write. |
| Project | `<.pm-root>/projects/<PRJ>/memories/` | PM. Committed alongside the project. |
| Ticket | `<ticket-dir>/memories/` | PM. Committed alongside the ticket. |

`<encoded-cwd>` maps every `/` in the working-directory path to `-`. For `/home/pbow/pm` this is `-home-pbow-pm`.

## Memory file format

Each memory is a markdown file with YAML front-matter:

```markdown
---
name: feedback-testing
description: Tests must use a real database connection, not a mock.
metadata:
  type: feedback
---

Long-form notes. Whatever the agent needs to remember.
```

`metadata.type` is one of `user`, `feedback`, `project`, `reference`. The shape mirrors Claude Code's auto-memory format.

## CLI

```bash
pm memory list <id>                                        # show memories linked to a ticket
pm memory write --scope project --type feedback --name <name> "body text"
pm memory write --scope ticket --ticket <id> --type reference --name <name> "body"
pm memory link <id> <name>                                 # link an existing memory to a ticket
pm memory unlink <id> <name>
pm memory promote <name>                                   # user <-> project promotion
pm memory show <name>
```

`--scope user` is rejected. PM never writes the user tier directly; promotion is the one indirect path that writes a back-reference (see below).

## Linking

`pm memory link TSK7 feedback-testing` resolves the name across tiers (project first, then ticket, then user) and records a `MemoryRef` variant in TSK7's CLAUDE.md front-matter:

```yaml
memories:
  - User: feedback-testing
  - Project: auth-stack-conventions
```

The variant carries the tier. `pm memory list TSK7` reads from front-matter directly; it does not re-resolve.

## Collision rules

If the same name exists at multiple tiers, `lookup_by_name` returns the project-tier hit first, then ticket, then user.

`pm context <id>` annotates each linked memory in the composed view with its tier tag:

```markdown
## Linked memories (TSK7)

### feedback-testing [User]
> Tests must use a real database connection, not a mock.

<body>

@/home/pbow/.claude/projects/-home-pbow-pm/memory/feedback-testing.md
```

The `@-import` line uses an absolute path so Claude Code's loader resolves it regardless of cwd.

## Promote and demote

```bash
pm memory promote <name>
```

Defaults to promoting from user tier to project tier:

1. Copy the file from `~/.claude/projects/.../memory/<name>.md` to `<.pm-root>/projects/<PRJ>/memories/<name>.md`.
2. Write a small back-reference at the user tier:

   ```markdown
   ---
   name: <name>
   description: Promoted to project tier.
   metadata:
     type: reference
   ---

   This memory was promoted to project tier on 2026-05-15.

   Canonical: <.pm-root>/projects/<PRJ>/memories/<name>.md
   ```

This back-reference is the only path by which PM writes the user tier.

Ticket-tier promotion is unsupported in v1; the verb returns `UnsupportedPromotion`.

To demote (project to user), pass `--to user`. PM writes the user-tier file (using the same back-reference rule does not apply; the original moves) and removes the project-tier file.

## Composed view toggle

```bash
pm context TSK7              # includes Linked memories section
pm context TSK7 --no-memories
```

The MCP `read_context` tool accepts the same toggle as `no_memories: true` in its arguments.

## See also

- [file-layout.md](file-layout.md) - where memories live on disk.
- [mcp.md](mcp.md) - `read_memories` and `write_memory` tools.
