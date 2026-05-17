//! stdio read-eval-print loop driving the MCP server.
//!
//! Each line on stdin is one JSON-RPC request. The server dispatches based
//! on the method name and writes one JSON-RPC response per line on stdout.
//! Notifications (no `id`) do not produce a response. The loop exits when
//! stdin closes (EOF).

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use super::handlers::{dispatch, Context};
use super::protocol::{Error as ProtocolError, Request, Response, ResponseError, JSONRPC_VERSION};
use super::tools::tool_catalog;

/// Server-level errors. These reach the caller when the read loop itself
/// fails; per-request errors are reported on the wire.
#[derive(Debug)]
pub enum ServerError {
    Io(io::Error),
    Protocol(ProtocolError),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Io(e) => write!(f, "mcp server io: {e}"),
            ServerError::Protocol(e) => write!(f, "mcp server protocol: {e}"),
        }
    }
}

impl std::error::Error for ServerError {}

/// One server instance bound to a workspace.
pub struct Server {
    pub ctx: Context,
}

impl Server {
    pub fn new(pm_dir: PathBuf) -> Self {
        Server {
            ctx: Context::for_pm_dir(pm_dir),
        }
    }

    /// Drive the read loop using arbitrary readers/writers. The binary
    /// entry point wires `stdin` and `stdout`; tests pipe their own.
    pub fn drive<R: BufRead, W: Write>(
        &mut self,
        reader: R,
        mut writer: W,
    ) -> Result<(), ServerError> {
        for line in reader.lines() {
            let line = line.map_err(ServerError::Io)?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let response = self.handle_line(trimmed);
            if let Some(resp) = response {
                let s = serde_json::to_string(&resp)
                    .map_err(|e| ServerError::Protocol(ProtocolError::Json(e)))?;
                writer.write_all(s.as_bytes()).map_err(ServerError::Io)?;
                writer.write_all(b"\n").map_err(ServerError::Io)?;
                writer.flush().map_err(ServerError::Io)?;
            }
        }
        Ok(())
    }

    /// Process one parsed line. Returns `None` for notifications (no `id`).
    fn handle_line(&mut self, raw: &str) -> Option<Response> {
        let request: Request = match serde_json::from_str(raw) {
            Ok(r) => r,
            Err(e) => {
                return Some(Response::err(
                    json!(null),
                    ResponseError::parse_error(format!("invalid JSON: {e}")),
                ));
            }
        };
        if request.jsonrpc != JSONRPC_VERSION {
            return request.id.map(|id| {
                Response::err(
                    id,
                    ResponseError::invalid_request("jsonrpc must be \"2.0\""),
                )
            });
        }
        let id = request.id.clone()?;
        Some(self.dispatch_method(&request.method, request.params.as_ref(), id))
    }

    fn dispatch_method(&mut self, method: &str, params: Option<&Value>, id: Value) -> Response {
        match method {
            "initialize" => Response::ok(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {
                        "name": "spacecell-thunder",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "tools": {}
                    }
                }),
            ),
            "tools/list" => {
                let tools: Vec<Value> =
                    tool_catalog().into_iter().map(|t| t.to_listing()).collect();
                Response::ok(id, json!({"tools": tools}))
            }
            "tools/call" => {
                let params = match params {
                    Some(p) => p,
                    None => {
                        return Response::err(id, ResponseError::invalid_params("missing params"))
                    }
                };
                let name = match params.get("name").and_then(Value::as_str) {
                    Some(n) => n,
                    None => {
                        return Response::err(id, ResponseError::invalid_params("missing name"))
                    }
                };
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match dispatch(&mut self.ctx, name, &args) {
                    Ok(payload) => Response::ok(
                        id,
                        json!({
                            "content": [{"type": "text", "text": serde_json::to_string(&payload).unwrap_or_default()}],
                            "isError": false,
                        }),
                    ),
                    Err(msg) => Response::ok(
                        id,
                        json!({
                            "content": [{"type": "text", "text": msg}],
                            "isError": true,
                        }),
                    ),
                }
            }
            _ => Response::err(id, ResponseError::method_not_found(method)),
        }
    }
}

/// Convenience entry point used by the `pm mcp` subcommand. Drives the
/// server against the process's `stdin` and `stdout`.
pub fn run(pm_dir: PathBuf) -> Result<(), ServerError> {
    let mut server = Server::new(pm_dir);
    let stdin = io::stdin();
    let stdout = io::stdout();
    server.drive(stdin.lock(), stdout.lock())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn tmp_pm_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-mcp-server-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Drive the server with a controlled in-memory reader/writer pair so
    /// tests can exercise the JSON-RPC dispatch without touching real stdio.
    fn rpc<S: AsRef<str>>(server: &mut Server, lines: &[S]) -> Vec<Value> {
        let mut input = String::new();
        for l in lines {
            input.push_str(l.as_ref());
            input.push('\n');
        }
        let mut out: Vec<u8> = Vec::new();
        server.drive(Cursor::new(input), &mut out).unwrap();
        let raw = String::from_utf8(out).unwrap();
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .collect()
    }

    #[test]
    fn initialize_returns_server_info() {
        let mut server = Server::new(tmp_pm_dir());
        let responses = rpc(
            &mut server,
            &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#],
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["id"], json!(1));
        assert_eq!(
            responses[0]["result"]["serverInfo"]["name"],
            json!("spacecell-thunder")
        );
        assert!(responses[0]["result"]["protocolVersion"].is_string());
    }

    #[test]
    fn tools_list_returns_fourteen_entries() {
        let mut server = Server::new(tmp_pm_dir());
        let responses = rpc(
            &mut server,
            &[r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#],
        );
        assert_eq!(responses.len(), 1);
        let tools = responses[0]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 14);
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let mut server = Server::new(tmp_pm_dir());
        let responses = rpc(
            &mut server,
            &[r#"{"jsonrpc":"2.0","id":3,"method":"surely/not"}"#],
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["error"]["code"], json!(-32601));
    }

    #[test]
    fn notification_produces_no_response() {
        let mut server = Server::new(tmp_pm_dir());
        let responses = rpc(
            &mut server,
            &[r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#],
        );
        assert!(
            responses.is_empty(),
            "notifications don't produce responses"
        );
    }

    #[test]
    fn invalid_json_returns_parse_error() {
        let mut server = Server::new(tmp_pm_dir());
        let responses = rpc(&mut server, &[r#"not even json"#]);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["error"]["code"], json!(-32700));
    }

    #[test]
    fn tools_call_unknown_tool_returns_is_error_text() {
        let mut server = Server::new(tmp_pm_dir());
        let responses = rpc(
            &mut server,
            &[
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
            ],
        );
        assert_eq!(responses.len(), 1);
        let result = &responses[0]["result"];
        assert_eq!(result["isError"], json!(true));
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown tool"));
    }
}
