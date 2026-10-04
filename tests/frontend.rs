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

fn conf() -> serde_json::Value {
    let text = fs::read_to_string(root().join("tauri.conf.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn window_config_has_min_size_and_title() {
    let v = conf();
    let w = &v["app"]["windows"][0];
    assert_eq!(w["title"], "calliope");
    assert_eq!(w["minWidth"], 1024);
    assert_eq!(w["minHeight"], 640);
    assert_eq!(w["visible"], false);
}

#[test]
fn tauri_conf_has_no_dev_settings() {
    let v = conf();
    assert!(v["build"].get("devUrl").is_none(), "devUrl belongs in tauri.dev.conf.json");
    assert!(v["app"]["security"].get("devCsp").is_none(), "devCsp belongs in tauri.dev.conf.json");
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn files_under(dir: &Path, ext: &str) -> Vec<std::path::PathBuf> {
    let mut all = Vec::new();
    walk(dir, &mut all);
    all.retain(|p| p.extension().is_some_and(|e| e == ext));
    all
}

#[test]
fn no_static_style_attributes_in_svelte() {
    for f in files_under(&root().join("src/ui"), "svelte") {
        let text = fs::read_to_string(&f).unwrap();
        assert!(
            !text.contains("style=\"") && !text.contains("style='"),
            "static style attribute in {} (blocked by CSP; use classes or style: directives)",
            f.display()
        );
    }
}

#[test]
fn dist_has_no_inline_code() {
    let html = fs::read_to_string(root().join("dist/index.html")).expect("run `npm run build` first");
    for part in html.split("<script").skip(1) {
        let tag = &part[..part.find('>').unwrap()];
        assert!(tag.contains("src="), "inline <script> in dist/index.html: <script{tag}>");
    }
    assert!(!html.contains("<style"));
    assert!(!html.contains("style="));
}

fn strip_css_comments(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = match rest[i + 2..].find("*/") {
            Some(j) => &rest[i + 2 + j + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

#[test]
fn dist_assets_are_local() {
    let dist = root().join("dist");
    let html = fs::read_to_string(dist.join("index.html")).expect("run `npm run build` first");
    assert!(!html.contains("http://") && !html.contains("https://"));
    for f in files_under(&dist, "css") {
        let css = strip_css_comments(&fs::read_to_string(&f).unwrap());
        let compact: String = css.chars().filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'').collect();
        for bad in ["url(http", "url(//", "url(data:", "@importhttp", "@import//", "@importurl(http", "@importurl(//"] {
            assert!(!compact.contains(bad), "{} contains `{bad}`", f.display());
        }
    }
    assert!(
        !files_under(&dist.join("assets"), "woff2").is_empty(),
        "no .woff2 font under dist/assets"
    );
}
