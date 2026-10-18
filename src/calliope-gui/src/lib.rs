//! calliope-gui library: every module of the app. `main.rs` is a thin binary over it, and the
//! integration tests in `tests/` import it as `calliope_gui` (so the unit tests run once, here).

pub mod audio_out;
pub mod backing_render;
pub mod beat_detect;
pub mod cli;
pub mod download;
pub mod editor;
pub mod fsutil;
pub mod gui;
pub mod import_job;
pub mod import_tmp;
pub mod ipc;
pub mod local_source;
pub mod media;
pub mod metronome;
pub mod mix_waveform;
pub mod mixer;
pub mod picker;
pub mod player;
mod player_ipc;
pub mod player_source;
pub mod player_state;
pub mod repository;
pub mod settings;
pub mod speed;
pub(crate) mod stretch;
pub mod stem_audio;
pub mod tempo_detect;
pub mod tempo_map;
pub mod tempo_store;
#[cfg(test)]
mod test_server;
pub mod tools;
pub mod track_meta;
pub mod transport;
#[allow(dead_code)] // only build.rs uses it outside tests
mod version;

pub const VERSION: &str = env!("CALLIOPE_VERSION");
