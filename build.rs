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

fn main() {
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
