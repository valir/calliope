//! QA acceptance tests for specs/gui-frontend-foundation.md (static / CLI checks).
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
fn read(p: &str) -> String {
    fs::read_to_string(root().join(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}
fn json(p: &str) -> serde_json::Value {
    serde_json::from_str(&read(p)).unwrap()
}
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)
        } else {
            out.push(p)
        }
    }
}

#[test]
fn req1_typescript_svelte_vite_all_source_in_src() {
    let pkg = json("package.json");
    let deps = |k: &str| pkg[k].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    let all: Vec<String> = deps("dependencies").into_iter().chain(deps("devDependencies")).collect();
    for d in ["svelte", "typescript", "vite", "tailwindcss", "bits-ui"] {
        assert!(all.iter().any(|x| x == d), "missing dependency {d}");
    }
    assert!(root().join("src/ui/main.ts").exists());
    assert!(root().join("components.json").exists(), "shadcn-svelte config");
    // No frontend source outside src/ (config files at the root are not source).
    let mut files = Vec::new();
    for dir in ["tests", "docs", "specs"] {
        walk(&root().join(dir), &mut files);
    }
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        assert!(!["ts", "svelte", "tsx", "js"].contains(&ext), "frontend source outside src/: {}", f.display());
    }
    // every svelte script block is TypeScript
    let mut all = Vec::new();
    walk(&root().join("src/ui"), &mut all);
    for f in all.iter().filter(|f| f.extension().is_some_and(|e| e == "svelte")) {
        let t = fs::read_to_string(f).unwrap();
        if t.contains("<script") {
            assert!(t.contains("lang=\"ts\""), "{} is not TypeScript", f.display());
        }
    }
}

#[test]
fn req2_readme_documents_one_command_each_and_scripts_exist() {
    let readme = read("README.md");
    let pkg = json("package.json");
    for (cmd, script) in [("npm run build:app", "build:app"), ("npm run app", "app"), ("npm run dev:app", "dev:app")] {
        assert!(readme.contains(cmd), "README lacks `{cmd}`");
        assert!(pkg["scripts"].get(script).is_some());
    }
    assert!(readme.contains("npm ci"));
    assert!(pkg["scripts"]["build:app"].as_str().unwrap().contains("cargo build --release"));
    assert!(readme.contains("target/release/calliope-gui"));
}

#[test]
fn req2_build_rs_fails_clearly_when_frontend_missing() {
    let b = read("build.rs");
    assert!(b.contains("the frontend is not built (dist/index.html is missing)"));
    assert!(b.contains("npm run build:app"));
}

#[test]
fn req3_dark_default_light_exists_fonts_bundled() {
    let html = read("src/ui/index.html");
    assert!(html.contains("class=\"dark\""), "first paint must be dark");
    let css = read("src/ui/app.css");
    assert!(css.contains(":root") && css.contains(".dark"), "both themes defined");
    assert!(css.contains("@fontsource-variable/inter"));
    // built CSS references only local font files that exist in dist/assets
    let mut files = Vec::new();
    walk(&root().join("dist/assets"), &mut files);
    assert!(files.iter().any(|f| f.extension().is_some_and(|e| e == "woff2")));
    let theme = read("src/ui/lib/theme.ts");
    assert!(theme.contains("dark"));
    // components required by the spec: buttons, inputs, dialogs, tabs
    for c in ["button", "input", "dialog", "tabs"] {
        assert!(root().join("src/ui/lib/components/ui").join(c).is_dir(), "missing component {c}");
    }
}

#[test]
fn req3_no_remote_references_anywhere_in_dist() {
    let mut files = Vec::new();
    walk(&root().join("dist"), &mut files);
    for f in files {
        if f.extension().is_some_and(|e| e == "woff2" || e == "woff") {
            continue;
        }
        let t = String::from_utf8_lossy(&fs::read(&f).unwrap()).to_string();
        // Only src/href/url/import/fetch style loads matter; licence URLs in comments do not.
        for needle in ["src=\"http", "href=\"http", "url(http", "url(\"http", "url(//", "import(\"http", "from\"http"] {
            assert!(!t.contains(needle), "{} contains remote load `{needle}`", f.display());
        }
    }
}

#[test]
fn req5_ipc_command_registered_and_typed_wrapper_exists() {
    assert!(read("src/gui.rs").contains("ipc::app_version"));
    assert!(read("src/ui/lib/ipc.ts").contains("invoke<string>('app_version')"));
    // only ipc.ts talks to tauri
    let mut files = Vec::new();
    walk(&root().join("src/ui"), &mut files);
    for f in files {
        if f.ends_with("ipc.ts") || f.to_string_lossy().contains(".test.") {
            continue;
        }
        if f.extension().is_some_and(|e| e == "ts" || e == "svelte") {
            assert!(!fs::read_to_string(&f).unwrap().contains("@tauri-apps/api"), "{}", f.display());
        }
    }
}

#[test]
fn req5_version_flag_matches_format_the_frontend_expects() {
    let out = Command::new(env!("CARGO_BIN_EXE_calliope-gui"))
        .arg("--version")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    let v = s.trim().strip_prefix("calliope-gui ").expect("prefix");
    let parts: Vec<_> = v.split('.').collect();
    assert_eq!(parts.len(), 3, "got {v:?}");
    assert!(parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())));
    assert_eq!(parts[2].len(), 4);
}

#[test]
fn req6_focus_ring_is_defined_globally() {
    let css = read("src/ui/app.css");
    let i = css.find(":focus-visible").expect("global :focus-visible rule");
    let rule = &css[i..css[i..].find('}').unwrap() + i];
    assert!(rule.contains("outline") || rule.contains("ring") || rule.contains("box-shadow"), "{rule}");
    assert!(!rule.contains("outline: none") && !rule.contains("outline:none"));
}

#[test]
fn req7_window_min_size_and_state_plugin() {
    let c = json("tauri.conf.json");
    let w = &c["app"]["windows"][0];
    assert!(w["minWidth"].as_u64().unwrap() >= 640 && w["minHeight"].as_u64().unwrap() >= 400);
    assert!(w["width"].as_u64().unwrap() >= w["minWidth"].as_u64().unwrap());
    assert!(w["height"].as_u64().unwrap() >= w["minHeight"].as_u64().unwrap());
    assert!(read("Cargo.toml").contains("tauri-plugin-window-state"));
    assert!(read("src/gui.rs").contains("tauri_plugin_window_state"));
    assert!(read("docs/ui.md").contains("1024"), "docs/ui.md min size");
}

#[test]
fn req8_production_csp_is_exactly_the_skeleton_csp() {
    let c = json("tauri.conf.json");
    let csp = c["app"]["security"]["csp"].as_str().unwrap();
    assert_eq!(
        csp,
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"
    );
    assert!(c["app"]["security"].get("devCsp").is_none());
    assert!(c["build"].get("devUrl").is_none());
}

#[test]
fn req8_main_ts_reports_csp_violations() {
    assert!(read("src/ui/main.ts").contains("securitypolicyviolation"));
}

#[test]
fn ac10_architecture_doc_is_updated() {
    let a = read("docs/architecture.md");
    for needle in ["Svelte 5", "npm run build:app", "src/ipc.rs", "gui-shot", "`:1`", "test:gui"] {
        assert!(a.contains(needle), "architecture.md lacks {needle}");
    }
    assert!(a.contains("outdated"), "old 'no display server' note must be marked outdated");
    assert!(a.contains("earlier note that agents have no display server is\n  outdated"));
}

#[test]
fn no_non_ascii_in_readme() {
    assert!(read("README.md").is_ascii());
}
