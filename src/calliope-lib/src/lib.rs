//! Code shared by `calliope-gui` and `calliope-stems`: the stems API v1 types, the FLAC peak scan,
//! a child-process runner and (feature `client`) the HTTP client.
//!
//! This crate must stay free of Tauri/GTK/WebKit and of any HTTP server (`tests/frontend.rs`
//! checks the dependency graph with `cargo metadata`).

pub mod flac_peak;
pub mod process;
pub mod stems_api;
#[cfg(feature = "client")]
pub mod stems_client;
