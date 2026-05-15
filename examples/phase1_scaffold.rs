//! End-to-end exercise of the Phase 1 store module.
//!
//! Builds a `.pm/` layout, allocates a few typed ids, ensures their on-disk
//! directories, writes state.json + aliases.json, and round-trips through
//! load. Optionally runs the migration planner against a supplied tasks.json
//! produced by the existing `Database` format.
//!
//! Usage:
//!     cargo run --example phase1_scaffold -- <pm-base-dir> [--migrate <tasks.json>]
//!
//! `<pm-base-dir>` is the directory under which `.pm/` will be created.

use std::path::PathBuf;
use std::process::ExitCode;

use project_management::store::{
    AddressId, Aliases, ItemEntry, Layout, MigrationPlan, State, TypePrefix,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <pm-base-dir> [--migrate <tasks.json>]", args[0]);
        return ExitCode::from(2);
    }
    let base = PathBuf::from(&args[1]);
    let migrate_arg = args
        .iter()
        .position(|a| a == "--migrate")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let layout = Layout::under(&base);
    if let Err(e) = layout.init() {
        eprintln!("layout init failed: {e}");
        return ExitCode::FAILURE;
    }
    println!("initialised layout at {}", layout.root.display());

    // Allocate a small representative tree.
    let mut state = State::fresh();
    let prj = state.allocate(TypePrefix::Project);
    let prd = state.allocate(TypePrefix::Product);
    let epc = state.allocate(TypePrefix::Epic);
    let tsk = state.allocate(TypePrefix::Task);
    let sbt = state.allocate(TypePrefix::Subtask);
    let mls = state.allocate(TypePrefix::Milestone);

    let address = AddressId::new(vec![prj, prd, epc, tsk, sbt]).unwrap();
    let task_dir = layout.directory_for(&address);
    let mls_dir = layout.directory_for(&AddressId::new(vec![prj, mls]).unwrap());

    let prj_dir = layout.directory_for(&AddressId::new(vec![prj]).unwrap());
    let prd_dir = layout.directory_for(&AddressId::new(vec![prj, prd]).unwrap());
    let epc_dir = layout.directory_for(&AddressId::new(vec![prj, prd, epc]).unwrap());
    let tsk_dir = layout.directory_for(&AddressId::new(vec![prj, prd, epc, tsk]).unwrap());

    for (leaf, rel) in [
        (prj, prj_dir),
        (prd, prd_dir),
        (epc, epc_dir),
        (tsk, tsk_dir),
        (sbt, task_dir.clone()),
        (mls, mls_dir),
    ] {
        let rel: PathBuf = rel;
        if let Err(e) = layout.ensure_node_path(&rel) {
            eprintln!("ensure_node_path({}) failed: {e}", rel.display());
            return ExitCode::FAILURE;
        }
        state.insert(leaf, ItemEntry { path: rel });
    }

    // Tombstone an extra task to demonstrate the skip behaviour on next alloc.
    let _ghost = state.allocate(TypePrefix::Task);
    state.tombstone(_ghost);

    // Aliases: record a moved-task example for review.
    let mut aliases = Aliases::empty();
    aliases.add(
        format!("{prj}-{prd}-{epc}-{tsk}"),
        format!("{prj}-{prd}-EPC9-{tsk}"),
    );

    if let Err(e) = state.save(&layout.state_path()) {
        eprintln!("state.save failed: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = aliases.save(&layout.aliases_path()) {
        eprintln!("aliases.save failed: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote state.json with {} items", state.items.len());
    println!("wrote aliases.json with {} entries", aliases.map.len());

    // Round-trip read.
    let loaded = State::load(&layout.state_path()).expect("state load");
    assert_eq!(loaded.items.len(), state.items.len());
    println!("round-trip read: ok");
    println!();
    println!("Allocated ids:");
    for (id, entry) in &loaded.items {
        println!("  {}  -> {}", id, entry.path.display());
    }

    if let Some(source_path) = migrate_arg {
        println!();
        match MigrationPlan::plan(&layout, &source_path) {
            Ok(plan) => println!("{}", plan.render()),
            Err(e) => {
                eprintln!("migration plan failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}
