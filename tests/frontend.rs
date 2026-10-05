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

fn dev_conf() -> serde_json::Value {
    let text = fs::read_to_string(root().join("tauri.dev.conf.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn parse_csp(csp: &str) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    csp.split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| {
            let mut it = d.split_whitespace();
            let name = it.next().unwrap().to_string();
            (name, it.map(str::to_string).collect())
        })
        .collect()
}

#[test]
fn production_csp_has_no_dev_relaxations() {
    let text = fs::read_to_string(root().join("tauri.conf.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let csp = v["app"]["security"]["csp"].as_str().unwrap();
    for bad in ["unsafe-inline", "unsafe-eval", "ws:", "localhost:5173"] {
        assert!(!csp.contains(bad), "production CSP contains {bad}");
    }
    for key in ["devUrl", "devCsp", "beforeDevCommand"] {
        assert!(!text.contains(key), "tauri.conf.json must not contain {key}");
    }
}

#[test]
fn dev_csp_is_minimal_and_separate() {
    let dev = dev_conf();
    assert_eq!(dev["build"]["devUrl"], "http://localhost:5173");
    assert!(dev["app"]["security"].get("csp").is_none(), "dev overlay must not override csp");
    let dev_csp = dev["app"]["security"]["devCsp"].as_str().expect("devCsp must be a string");
    assert!(!dev_csp.contains("unsafe-eval"));
    let prod = conf();
    let mut expected = parse_csp(prod["app"]["security"]["csp"].as_str().unwrap());
    expected.get_mut("style-src").unwrap().insert("'unsafe-inline'".into());
    expected.get_mut("connect-src").unwrap().insert("ws://localhost:5173".into());
    assert_eq!(parse_csp(dev_csp), expected);
}

#[test]
fn e2e_hooks_feature_is_never_default_or_in_release_scripts() {
    let cargo = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let features = &cargo[cargo.find("[features]").expect("[features] section")..];
    assert!(features.contains("e2e-hooks = []"));
    if let Some(i) = features.lines().position(|l| l.trim_start().starts_with("default")) {
        let start: usize = features.lines().take(i).map(|l| l.len() + 1).sum();
        let rest = &features[start..];
        let list = &rest[..rest.find(']').expect("default list end")];
        assert!(!list.contains("e2e-hooks"), "e2e-hooks must not be a default feature");
    }
    let pkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root().join("package.json")).unwrap()).unwrap();
    for name in ["app", "build:app", "build"] {
        let s = pkg["scripts"][name].as_str().unwrap();
        assert!(!s.contains("e2e-hooks"), "script {name} must not use e2e-hooks: {s}");
    }
}

fn names_between(text: &str, start: &str, end: &str) -> std::collections::BTreeSet<String> {
    let s = text.find(start).expect("start marker") + start.len();
    let e = s + text[s..].find(end).expect("end marker");
    text[s..e]
        .split(',')
        .map(|n| n.trim().trim_matches('"').rsplit("::").next().unwrap().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

#[test]
fn handler_list_build_list_and_capability_agree() {
    let gui = fs::read_to_string(root().join("src/gui.rs")).unwrap();
    let handlers = names_between(&gui, "generate_handler![", "]");
    let build = fs::read_to_string(root().join("build.rs")).unwrap();
    let commands = names_between(&build, "const COMMANDS: &[&str] = &[", "];");
    let cap: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root().join("capabilities/main.json")).unwrap())
            .unwrap();
    let perms: Vec<&str> =
        cap["permissions"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    let allow: std::collections::BTreeSet<String> = perms
        .iter()
        .map(|p| p.strip_prefix("allow-").expect("only allow-* entries").replace('-', "_"))
        .collect();
    assert_eq!(perms.len(), allow.len(), "duplicate permission");
    assert!(handlers.len() >= 14);
    assert_eq!(handlers, commands, "generate_handler! vs build.rs COMMANDS");
    assert_eq!(handlers, allow, "generate_handler! vs capabilities/main.json");
    assert_eq!(cap["windows"], serde_json::json!(["main"]));
    assert_eq!(cap["local"], true);
    for p in perms {
        for bad in ["core:", "dialog:", "fs:"] {
            assert!(!p.contains(bad), "{p}");
        }
    }
    assert!(cap.get("remote").is_none());
}

/// Names declared in the given tables of a Cargo.toml (simple `name = ...` lines).
fn cargo_dependency_names(toml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = matches!(line, "[dependencies]" | "[build-dependencies]" | "[dev-dependencies]");
        } else if in_deps && !line.is_empty() && !line.starts_with('#') {
            if let Some((name, _)) = line.split_once('=') {
                names.push(name.trim().to_string());
            }
        }
    }
    names
}

#[test]
fn licence_record_lists_every_direct_dependency() {
    let record = fs::read_to_string(root().join("docs/licences.md")).expect("docs/licences.md");
    let mut names = cargo_dependency_names(&fs::read_to_string(root().join("Cargo.toml")).unwrap());
    assert!(names.len() >= 8, "Cargo.toml parsing found too few dependencies: {names:?}");
    let pkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root().join("package.json")).unwrap()).unwrap();
    for section in ["dependencies", "devDependencies"] {
        names.extend(pkg[section].as_object().unwrap().keys().cloned());
    }
    let missing: Vec<_> = names
        .iter()
        .filter(|n| !record.contains(&format!("`{n}`")))
        .collect();
    assert!(missing.is_empty(), "docs/licences.md is missing: {missing:?}");
}

#[test]
fn dist_ships_the_inter_font_licence() {
    let src = fs::read_to_string(root().join("node_modules/@fontsource-variable/inter/LICENSE")).unwrap();
    let shipped = fs::read_to_string(root().join("dist/licenses/Inter-OFL-1.1.txt"))
        .expect("dist/licenses/Inter-OFL-1.1.txt missing; run `npm run build` first");
    assert_eq!(shipped, src);
    assert!(shipped.contains("SIL OPEN FONT LICENSE Version 1.1"));
}
