//! Computes `CALLIOPE_VERSION` (`YY.MM.BBBB`) at build time.

#[path = "src/version.rs"]
mod version;

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn build_number() -> u32 {
    if let Ok(v) = std::env::var("CALLIOPE_BUILD_NUMBER") {
        return match v.trim().parse::<u32>() {
            Ok(n) if n <= 9999 => n,
            _ => fail(&format!(
                "CALLIOPE_BUILD_NUMBER must be an integer between 0 and 9999, got '{v}'"
            )),
        };
    }
    if let Some(n) = git(&["rev-list", "--count", "HEAD"]).and_then(|s| s.parse::<u32>().ok()) {
        if n > 9999 {
            fail(&format!(
                "git commit count {n} does not fit in 4 digits; set CALLIOPE_BUILD_NUMBER"
            ));
        }
        return n;
    }
    println!("cargo:warning=git commit count unavailable; using build number 0");
    0
}

fn newest_mtime(dir: &std::path::Path) -> Option<SystemTime> {
    let mut newest = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == "node_modules") {
            continue;
        }
        let t = if path.is_dir() {
            newest_mtime(&path)
        } else {
            entry.metadata().ok().and_then(|m| m.modified().ok())
        };
        newest = newest.max(t);
    }
    newest
}

/// Guards against dev configuration in release builds. Returns true when a debug build
/// runs in dev mode (`TAURI_CONFIG` sets a `devUrl`), where `dist/` is not embedded.
fn dev_mode() -> bool {
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    let cfg = std::env::var("TAURI_CONFIG").unwrap_or_default();
    let release = std::env::var("PROFILE").is_ok_and(|p| p == "release");
    if release && (cfg.contains("devUrl") || cfg.contains("devCsp")) {
        fail(
            "dev configuration (devUrl/devCsp) must not be used in a release build; use npm run build:app",
        );
    }
    !release && cfg.contains("devUrl")
}

fn check_frontend() {
    println!("cargo:rerun-if-changed=src/ui");
    println!("cargo:rerun-if-changed=dist/index.html");
    let index = std::path::Path::new("dist/index.html");
    let Ok(built) = std::fs::metadata(index).and_then(|m| m.modified()) else {
        fail(
            "the frontend is not built (dist/index.html is missing). Build the app with: \
             npm ci && npm run build:app   (or only the frontend: npm run build)",
        );
    };
    if newest_mtime(std::path::Path::new("src/ui")).is_some_and(|t| t > built) {
        if std::env::var("PROFILE").is_ok_and(|p| p == "release") {
            fail(
                "the frontend in dist/ is out of date (src/ui is newer than dist/index.html), \
                 and a release build must embed the current frontend. Run: npm run build:app",
            );
        }
        println!(
            "cargo:warning=the frontend in dist/ is stale (src/ui is newer than dist/index.html); run: npm run build"
        );
    }
}

/// The scripted dialog picker is a test hook; it must never ship.
fn guard_e2e_hooks() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_E2E_HOOKS");
    let release = std::env::var("PROFILE").is_ok_and(|p| p == "release");
    if release && std::env::var_os("CARGO_FEATURE_E2E_HOOKS").is_some() {
        fail("the e2e-hooks feature must never be in a release build");
    }
}

fn main() {
    guard_e2e_hooks();
    if !dev_mode() {
        check_frontend();
    }
    println!("cargo:rerun-if-env-changed=CALLIOPE_BUILD_NUMBER");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=src/version.rs");
    for p in ["HEAD", "refs/"] {
        if let Some(path) = git(&["rev-parse", "--git-path", p]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    let secs: i64 = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(v) => v.trim().parse().unwrap_or_else(|_| {
            fail(&format!("SOURCE_DATE_EPOCH must be an integer, got '{v}'"))
        }),
        Err(_) => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };
    let (yy, mm) = version::yymm_from_unix(secs);
    let v = version::format_version(yy, mm, build_number());
    println!("cargo:rustc-env=CALLIOPE_VERSION={v}");

    tauri_build::build();
}
