//! Phase 11 acceptance tests: drive `pm mcp` as a subprocess.
//!
//! Each test spawns the compiled binary, pipes a sequence of JSON-RPC
//! requests on its stdin, closes stdin (which makes the read loop exit),
//! and parses the responses from stdout. This covers the wire format
//! end-to-end without relying on a real MCP client.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pm-phase11-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `pm --db <pm_dir> <args>` non-interactively for setup. Used to
/// seed the workspace before driving `pm mcp`.
fn pm(pm_dir: &Path, args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_pm");
    let mut cmd = Command::new(bin);
    cmd.arg("--db").arg(pm_dir).args(args);
    cmd.output().expect("invoke pm")
}

fn assert_ok(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed: status={:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Spawn `pm --db <pm_dir> mcp`, write `lines` to its stdin (one request
/// per line), close stdin, and parse the responses from stdout. Each
/// response is one JSON value on its own line.
fn drive_mcp(pm_dir: &Path, lines: &[&str]) -> Vec<Value> {
    let bin = env!("CARGO_BIN_EXE_pm");
    let mut child = Command::new(bin)
        .arg("--db")
        .arg(pm_dir)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pm mcp");

    {
        let stdin = child.stdin.as_mut().expect("mcp stdin");
        for line in lines {
            stdin.write_all(line.as_bytes()).expect("write request");
            stdin.write_all(b"\n").expect("newline");
        }
    }
    // Drop stdin so the server's read loop sees EOF and exits.
    drop(child.stdin.take());

    let mut stdout = String::new();
    child
        .stdout
        .as_mut()
        .expect("mcp stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    let _ = child.wait();
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("response is JSON"))
        .collect()
}

fn seed_workspace(label: &str) -> PathBuf {
    let pm_dir = tmp_dir(label);
    assert_ok(&pm(&pm_dir, &["init"]), "pm init");
    assert_ok(
        &pm(&pm_dir, &["add", "Demo project", "--kind", "project"]),
        "add PRJ",
    );
    assert_ok(
        &pm(
            &pm_dir,
            &["add", "Core", "--kind", "product", "--parent", "PRJ1"],
        ),
        "add PRD",
    );
    assert_ok(
        &pm(
            &pm_dir,
            &["add", "Checkouts", "--kind", "epic", "--parent", "PRD1"],
        ),
        "add EPC",
    );
    assert_ok(
        &pm(
            &pm_dir,
            &["add", "Lock protocol", "--kind", "task", "--parent", "EPC1"],
        ),
        "add TSK",
    );
    pm_dir
}

#[test]
fn initialize_returns_protocol_version_and_server_info() {
    let pm_dir = seed_workspace("init");
    let resp = drive_mcp(
        &pm_dir,
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#],
    );
    assert_eq!(resp.len(), 1);
    assert_eq!(resp[0]["id"], json!(1));
    let result = &resp[0]["result"];
    assert!(result["protocolVersion"].is_string());
    assert_eq!(result["serverInfo"]["name"], json!("pm"));
    assert!(result["capabilities"]["tools"].is_object());
}

#[test]
fn tools_list_returns_fourteen_tools_with_schemas() {
    let pm_dir = seed_workspace("tools-list");
    let resp = drive_mcp(
        &pm_dir,
        &[r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#],
    );
    assert_eq!(resp.len(), 1);
    let tools = resp[0]["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 14);
    for tool in tools {
        assert!(tool["name"].is_string());
        assert!(tool["description"].is_string());
        assert_eq!(tool["inputSchema"]["type"], json!("object"));
    }
}

#[test]
fn tools_call_list_returns_seeded_tickets() {
    let pm_dir = seed_workspace("list");
    let resp = drive_mcp(
        &pm_dir,
        &[
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list","arguments":{}}}"#,
        ],
    );
    assert_eq!(resp.len(), 1);
    let envelope = &resp[0]["result"];
    assert_eq!(envelope["isError"], json!(false));
    let text = envelope["content"][0]["text"].as_str().expect("text");
    let payload: Value = serde_json::from_str(text).expect("payload parses");
    let tickets = payload["tickets"].as_array().expect("tickets array");
    // Seed added PRJ1 + PRD1 + EPC1 + TSK1.
    assert_eq!(tickets.len(), 4);
}

#[test]
fn tools_call_get_returns_front_matter_and_child_count() {
    let pm_dir = seed_workspace("get");
    let resp = drive_mcp(
        &pm_dir,
        &[
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get","arguments":{"id":"PRD1"}}}"#,
        ],
    );
    let envelope = &resp[0]["result"];
    assert_eq!(envelope["isError"], json!(false));
    let payload: Value =
        serde_json::from_str(envelope["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["id"], json!("PRD1"));
    assert_eq!(payload["kind"], json!("product"));
    assert_eq!(payload["children"], json!(1)); // one EPC under it
}

#[test]
fn tools_call_add_then_list_includes_new_ticket() {
    let pm_dir = seed_workspace("add");
    let resp = drive_mcp(
        &pm_dir,
        &[
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"add","arguments":{"title":"Heartbeat","kind":"task","parent":"EPC1"}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list","arguments":{"kind":"task"}}}"#,
        ],
    );
    assert_eq!(resp.len(), 2);

    let add_text = resp[0]["result"]["content"][0]["text"].as_str().unwrap();
    let add_payload: Value = serde_json::from_str(add_text).unwrap();
    let new_id = add_payload["id"].as_str().unwrap();
    assert!(new_id.starts_with("TSK"), "new id has TSK prefix: {new_id}");

    let list_text = resp[1]["result"]["content"][0]["text"].as_str().unwrap();
    let list_payload: Value = serde_json::from_str(list_text).unwrap();
    let titles: Vec<&str> = list_payload["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert!(
        titles.contains(&"Heartbeat"),
        "new ticket appears in list: {titles:?}"
    );
}

#[test]
fn tools_call_complete_marks_done_and_emits_event() {
    let pm_dir = seed_workspace("complete");
    let resp = drive_mcp(
        &pm_dir,
        &[
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"complete","arguments":{"id":"TSK1"}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get","arguments":{"id":"TSK1"}}}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"events","arguments":{"limit":50}}}"#,
        ],
    );
    assert_eq!(resp.len(), 3);
    let get_payload: Value =
        serde_json::from_str(resp[1]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(get_payload["status"], json!("done"));
    let events_payload: Value =
        serde_json::from_str(resp[2]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let verbs: Vec<&str> = events_payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["verb"].as_str().unwrap())
        .collect();
    assert!(
        verbs.contains(&"complete"),
        "complete event emitted: {verbs:?}"
    );
}

#[test]
fn write_memory_user_scope_returns_is_error() {
    let pm_dir = seed_workspace("write-user");
    let resp = drive_mcp(
        &pm_dir,
        &[
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"write_memory","arguments":{"scope":"user","type":"user","name":"x","content":"y"}}}"#,
        ],
    );
    let envelope = &resp[0]["result"];
    assert_eq!(envelope["isError"], json!(true), "user scope refused");
    let text = envelope["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("user"),
        "stderr-equivalent text mentions user tier"
    );
}

#[test]
fn unknown_method_returns_minus_32601() {
    let pm_dir = seed_workspace("unknown");
    let resp = drive_mcp(
        &pm_dir,
        &[r#"{"jsonrpc":"2.0","id":11,"method":"surely/not"}"#],
    );
    assert_eq!(resp.len(), 1);
    assert_eq!(resp[0]["error"]["code"], json!(-32601));
}
