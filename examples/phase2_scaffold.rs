//! End-to-end exercise of the Phase 2 surfaces.
//!
//! Builds on `phase1_scaffold` by additionally writing real `CLAUDE.md` files
//! for every allocated ticket, using the per-kind built-in section template.
//! Demonstrates:
//!
//! - Layout::init (Phase 1)
//! - State + ItemEntry allocation (Phase 1)
//! - Ticket::scaffold + Ticket::write_to (Phase 2)
//! - Ticket::read + upsert_section + re-write (Phase 2)
//! - Ticket::apply_template upgrade flow (Phase 2)
//!
//! Usage:
//!     cargo run --example phase2_scaffold -- <pm-base-dir>
//!
//! `<pm-base-dir>` is the directory under which `.pm/` will be created.

use std::path::PathBuf;
use std::process::ExitCode;

use project_management::store::templates::{builtin, resolve};
use project_management::store::{AddressId, ItemEntry, Layout, State, Ticket, TypePrefix};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <pm-base-dir>", args[0]);
        return ExitCode::from(2);
    }
    let base = PathBuf::from(&args[1]);

    let layout = Layout::under(&base);
    if let Err(e) = layout.init() {
        eprintln!("layout init failed: {e}");
        return ExitCode::FAILURE;
    }

    let mut state = State::fresh();

    // Allocate a complete chain plus an orphan task and a milestone.
    let prj = state.allocate(TypePrefix::Project);
    let prd = state.allocate(TypePrefix::Product);
    let epc = state.allocate(TypePrefix::Epic);
    let tsk = state.allocate(TypePrefix::Task);
    let sbt = state.allocate(TypePrefix::Subtask);
    let mls = state.allocate(TypePrefix::Milestone);
    let orphan = state.allocate(TypePrefix::Task);

    let address = AddressId::new(vec![prj, prd, epc, tsk, sbt]).unwrap();
    let subtask_dir = layout.directory_for(&address);
    let mls_dir = layout.directory_for(&AddressId::new(vec![prj, mls]).unwrap());
    let orphan_dir = layout.orphan_directory_for(orphan);

    let prj_dir = layout.directory_for(&AddressId::new(vec![prj]).unwrap());
    let prd_dir = layout.directory_for(&AddressId::new(vec![prj, prd]).unwrap());
    let epc_dir = layout.directory_for(&AddressId::new(vec![prj, prd, epc]).unwrap());
    let tsk_dir = layout.directory_for(&AddressId::new(vec![prj, prd, epc, tsk]).unwrap());

    // Map (kind, leaf, title, directory) so we can both write CLAUDE.md and
    // populate state.items in one pass.
    let placements: Vec<(TypePrefix, project_management::store::LeafId, &str, PathBuf)> = vec![
        (TypePrefix::Project, prj, "pm", prj_dir),
        (TypePrefix::Product, prd, "core", prd_dir),
        (TypePrefix::Epic, epc, "Checkout protocol", epc_dir),
        (
            TypePrefix::Task,
            tsk,
            "Lock protocol with TTL and heartbeat",
            tsk_dir,
        ),
        (
            TypePrefix::Subtask,
            sbt,
            "Stale-lock cleanup on missed heartbeat",
            subtask_dir,
        ),
        (TypePrefix::Milestone, mls, "v1.0 release", mls_dir),
        (
            TypePrefix::Task,
            orphan,
            "A standalone task with no product",
            orphan_dir,
        ),
    ];

    for (prefix, leaf, title, rel) in &placements {
        if let Err(e) = layout.ensure_node_path(rel) {
            eprintln!("ensure_node_path({}) failed: {e}", rel.display());
            return ExitCode::FAILURE;
        }
        state.insert(*leaf, ItemEntry { path: rel.clone() });

        let resolved = resolve(*prefix, &layout.root, dirs_home().as_deref());
        let ticket = Ticket::scaffold(*leaf, *title, &resolved.content);
        let abs_dir = layout.root.join(rel);
        match ticket.write_to(&abs_dir) {
            Ok(p) => println!("wrote {} ({})", p.display(), leaf),
            Err(e) => {
                eprintln!("write failed for {leaf}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    if let Err(e) = state.save(&layout.state_path()) {
        eprintln!("state.save failed: {e}");
        return ExitCode::FAILURE;
    }

    // Demonstrate upsert + read-back: write a real description into the
    // lock-protocol task, then read it back to confirm round-trip.
    let tsk_path = layout
        .root
        .join("projects/PRJ1/products/PRD1/epics/EPC1/tasks/TSK1/CLAUDE.md");
    let mut reloaded = Ticket::read(&tsk_path).expect("read task");
    reloaded.upsert_section(
        "Description",
        "Each agent gets an exclusive lock per ticket. Locks carry a TTL and\nheartbeat so a crashed agent does not hold a ticket indefinitely.\n",
    );
    reloaded.upsert_section(
        "Acceptance Criteria",
        "- TTL expires within 60s of last heartbeat.\n- Cleanup runs on `pm doctor`.\n",
    );
    reloaded
        .write_to(tsk_path.parent().unwrap())
        .expect("rewrite task");

    let final_state = Ticket::read(&tsk_path).expect("final read");
    println!();
    println!("--- Final TSK1 CLAUDE.md ---");
    println!("{}", final_state.render().expect("render final"));

    // Demonstrate the template-application flow: re-apply the Task template
    // against the orphan task (already applied, but exercises the path).
    let orphan_path = layout.root.join("tasks/TSK2/CLAUDE.md");
    let mut orphan_t = Ticket::read(&orphan_path).expect("read orphan");
    orphan_t.apply_template(builtin(TypePrefix::Task));
    orphan_t
        .write_to(orphan_path.parent().unwrap())
        .expect("write orphan");

    println!("--- Orphan TSK2 CLAUDE.md ---");
    let orphan_final = Ticket::read(&orphan_path).expect("orphan final read");
    println!("{}", orphan_final.render().expect("render orphan"));

    ExitCode::SUCCESS
}

/// Resolve the user's home directory if available; tests and headless
/// environments may have `HOME` unset.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
