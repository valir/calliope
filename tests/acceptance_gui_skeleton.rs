//! Independent acceptance tests for specs/gui-skeleton.md (headless only).
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_calliope-gui");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run")
}

fn out(args: &[&str]) -> String {
    String::from_utf8(run(args).stdout).unwrap()
}

fn ver() -> String {
    out(&["--version"]).trim_end().strip_prefix("calliope-gui ").unwrap().to_string()
}

#[test]
fn ac1_binary_exists_and_is_named_calliope_gui() {
    assert!(Path::new(BIN).is_file());
    assert_eq!(Path::new(BIN).file_stem().unwrap(), "calliope-gui");
}

#[test]
fn req2_source_lives_in_src() {
    for f in ["src/main.rs", "src/cli.rs", "src/gui.rs", "src/ui/index.html"] {
        assert!(Path::new(ROOT).join(f).is_file(), "{f}");
    }
}

#[test]
fn req3_tauri_is_used() {
    let toml = std::fs::read_to_string(Path::new(ROOT).join("Cargo.toml")).unwrap();
    assert!(toml.contains("tauri = "));
    let gui = std::fs::read_to_string(Path::new(ROOT).join("src/gui.rs")).unwrap();
    assert!(gui.contains("tauri::Builder"));
}

#[test]
fn ac2_static_gui_content_and_wiring() {
    let html = std::fs::read_to_string(Path::new(ROOT).join("src/ui/index.html")).unwrap();
    assert!(html.contains("<title>calliope</title>"));
    let conf: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(ROOT).join("tauri.conf.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(conf["build"]["frontendDist"], "dist");
    assert_eq!(conf["app"]["windows"].as_array().unwrap().len(), 1);
    // GUI is only started for no-arg; args never reach it (no window for CLI paths).
    let main = std::fs::read_to_string(Path::new(ROOT).join("src/main.rs")).unwrap();
    assert!(main.contains("Command::Gui => gui::run()"));
}

#[test]
fn ac3_help_exact_output() {
    let v = ver();
    let expected = format!(
        "calliope-gui, version {v}\n(c) 2026 Valentin Rusu\n\nUsage: calliope-gui [options]\n\nOptions:\n   --help: produces this output\n   --version: produces short string containing the version number\n"
    );
    let o = run(&["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(o.stderr.is_empty());
    assert_eq!(String::from_utf8(o.stdout).unwrap(), expected);
}

#[test]
fn ac4_version_exact_output_and_format() {
    let o = run(&["--version"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(o.stderr.is_empty());
    let s = String::from_utf8(o.stdout).unwrap();
    let v = s.strip_suffix('\n').unwrap().strip_prefix("calliope-gui ").unwrap();
    let p: Vec<&str> = v.split('.').collect();
    assert_eq!(p.len(), 3);
    assert_eq!(p.iter().map(|x| x.len()).collect::<Vec<_>>(), vec![2, 2, 4]);
    assert!(p.iter().all(|x| x.bytes().all(|b| b.is_ascii_digit())));
}

#[test]
fn req7_version_date_is_current_utc_and_build_is_commit_count() {
    let v = ver();
    let date = Command::new("date").args(["-u", "+%y.%m"]).output().unwrap();
    let now = String::from_utf8(date.stdout).unwrap();
    if std::env::var("SOURCE_DATE_EPOCH").is_err() {
        assert_eq!(&v[..5], now.trim(), "YY.MM should be UTC build date");
    }
    if std::env::var("CALLIOPE_BUILD_NUMBER").is_err() {
        let g = Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(ROOT)
            .output()
            .unwrap();
        if g.status.success() {
            let n: u32 = String::from_utf8(g.stdout).unwrap().trim().parse().unwrap();
            let b: u32 = v[6..].parse().unwrap();
            // build may predate later commits, never exceed
            assert!(b <= n, "build {b} > commits {n}");
        }
    }
}

#[test]
fn req4_6_unknown_and_extra_args_rejected_without_gui() {
    for args in [vec!["--bogus"], vec!["file.mp3"], vec!["--help", "--version"], vec!["-h"], vec![""]] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
        assert!(!o.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn help_has_no_trailing_whitespace_and_single_final_newline() {
    let s = out(&["--help"]);
    assert!(s.ends_with("number\n") && !s.ends_with("\n\n"));
    assert!(s.lines().all(|l| l == l.trim_end()));
}

#[test]
fn no_args_without_display_does_not_print_cli_output() {
    // No display: GUI init fails; it must not print help/version text on stdout.
    let o = run(&[]);
    assert!(o.stdout.is_empty());
}
