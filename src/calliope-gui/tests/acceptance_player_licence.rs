//! Independent acceptance check of the licence switch to MPL-2.0 (plan task 18, owner decision 3).
use std::path::Path;

fn repo() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn licence_file_is_the_mpl_2_0_text() {
    let t = std::fs::read_to_string(repo().join("LICENSE")).unwrap();
    assert!(t.starts_with("Mozilla Public License Version 2.0"));
    for s in ["Exhibit A - Source Code Form License Notice", "Exhibit B - \"Incompatible With Secondary Licenses\" Notice", "1.1. \"Contributor\""] {
        assert!(t.contains(s), "{s}");
    }
    assert!(!t.contains("Apache License"));
    assert!(t.len() > 14000 && t.len() < 20000, "{}", t.len());
}

#[test]
fn every_crate_and_the_npm_project_say_mpl_2_0() {
    let root = std::fs::read_to_string(repo().join("Cargo.toml")).unwrap();
    assert!(root.contains("package.license = \"MPL-2.0\""));
    for c in ["calliope-gui", "calliope-lib", "calliope-stems"] {
        let t = std::fs::read_to_string(repo().join("src").join(c).join("Cargo.toml")).unwrap();
        assert!(t.contains("license.workspace = true"), "{c}");
        assert!(!t.contains("Apache-2.0\""), "{c}");
    }
    let pkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo().join("src/calliope-gui/package.json")).unwrap()).unwrap();
    assert_eq!(pkg["license"], "MPL-2.0");
    let lock: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo().join("src/calliope-gui/package-lock.json")).unwrap()).unwrap();
    assert_eq!(lock["packages"][""]["license"], "MPL-2.0");
}

#[test]
fn readme_and_notices_name_mpl_2_0() {
    let readme = std::fs::read_to_string(repo().join("README.md")).unwrap();
    assert!(readme.contains("MPL-2.0") && readme.contains("Mozilla Public License"));
    assert!(!readme.contains("Apache-2.0 licensed") && !readme.contains("licensed under the Apache"));
    let notices = std::fs::read_to_string(repo().join("src/calliope-stems/THIRD-PARTY-NOTICES.txt")).unwrap();
    assert!(notices.contains("MPL-2.0") || notices.contains("Mozilla Public License"));
}
