[33mcommit 28203dc8f040cb97e6c4838571e7614bc1434295[m[33m ([m[1;36mHEAD[m[33m -> [m[1;32mphase-2-claude-md[m[33m)[m
Author: PB <37089506+pbower@users.noreply.github.com>
Date:   Wed May 13 00:52:58 2026 +0100

    [phase-2] Add phase2_scaffold example exercising the full ticket surface
    
    End-to-end demo that builds on phase1_scaffold by writing real
    CLAUDE.md files for every allocated ticket using the per-kind built-in
    template via templates::resolve(). Demonstrates:
    
    - Layout::init + State allocation (Phase 1)
    - Ticket::scaffold + Ticket::write_to for each kind (PRJ/PRD/EPC/TSK/
      SBT/MLS + an orphan TSK)
    - Ticket::read + upsert_section + re-write round-trip
    - Ticket::apply_template upgrade flow
    
    Verified by running against /tmp/pm-phase2-demo: produces a complete
    .pm/ tree with valid CLAUDE.md files at every node, correct templates
    per kind, and round-trips arbitrary edits through the read/write path.
    
    Phase 2 task 19 of PM_BUILD_PLAN.md.

[33mcommit 4628557963934437674adc5f17b5746dc107c60b[m
Author: PB <37089506+pbower@users.noreply.github.com>
Date:   Wed May 13 00:51:53 2026 +0100

    [phase-2] Add store::claude_md - Ticket orchestrator for full files
    
    Ticket struct ties FrontMatter + ParsedBody + per-kind template into a
    single public API the rest of the crate uses:
    
    - Ticket::scaffold(leaf, title, slug, template) builds a fresh ticket
      with the template applied to an empty body. Front-matter created/
      updated timestamps initialised to Utc::now().
    - Ticket::read(path) parses a CLAUDE.md from disk: front-matter via
      Document::parse, then ParsedBody::parse on the body with the trailing
      @-import stripped.
    - Ticket::render() emits a complete CLAUDE.md-shaped string with the
      YAML delimiters and the `@artifacts/ARTIFACTS.md` trailer line.
    - Ticket::write_to(dir) does an atomic temp+rename write, bumping the
      front-matter `updated` timestamp.
    - Ticket::upsert_section(name, body) routes through ParsedBody::upsert.
    - Ticket::apply_template(template) re-applies a per-kind template
      without overwriting matching-section content or dropping user-added
      sections.
    
    CLAUDE_MD and ARTIFACTS_IMPORT exposed as crate-level constants.
    
    11 unit tests cover all PM_DESIGN.md Section 6.5 fixtures:
    - Blank ticket (all template sections, empty bodies)
    - Fully-filled ticket round-tripping through disk
    - User-added section surviving a write/read cycle
    - Reordered sections (Notes-first) preserved across round-trip
    - Deleted section staying deleted on read
    - apply_template restoring a deleted section with empty body
    - Content preservation in matching sections after template upgrade
      (Subtask -> Task) while keeping user-added Field Notes
    
    Plus the orchestrator-level tests: scaffold output shape, render
    trailer placement, upsert preserving other sections via disk.
    
    Phase 2 task 18 of PM_BUILD_PLAN.md.

[33mcommit 866b6d8b11aacc7a821fc132eb0d2cf2150a4ed3[m
Author: PB <37089506+pbower@users.noreply.github.com>
Date:   Wed May 13 00:49:24 2026 +0100

    [phase-2] Add store::templates - per-kind section templates
    
    Six built-in templates (project / product / epic / task / subtask /
    milestone) compiled into the binary via include_str! against tiny
    markdown files in src/store/templates/. Each template is just the
    section headings with no body content.
    
    resolve(prefix, pm_root, home_dir) walks the override chain:
      1. <pm_root>/templates/<kind>.md  (project-local override)
      2. <home>/.pm-templates/<kind>.md (user-global override)
      3. Built-in compiled-in default.
    Returns a ResolvedTemplate carrying the content plus a TemplateSource
    tag so callers (pm template edit) can tell where it came from.
    
    apply(template, &mut ParsedBody) is the round-trip-safe combinator:
      - For each section in the template, preserve any existing content
        under that name; otherwise add an empty-bodied section.
      - User-added sections (not in the template) are kept and appended
        after the templated sections in their original order. Never
        silently dropped.
      - Idempotent on already-templated bodies.
    
    scaffold(template) is the convenience for new tickets: empty body
    plus apply(template).
    
    8 unit tests cover all six built-ins, scaffold ordering, content
    preservation, user-added survival, idempotency, override resolution
    (project beats user beats built-in).
    
    Phase 2 task 17 of PM_BUILD_PLAN.md.

[33mcommit bf2eb2921344c4140ca02dba872db202fd5a1a57[m
Author: PB <37089506+pbower@users.noreply.github.com>
Date:   Wed May 13 00:48:08 2026 +0100

    [phase-2] Add store::sections - markdown section parser and writer
    
    ParsedBody splits a CLAUDE.md body into preamble + Vec<Section>. Each
    Section has a name (h1 heading text) and a body (content between this
    heading and the next, or EOF). Designed for the form-to-section
    round-trip the TUI quick-entry flow needs:
    
    - Level 1 only: ## and deeper are body content of the enclosing
      section, never new sections.
    - Fenced code blocks (``` and ~~~) are honoured; headings inside them
      never become section starts.
    - Preamble (content above the first h1) is preserved.
    - Section order is preserved including user-added custom sections.
    - ATX-style trailing hashes (`# Title #`) trimmed on parse.
    
    Mutators:
    - find / find_mut by exact name (case-sensitive).
    - upsert(name, body) replaces or appends. Returns whether a replace.
    - remove(name) drops by name.
    - names() returns ordered slice for diffing.
    
    Inter-section blank lines are stripped from the previous section's
    body on close so the gap is purely presentational. Render reproduces a
    single blank line between sections; empty bodies do not produce a
    phantom newline. Result: parse -> render -> parse is idempotent.
    
    17 unit tests cover empty/preamble-only/single/multi parsing, h2+
    ignored, code-fence handling, user-added section round-trip, upsert /
    remove, empty-body sections, preamble survival, ATX trailing-hash
    trimming, tilde fences.
    
    Phase 2 task 16 of PM_BUILD_PLAN.md.

[33mcommit 2fd4f5348628727881582af1242e08039efbf16d[m
Author: PB <37089506+pbower@users.noreply.github.com>
Date:   Wed May 13 00:45:16 2026 +0100

    [phase-2] Add store::front_matter - typed YAML front-matter for CLAUDE.md
    
    FrontMatter struct mirrors PM_DESIGN.md Section 5.4: id (required),
    parent, project, title, slug, status (defaults to open), priority,
    urgency, process_stage, due, tags, deps, milestone, memories, links,
    created, updated. Reuses existing Priority/Urgency/ProcessStage/Status
    enums from fields.rs.
    
    Document struct holds a parsed (FrontMatter, body) pair and renders
    back to a complete CLAUDE.md-shaped string. split_front_matter is the
    free function that extracts the YAML block delimited by --- lines,
    strips a UTF-8 BOM and an optional leading newline, and emits a clean
    body without the stray newline after the closing delimiter.
    
    MemoryRef enum (User/Project/Ticket) serialises as a single-key map so
    front-matter reads naturally: `- user: feedback-testing`.
    
    Errors: MissingOpenDelimiter, MissingCloseDelimiter, Yaml, Io. Each
    maps to a specific YAML or markdown-structure failure mode.
    
    serde_yml 0.0.12 chosen over the deprecated serde_yaml 0.9. Same API
    surface, actively maintained fork. 11 unit tests cover roundtrips
    (minimal and full), default backfill, missing-id rejection, malformed
    YAML rejection, delimiter handling, body-less rendering.
