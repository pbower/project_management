//! Phase 11: stdio MCP server.
//!
//! Exposes 14 tools (PM_DESIGN.md Section 8.2) over JSON-RPC 2.0 on
//! `stdin`/`stdout`. Each tool maps to a small handler that calls into the
//! existing primitives - the `Database`, `Ticket`, `memory::*`, `store::events`,
//! and the lock module - rather than going through the CLI's `cmd_*` wrappers.
//! That keeps the agent-facing vocabulary minimal and lets the CLI keep its
//! own ergonomics without the two diverging.
//!
//! The wire surface is the standard MCP shape:
//!
//! - `initialize` returns server info and the protocol version.
//! - `initialized` is a notification; the server records the handshake.
//! - `tools/list` returns the registered tool catalog with JSON Schemas.
//! - `tools/call` dispatches to the named handler with the supplied
//!   arguments. The handler's return string becomes a single text content
//!   block in the response.

pub mod handlers;
pub mod protocol;
pub mod server;
pub mod tools;

pub use protocol::{Error as ProtocolError, Request, Response, ResponseError};
pub use server::{run as run_server, Server, ServerError};
pub use tools::{tool_catalog, ToolDef};
