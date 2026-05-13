//! End-to-end exercise of the Phase 3 artifact surfaces.
//!
//! Builds a ticket, starts the auto-sweep watcher, drops a couple of files
//! into the ticket's `artifacts/` directory, demonstrates that hand-edited
//! descriptions survive a sweep, and renames an artifact to show the
//! description carries over.
//!
//! Usage:
//!     cargo run --example phase3_artifacts -- <pm-base-dir>

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use project_management::store::{
    artifacts::{rename_artifact, ArtifactsIndex, ARTIFACTS_MD},
    templates::builtin,
    ArtifactsWatcher, ItemEntry, Layout, State, Ticket, TypePrefix,
};

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
    let prj = state.allocate(TypePrefix::Project);
    let prd = state.allocate(TypePrefix::Product);
    let epc = state.allocate(TypePrefix::Epic);
    let tsk = state.allocate(TypePrefix::Task);

    let task_dir_rel = PathBuf::from(
        "projects/pm/products/core/epics/checkouts/tasks/lock-protocol",
    );
    if let Err(e) = layout.ensure_node_path(&task_dir_rel) {
        eprintln!("ensure_node_path: {e}");
        return ExitCode::FAILURE;
    }
    state.insert(prj, ItemEntry { path: PathBuf::from("projects/pm") });
    state.insert(prd, ItemEntry { path: PathBuf::from("projects/pm/products/core") });
    state.insert(epc, ItemEntry { path: PathBuf::from("projects/pm/products/core/epics/checkouts") });
    state.insert(tsk, ItemEntry { path: task_dir_rel.clone() });

    let ticket = Ticket::scaffold(tsk, "Lock protocol", "lock-protocol",
        builtin(TypePrefix::Task));
    let task_dir = layout.root.join(&task_dir_rel);
    if let Err(e) = ticket.write_to(&task_dir) {
        eprintln!("write ticket: {e}");
        return ExitCode::FAILURE;
    }
    println!("scaffolded TSK1 at {}", task_dir.display());

    let artifacts_dir = task_dir.join("artifacts");
    let mut watcher = ArtifactsWatcher::new().expect("create watcher");
    watcher.watch(&artifacts_dir, tsk, Some("lock-protocol".to_string()))
        .expect("watch artifacts");
    println!("watching {}", artifacts_dir.display());

    // Drop two files; the watcher should sweep and update ARTIFACTS.md.
    std::fs::write(artifacts_dir.join("schema.png"), b"PNG").unwrap();
    std::fs::write(artifacts_dir.join("bench.csv"), b"a,b\n").unwrap();
    std::thread::sleep(Duration::from_millis(450));

    let index_path = artifacts_dir.join(ARTIFACTS_MD);
    print_index(&index_path, "after drop");

    // Hand-edit a description on schema.png, then drop another file; the
    // description should survive the sweep triggered by the third file.
    let mut idx = ArtifactsIndex::load(&index_path).unwrap();
    idx.find_mut("schema.png").unwrap().desc =
        "ER diagram of the lock-file format".into();
    idx.find_mut("schema.png").unwrap().tags = vec!["reference".into(), "design".into()];
    idx.save(&index_path, Some("lock-protocol")).unwrap();
    std::fs::write(artifacts_dir.join("notes.md"), b"# scratch\n").unwrap();
    std::thread::sleep(Duration::from_millis(450));

    print_index(&index_path, "after hand-edit + new file");

    // Rename schema.png -> er-diagram.png. Description must carry over.
    rename_artifact(&artifacts_dir, "schema.png", "er-diagram.png", Some("lock-protocol"))
        .expect("rename artifact");
    std::thread::sleep(Duration::from_millis(450));

    print_index(&index_path, "after rename");

    // Drop a file from disk to demonstrate removal detection.
    std::fs::remove_file(artifacts_dir.join("bench.csv")).ok();
    std::thread::sleep(Duration::from_millis(450));

    print_index(&index_path, "after deletion");

    drop(watcher);
    println!("watcher dropped; sweep stops.");
    ExitCode::SUCCESS
}

fn print_index(index_path: &Path, label: &str) {
    println!("\n--- ARTIFACTS.md ({}) ---", label);
    match std::fs::read_to_string(index_path) {
        Ok(raw) => println!("{raw}"),
        Err(e) => println!("(read failed: {e})"),
    }
}
