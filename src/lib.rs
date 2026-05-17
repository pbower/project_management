//! Library surface for the `spacecell` binary, examples and integration
//! tests.
//!
//! All public modules live here; `src/main.rs` is a thin wrapper that
//! calls into them. Splitting into lib + bin lets `cargo run --example`
//! and `tests/` integration suites link against the same code path the
//! binary uses.

pub mod cli;
pub mod cmd;
pub mod db;
pub mod fields;
pub mod mcp;
pub mod memory;
pub mod project;
pub mod store;
pub mod style;
pub mod task;
pub mod views;
pub mod tui {
    pub mod enums;
    pub mod nav;
    pub mod run;
    pub mod utils;
    pub mod workflow;
    pub mod workflow_run;
}
