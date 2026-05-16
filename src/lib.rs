//! Library surface for the `pm` binary and its examples / integration tests.
//!
//! All public modules live here; `src/main.rs` is a thin wrapper that calls
//! into them. Splitting into lib + bin lets `cargo run --example ...` and
//! `tests/` integration suites link against the same code path the binary
//! uses.

pub mod cli;
pub mod cmd;
pub mod db;
pub mod editor;
pub mod fields;
pub mod mcp;
pub mod memory;
pub mod project;
pub mod store;
pub mod task;
pub mod views;
pub mod tui {
    pub mod app;
    pub mod colors;
    pub mod enums;
    pub mod input;
    pub mod menu;
    pub mod run;
    pub mod task_form;
    pub mod utils;
    pub mod workflow;
    pub mod workflow_run;
}
