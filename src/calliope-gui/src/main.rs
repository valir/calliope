mod audio_out;
mod backing_render;
mod cli;
mod download;
mod editor;
mod fsutil;
mod import_job;
mod import_tmp;
mod gui;
mod ipc;
mod local_source;
mod media;
#[allow(dead_code)] // until the player session uses it
mod metronome;
mod mixer;
mod picker;
#[allow(dead_code)] // until the player session uses it
mod player_source;
#[allow(dead_code)] // until the player session uses it
mod player_state;
mod repository;
mod settings;
mod stem_audio;
#[allow(dead_code)] // until the player session uses it
mod tempo_map;
mod tools;
mod transport;
mod track_meta;
#[allow(dead_code)] // only build.rs uses it outside tests
mod version;

const VERSION: &str = env!("CALLIOPE_VERSION");

fn main() {
    match cli::parse_args(std::env::args().skip(1)) {
        cli::Command::Help => print!("{}", cli::help_text(VERSION)),
        cli::Command::Version => print!("{}", cli::version_line(VERSION)),
        cli::Command::Gui => gui::run(),
        cli::Command::Error(m) => {
            eprint!("{}: {m}\n\n{}", cli::BIN_NAME, cli::help_text(VERSION));
            std::process::exit(2);
        }
    }
}
