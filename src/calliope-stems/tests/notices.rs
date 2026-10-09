//! THIRD-PARTY-NOTICES.txt (embedded, printed by `--licenses`) must list every third-party
//! crate linked into calliope-stems at its locked version. Regenerate it with
//! `npm run notices:stems` in src/calliope-gui after changing dependencies.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

#[test]
fn notices_list_every_linked_crate() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let notices = std::fs::read_to_string(dir.join("THIRD-PARTY-NOTICES.txt")).unwrap();
    let rustc = Command::new("rustc").arg("-vV").output().expect("run rustc -vV");
    let host = String::from_utf8(rustc.stdout).unwrap();
    let host = host.lines().find_map(|l| l.strip_prefix("host: ")).unwrap().to_string();
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--locked", "--filter-platform", &host])
        .current_dir(dir)
        .output()
        .expect("run cargo metadata");
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let members = meta["workspace_members"].as_array().unwrap();
    let packages = meta["packages"].as_array().unwrap();
    let nodes = meta["resolve"]["nodes"].as_array().unwrap();
    let start = packages.iter().find(|p| p["name"] == "calliope-stems").unwrap()["id"].clone();
    let mut seen = BTreeSet::new();
    let mut todo = vec![start];
    while let Some(id) = todo.pop() {
        if !seen.insert(id.to_string()) {
            continue;
        }
        let node = nodes.iter().find(|n| n["id"] == id).unwrap();
        for d in node["deps"].as_array().unwrap() {
            if d["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null()) {
                todo.push(d["pkg"].clone());
            }
        }
    }
    let linked: Vec<String> = packages
        .iter()
        .filter(|p| seen.contains(&p["id"].to_string()) && !members.contains(&p["id"]))
        .map(|p| format!("{} {}", p["name"].as_str().unwrap(), p["version"].as_str().unwrap()))
        .collect();
    let missing: Vec<_> = linked.iter().filter(|l| !notices.contains(&format!("\n{l}  ["))).collect();
    assert!(
        missing.is_empty(),
        "crates missing from THIRD-PARTY-NOTICES.txt (run `npm run notices:stems` in src/calliope-gui): {missing:?}"
    );
    let listed = notices.lines().filter(|l| l.contains("  [")).count();
    assert_eq!(listed, linked.len(), "the notices list crates no longer linked; regenerate them");
}
