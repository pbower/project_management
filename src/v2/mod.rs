//! v2 CLI surface. New command tree operating on the typed-id store in
//! [`crate::store`]. Reachable from the existing binary as `pm v2 <verb>`.
//!
//! Long-term these commands replace the original v0.9.x verbs; for now the
//! two surfaces coexist so users can migrate at their own pace.

pub mod cli;
pub mod cmd;
pub mod root;
