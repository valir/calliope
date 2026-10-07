//! Code shared by `calliope-gui` and `calliope-stems`.
//!
//! This crate must stay free of Tauri/GTK/WebKit and of any HTTP server (`tests/frontend.rs`
//! checks the dependency graph with `cargo metadata`).

pub mod process;
pub mod stems_api;
#[cfg(feature = "client")]
pub mod stems_client;
