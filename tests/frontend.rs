use std::fs;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn tauri_conf_points_at_frontend() {
    let text = fs::read_to_string(root().join("tauri.conf.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["build"]["frontendDist"], "dist");
    assert_eq!(v["productName"], "calliope-gui");
    assert_eq!(v["app"]["windows"].as_array().unwrap().len(), 1);
}

#[test]
fn tauri_conf_has_strict_csp() {
    let text = fs::read_to_string(root().join("tauri.conf.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let csp = v["app"]["security"]["csp"].as_str().expect("csp must be a string");
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("object-src 'none'"));
    assert!(!csp.contains("unsafe-inline"));
    assert!(!csp.contains("unsafe-eval"));
}

#[test]
fn index_html_has_no_inline_code() {
    let html = fs::read_to_string(root().join("src/ui/index.html")).unwrap();
    for part in html.split("<script").skip(1) {
        let tag = &part[..part.find('>').unwrap()];
        assert!(tag.contains("src="), "inline <script> found: <script{tag}>");
    }
    assert!(!html.contains("<style"));
    assert!(!html.contains("style="));
}
