//! v0.3.4 acceptance tests: the bottom activity strip's live refresh
//! and state-change ticker.
//!
//! These exercise [`ActivityStrip::poll`] directly so we get a real
//! `notify-debouncer-mini` watcher in the loop without needing a TTY.
//! The render path stays untested at this layer because it depends on
//! a terminal backend; its content is the same `ActivityView` buffer
//! the v0.3.1 path already covers.

use std::fs;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use project_management::store::events::emit_event;
use project_management::store::id::{LeafId, TypePrefix};
use project_management::store::layout::Layout as StoreLayout;
use project_management::tui::activity::ActivityStrip;

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-v034-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_workspace(label: &str) -> PathBuf {
    let dir = tmp_dir(label);
    let layout = StoreLayout::at(&dir);
    layout.init().expect("init workspace");
    dir
}

#[test]
fn first_poll_after_init_returns_false_when_no_external_writes() {
    let dir = init_workspace("idle");
    // Seed events.log so the watcher has something to attach to.
    emit_event(&dir, "init", None, None).unwrap();
    let mut strip = ActivityStrip::new(dir);

    // No external mutation between `new` (which loaded the buffer) and
    // this first poll; the result must be `false` so the shell does not
    // reload Database for nothing.
    assert!(!strip.poll(), "idle poll should not signal a refresh");
}

#[test]
fn poll_returns_true_after_external_append() {
    let dir = init_workspace("append");
    emit_event(&dir, "init", None, None).unwrap();
    let mut strip = ActivityStrip::new(dir.clone());

    // Drain any initial signal so the next poll reflects only the
    // external write.
    let _ = strip.poll();

    let leaf = LeafId::new(TypePrefix::Task, 1);
    emit_event(&dir, "status", Some(leaf), Some("in-progress")).unwrap();

    // Give the debouncer a window plus headroom to fire.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if strip.poll() {
            return;
        }
        sleep(Duration::from_millis(50));
    }
    panic!("watcher never signalled within 2s of an external append");
}

#[test]
fn poll_falls_back_to_throttle_when_watcher_cannot_start() {
    // Pointing the strip at a directory with no events.log forces the
    // start_watcher path to return None and the strip into pure
    // polling mode. The strip must still keep working.
    let dir = tmp_dir("no-events-log");
    let mut strip = ActivityStrip::new(dir.clone());

    // Without an events.log there is nothing to refresh; the strip
    // tolerates the missing file silently. A poll cannot signal a
    // length change because there is no buffer growth, so it returns
    // false.
    assert!(!strip.poll(), "missing events.log poll should be false");

    // Create the file mid-flight and append. The fallback poll picks
    // it up on the next tick after the 500ms throttle elapses.
    let layout = StoreLayout::at(&dir);
    layout.init().unwrap();
    let leaf = LeafId::new(TypePrefix::Task, 7);
    emit_event(&dir, "status", Some(leaf), None).unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut saw_signal = false;
    while std::time::Instant::now() < deadline {
        if strip.poll() {
            saw_signal = true;
            break;
        }
        sleep(Duration::from_millis(100));
    }
    assert!(
        saw_signal,
        "fallback poll should pick up the append once events.log exists"
    );
}
