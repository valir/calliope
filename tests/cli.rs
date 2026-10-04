use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calliope-gui"))
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run calliope-gui")
}

fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_version(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 3 && digits(p[0], 2) && digits(p[1], 2) && digits(p[2], 4)
}

fn version() -> String {
    let out = run(&["--version"]);
    let s = String::from_utf8(out.stdout).unwrap();
    s.strip_prefix("calliope-gui ")
        .and_then(|r| r.strip_suffix('\n'))
        .expect("version line shape")
        .to_string()
}

#[test]
fn version_flag() {
    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let s = String::from_utf8(out.stdout).unwrap();
    let v = s.strip_prefix("calliope-gui ").unwrap();
    assert!(v.ends_with('\n') && !v[..v.len() - 1].contains('\n'));
    assert!(is_version(&v[..v.len() - 1]), "bad version: {s:?}");
}

#[test]
fn help_flag() {
    let v = version();
    assert!(is_version(&v));
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let s = String::from_utf8(out.stdout).unwrap();
    let expected = format!(
        "calliope-gui, version {v}\n(c) 2026 Valentin Rusu\n\nUsage: calliope-gui [options]\n\nOptions:\n   --help: produces this output\n   --version: produces short string containing the version number\n"
    );
    assert_eq!(s, expected);
    let first = s.lines().next().unwrap();
    assert!(is_version(first.strip_prefix("calliope-gui, version ").unwrap()));
}

#[test]
fn unknown_option() {
    let out = run(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let e = String::from_utf8(out.stderr).unwrap();
    assert!(e.contains("unknown option '--bogus'"));
    assert!(e.contains("Usage: calliope-gui [options]"));
}
