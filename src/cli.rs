//! Pure argument parsing and help/version text (no Tauri dependency).

pub const BIN_NAME: &str = "calliope-gui";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Gui,
    Help,
    Version,
    Error(String),
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Command {
    let args: Vec<String> = args.into_iter().collect();
    match args.as_slice() {
        [] => Command::Gui,
        [a] if a == "--help" => Command::Help,
        [a] if a == "--version" => Command::Version,
        [a] => Command::Error(format!("unknown option '{a}'")),
        _ => Command::Error("too many arguments".to_string()),
    }
}

pub fn help_text(version: &str) -> String {
    format!(
        "{BIN_NAME}, version {version}\n\
         (c) 2026 Valentin Rusu\n\
         \n\
         Usage: {BIN_NAME} [options]\n\
         \n\
         Options:\n\
         \x20  --help: produces this output\n\
         \x20  --version: produces short string containing the version number\n"
    )
}

pub fn version_line(version: &str) -> String {
    format!("{BIN_NAME} {version}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Command {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p(&[]), Command::Gui);
        assert_eq!(p(&["--help"]), Command::Help);
        assert_eq!(p(&["--version"]), Command::Version);
    }

    #[test]
    fn parses_errors() {
        match p(&["--foo"]) {
            Command::Error(m) => {
                assert!(m.contains("--foo"));
                assert_eq!(m, "unknown option '--foo'");
            }
            c => panic!("unexpected {c:?}"),
        }
        assert!(matches!(p(&["--help", "--version"]), Command::Error(_)));
        assert!(matches!(p(&["file.mp3"]), Command::Error(_)));
    }

    #[test]
    fn help_text_exact() {
        let expected = "calliope-gui, version 26.10.0042\n(c) 2026 Valentin Rusu\n\nUsage: calliope-gui [options]\n\nOptions:\n   --help: produces this output\n   --version: produces short string containing the version number\n";
        assert_eq!(help_text("26.10.0042"), expected);
        assert!(help_text("1").lines().all(|l| l == l.trim_end()));
    }

    #[test]
    fn version_line_exact() {
        assert_eq!(version_line("26.10.0042"), "calliope-gui 26.10.0042\n");
    }
}
