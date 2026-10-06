mod cli;
mod fsutil;
mod import_tmp;
mod gui;
mod ipc;
mod media;
mod picker;
mod repository;
mod settings;
mod tools;
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
