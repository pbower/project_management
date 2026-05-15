//! Shared TUI views that the App and standalone binaries reuse.
//!
//! Phase 9 lands the first view here: `events_view`, the full-screen activity
//! feed shared between Mode 3 inside the main TUI and the standalone `pm tv`
//! kiosk. Future phases can grow this module with additional shared views.

pub mod events_view;
