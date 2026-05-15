//! JSON-RPC 2.0 wire types for the MCP server.
//!
//! MCP runs on top of JSON-RPC 2.0 over stdio. Each line of stdin is one
//! request; each line of stdout is one response (or one notification, which
//! has no `id` and expects no response). The types here are the minimum
//! shape we need: a request, a response, and a structured error.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC version string sent in every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// One JSON-RPC request. `id` is absent for notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
}

/// One JSON-RPC response. Exactly one of `result` or `error` is present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: ResponseError) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// JSON-RPC error object. `code` follows the canonical reserved range; the
/// MCP spec layers application-specific codes on top.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ResponseError {
    pub fn parse_error(message: impl Into<String>) -> Self {
        ResponseError { code: -32700, message: message.into(), data: None }
    }
    pub fn invalid_request(message: impl Into<String>) -> Self {
        ResponseError { code: -32600, message: message.into(), data: None }
    }
    pub fn method_not_found(message: impl Into<String>) -> Self {
        ResponseError { code: -32601, message: message.into(), data: None }
    }
    pub fn invalid_params(message: impl Into<String>) -> Self {
        ResponseError { code: -32602, message: message.into(), data: None }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        ResponseError { code: -32603, message: message.into(), data: None }
    }
}

/// Internal error surface for the protocol layer. The server module maps
/// these to `ResponseError` values on the wire.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "mcp io: {e}"),
            Error::Json(e) => write!(f, "mcp json: {e}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req: Request = serde_json::from_str(raw).unwrap();
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, Some(serde_json::json!(1)));
        assert!(req.params.is_none());
    }

    #[test]
    fn response_ok_serialises_without_error_field() {
        let resp = Response::ok(serde_json::json!(7), serde_json::json!({"hello": "world"}));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains(r#""result":{"hello":"world"}"#), "{s}");
        assert!(!s.contains("error"), "no error field on success: {s}");
    }

    #[test]
    fn response_err_serialises_without_result_field() {
        let resp = Response::err(serde_json::json!(2), ResponseError::method_not_found("nope"));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("nope"));
        assert!(s.contains(r#""code":-32601"#));
        assert!(!s.contains("\"result\""));
    }

    #[test]
    fn notification_has_no_id() {
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req: Request = serde_json::from_str(raw).unwrap();
        assert!(req.id.is_none());
    }
}
