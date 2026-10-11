use calliope_gui::{cli, gui, VERSION};

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
