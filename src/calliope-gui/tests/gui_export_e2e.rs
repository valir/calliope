//! Export GUI end-to-end tests. They need an X display and are run with:
//! `npm run test:gui` (or
//! `cargo test --features e2e-hooks --test gui_export_e2e -- --ignored --test-threads=1`).
//!
//! Every test works on a copy of `tests/fixtures/library-export` under `target/gui-e2e/<test>/`
//! with scripted folder dialogs; destinations are temp folders below the same directory.

mod common;

use common::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg") && have("ffmpeg") && have("ffprobe")
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn answers(dirs: &Dirs, lines: &[String]) -> PathBuf {
    let p = dirs.base().join("answers");
    std::fs::write(&p, lines.join("\n") + "\n").unwrap();
    p
}

/// A fresh RC-5 drive: `<test>/<name>/ROLAND`.
fn rc5(dirs: &Dirs, name: &str) -> PathBuf {
    let p = dirs.base().join(name);
    std::fs::create_dir_all(p.join("ROLAND")).unwrap();
    p
}

fn wav(base: &Path, n: u8) -> PathBuf {
    base.join(format!("ROLAND/WAVE/{n:03}_1/{n:03}_1.WAV"))
}

/// Starts the app on the export fixture (capture audio: never the real device), floating at
/// 1280x800, with the Export view open (Alt+6).
fn start(dirs: &Dirs, ans: &Path, extra: &[(&str, &str)]) -> (App, String) {
    let audio = format!("capture:{}", dirs.capture().display());
    let extra: Vec<(&str, PathBuf)> = extra.iter().map(|(k, v)| (*k, PathBuf::from(v))).collect();
    let mut env: Vec<(&str, &Path)> = vec![("CALLIOPE_E2E_DIALOG_ANSWERS", ans), ("CALLIOPE_E2E_AUDIO", Path::new(&audio))];
    for (k, v) in &extra {
        env.push((k, v));
    }
    let (mut app, wid, _) = App::start_lib(dirs, &env);
    app.key(&wid, "alt+6");
    app.wait_view("export");
    app.wait_contains("export layout");
    (app, wid)
}

/// Selects a track by its title through the search box.
fn pick(app: &mut App, wid: &str, query: &str, id: &str) {
    app.select_by_search(wid, query, id);
}

fn target_line(app: &mut App, id: &str, kind: &str) -> String {
    let want = format!("export target id={id} kind={kind}");
    app.wait_line(|l| l.contains(&want), secs(10))
}

// Buttons at 1280x800 (window coordinates).
const BTN_RC5: (i32, i32) = (748, 290);
const BTN_MP3: (i32, i32) = (748, 362);
const BTN_OGG: (i32, i32) = (748, 434);
const MEMORY_BOX: (i32, i32) = (971, 290);

fn press(app: &App, wid: &str, at: (i32, i32)) {
    app.click(wid, at.0, at.1);
}

fn set_memory(app: &App, wid: &str, text: &str) {
    press(app, wid, MEMORY_BOX);
    app.key(wid, "ctrl+a");
    app.typ(wid, text);
    settle();
}

fn lines_with(app: &App, what: &str) -> Vec<String> {
    app.all_lines().into_iter().filter(|l| l.contains(what)).collect()
}

fn wait_done(app: &mut App) -> String {
    app.wait_line(|l| l.contains("export done path="), secs(30))
}

/// The settings.json the app wrote.
fn settings(dirs: &Dirs) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dirs.app_config().join("settings.json")).unwrap()).unwrap()
}

/// Checks the RC-5 WAV header (44 bytes, float32 stereo) and returns (rate, left, right).
fn read_wav(path: &Path) -> (u32, Vec<f32>, Vec<f32>) {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(&b[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes([b[20], b[21]]), 3, "float format");
    assert_eq!(u16::from_le_bytes([b[22], b[23]]), 2, "stereo");
    assert_eq!(&b[36..40], b"data");
    let rate = u32::from_le_bytes([b[24], b[25], b[26], b[27]]);
    let f = |c: &[u8]| f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for fr in b[44..].chunks_exact(8) {
        l.push(f(&fr[0..4]));
        r.push(f(&fr[4..8]));
    }
    (rate, l, r)
}

fn tags(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let o = out("ffprobe", &["-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "json", &path.display().to_string()]);
    let v: serde_json::Value = serde_json::from_str(&o).unwrap();
    // MP3 keeps the tags on the format, Ogg on its stream.
    let mut m = serde_json::Map::new();
    for t in [&v["format"]["tags"], &v["streams"][0]["tags"]] {
        for (k, val) in t.as_object().into_iter().flatten() {
            m.insert(k.to_lowercase(), val.clone());
        }
    }
    assert!(m.contains_key("title"), "no tags in {o}");
    m
}

fn layout_overflow(app: &App) -> i64 {
    let l = lines_with(app, "export layout overflow=").pop().expect("export layout line");
    field(&l, "overflow").parse().unwrap()
}

#[test]
#[ignore]
fn export_view_buttons() {
    if !ready() {
        return;
    }
    let dirs = export_dirs("export_view_buttons");
    let before = snapshot(&dirs.repo());
    let ans = answers(&dirs, &[]);
    let (mut app, wid) = start(&dirs, &ans, &[]);
    settle();
    shot("export-idle");

    pick(&mut app, &wid, "stem mix", EX_STEM);
    let l = target_line(&mut app, EX_STEM, "track");
    assert!(l.contains("available=true"), "{l}");
    settle();
    shot("export-dark-1280");
    assert!(layout_overflow(&app) <= 14, "layout overflow {}", layout_overflow(&app));

    pick(&mut app, &wid, "no mix", EX_NOMIX);
    let l = target_line(&mut app, EX_NOMIX, "track");
    assert!(l.contains("available=false"), "{l}");
    settle();
    shot("export-no-mix");

    // The band row: a group, nothing to export.
    app.key(&wid, "ctrl+f");
    app.typ(&wid, "example");
    settle();
    app.key(&wid, "Down Home");
    let l = app.wait_line(|l| l.contains("export target id=none kind=group"), secs(10));
    assert!(l.contains("available=false"), "{l}");
    settle();
    shot("export-group");

    pick(&mut app, &wid, "plain mp3", EX_PLAIN);
    let l = target_line(&mut app, EX_PLAIN, "track");
    assert!(l.contains("available=true"), "{l}");

    pick(&mut app, &wid, "no tempo", EX_NOTEMPO);
    let l = target_line(&mut app, EX_NOTEMPO, "track");
    assert!(l.contains("available=false"), "{l}");
    settle();
    shot("export-no-tempo");
    assert!(layout_overflow(&app) <= 14, "layout overflow {}", layout_overflow(&app));

    // Light theme at 1024x640 (Settings: Tab, Right), back to the Export view.
    pick(&mut app, &wid, "stem mix", EX_STEM);
    app.key(&wid, "alt+7");
    app.wait_view("settings");
    app.key(&wid, "Tab");
    app.key(&wid, "Right");
    app.wait_line(|l| l.trim_end().ends_with("theme=light"), secs(10));
    app.key(&wid, "alt+6");
    app.wait_view("export");
    size(&wid, 1024, 640);
    settle();
    shot("export-light-1024");
    app.key(&wid, "ctrl+f");
    app.typ(&wid, "x");
    settle();
    shot("export-light-1024-tree");

    assert_eq!(snapshot(&dirs.repo()), before, "the repository must not change");
    assert!(lines_with(&app, "dialog kind=").is_empty(), "no dialog was needed");
    app.assert_no_real_audio();
    app.no_csp_violation();
}

#[test]
#[ignore]
fn export_rc5_flow() {
    if !ready() {
        return;
    }
    let dirs = export_dirs("export_rc5_flow");
    let before = snapshot(&dirs.repo());
    let base = rc5(&dirs, "rc5");
    let target = wav(&base, 7);
    let ans = answers(&dirs, &[format!("export-rc5 {}", base.display())]);
    {
        let (mut app, wid) = start(&dirs, &ans, &[]);
        pick(&mut app, &wid, "mono backing", EX_MONO);
        target_line(&mut app, EX_MONO, "track");
        set_memory(&app, &wid, "7");
        press(&app, &wid, BTN_RC5);
        let l = app.wait_line(|l| l.contains("dialog start kind=export-rc5"), secs(10));
        assert!(l.contains("from=home") || l.contains("from=none") || !l.contains("from=last"), "{l}");
        wait_done(&mut app);
        settle();
        shot("export-done");
        assert_eq!(settings(&dirs)["rc5_memory"], 7);
        assert_eq!(settings(&dirs)["rc5_dir"], base.display().to_string());
        // A mono original becomes stereo with equal channels; 4 s + 5 beats of 120 BPM.
        let (rate, l, r) = read_wav(&target);
        assert_eq!(rate, 44100);
        assert_eq!(l, r, "equal channels");
        assert_eq!(l.len(), 44100 * 4 + 110250);
        assert!(l.iter().any(|v| v.abs() > 0.05), "the audio is there");
        assert!(!target.with_extension("WAV.part").exists());
        app.assert_no_real_audio();
    }

    // After a restart: the dialog starts in the remembered drive and the box suggests 8.
    let ans = answers(&dirs, &[format!("export-rc5 {}", base.display()), format!("export-rc5 {}", base.display())]);
    let (mut app, wid) = start(&dirs, &ans, &[]);
    pick(&mut app, &wid, "mono backing", EX_MONO);
    target_line(&mut app, EX_MONO, "track");
    settle();
    shot("export-restart-memory-8");
    set_memory(&app, &wid, "7");
    press(&app, &wid, BTN_RC5);
    let l = app.wait_line(|l| l.contains("dialog start kind=export-rc5"), secs(10));
    assert!(l.contains("from=last"), "{l}");
    app.wait_line(|l| l.contains("export prepare target=rc5 result=plan exists=true"), secs(10));
    settle();
    shot("export-confirm-overwrite");
    let old = std::fs::read(&target).unwrap();
    // No is focused: Return answers No.
    app.key(&wid, "Return");
    let l = app.wait_line(|l| l.contains("export confirm answer="), secs(10));
    assert!(l.contains("answer=no") && l.contains(&target.display().to_string()), "{l}");
    settle();
    assert_eq!(std::fs::read(&target).unwrap(), old, "No leaves the file alone");
    assert_eq!(app.count("export phase=writing"), 0, "nothing started");

    // Yes replaces it (the old content is swapped for a marker first, to see the replacement).
    std::fs::write(&target, b"old content").unwrap();
    press(&app, &wid, BTN_RC5);
    app.wait_line(|l| l.contains("export prepare target=rc5 result=plan exists=true"), secs(10));
    app.key(&wid, "Tab Return");
    let l = app.wait_line(|l| l.contains("export confirm answer="), secs(10));
    assert!(l.contains("answer=yes"), "{l}");
    wait_done(&mut app);
    let (_, l, r) = read_wav(&target);
    assert_eq!(l, r);
    assert_eq!(settings(&dirs)["rc5_memory"], 7);
    assert_eq!(snapshot(&dirs.repo()), before, "the repository must not change");
    app.assert_no_real_audio();
    app.no_csp_violation();
}

#[test]
#[ignore]
fn export_rc5_refusals() {
    if !ready() {
        return;
    }
    let dirs = export_dirs("export_rc5_refusals");
    let plain = dirs.base().join("plain-drive");
    std::fs::create_dir_all(&plain).unwrap();
    let ans = answers(&dirs, &[format!("export-rc5 {}", plain.display())]);
    let (mut app, wid) = start(&dirs, &ans, &[]);
    pick(&mut app, &wid, "stem mix", EX_STEM);
    target_line(&mut app, EX_STEM, "track");

    set_memory(&app, &wid, "abc");
    press(&app, &wid, BTN_RC5);
    app.wait_line(|l| l.contains("export prepare target=rc5 result=refused"), secs(10));
    settle();
    shot("export-refused-memory");
    assert!(lines_with(&app, "dialog start").is_empty(), "a bad memory opens no dialog");
    assert!(lines_with(&app, "dialog kind=").is_empty());

    set_memory(&app, &wid, "5");
    press(&app, &wid, BTN_RC5);
    app.wait_line(|l| l.contains("dialog start kind=export-rc5"), secs(10));
    app.wait_line(|l| l.contains("export prepare target=rc5 result=refused"), secs(10));
    settle();
    shot("export-refused-no-roland");
    assert_eq!(std::fs::read_dir(&plain).unwrap().count(), 0, "nothing created on the drive");
    if dirs.app_config().join("settings.json").exists() {
        assert!(settings(&dirs).get("rc5_dir").is_none_or(|v| v.is_null()), "a refused drive is not remembered");
    }
    assert_eq!(app.count("export phase="), 0);
    app.no_csp_violation();
}

#[test]
#[ignore]
fn export_mp3_and_ogg() {
    if !ready() {
        return;
    }
    let dirs = export_dirs("export_mp3_and_ogg");
    let before = snapshot(&dirs.repo());
    let dest = dirs.base().join("music");
    std::fs::create_dir_all(&dest).unwrap();
    let ans = answers(&dirs, &[format!("export-audio {}", dest.display()), format!("export-audio {}", dest.display())]);
    let (mut app, wid) = start(&dirs, &ans, &[]);
    pick(&mut app, &wid, "stem mix", EX_STEM);
    target_line(&mut app, EX_STEM, "track");

    press(&app, &wid, BTN_MP3);
    let l = app.wait_line(|l| l.contains("dialog start kind=export-audio"), secs(10));
    assert!(!l.contains("from=last"), "{l}");
    wait_done(&mut app);
    settle();
    let mp3 = dest.join("The Example Band - Stem Mix.mp3");
    assert!(mp3.is_file());
    let t = tags(&mp3);
    assert_eq!(t["title"], "Stem Mix");
    assert_eq!(t["artist"], "The Example Band");
    assert_eq!(t["album"], "Export Tests");
    assert_eq!(t["track"], "3");
    assert!(t["date"].as_str().unwrap().starts_with("2021"), "{t:?}");
    let dur: f64 = out("ffprobe", &["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", &mp3.display().to_string()])
        .parse()
        .unwrap();
    assert!((dur - 6.5).abs() < 0.2, "4 s + 2.5 s count-in, got {dur}");

    press(&app, &wid, BTN_OGG);
    let l = app.wait_line(|l| l.contains("dialog start kind=export-audio"), secs(10));
    assert!(l.contains("from=last"), "{l}");
    app.wait_line(|l| l.contains("export done path="), secs(30));
    settle();
    shot("export-ogg-done");
    let ogg = dest.join("The Example Band - Stem Mix.ogg");
    assert!(ogg.is_file());
    let t = tags(&ogg);
    assert_eq!(t["title"], "Stem Mix");
    assert_eq!(t["artist"], "The Example Band");
    assert_eq!(t["album"], "Export Tests");
    assert_eq!(t["track"], "3");
    assert_eq!(settings(&dirs)["export_dir"], dest.display().to_string());
    assert!(settings(&dirs).get("rc5_memory").is_none_or(|v| v.is_null()), "only RC-5 exports set the memory");
    let mut names: Vec<_> = std::fs::read_dir(&dest).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    names.sort();
    assert_eq!(names, ["The Example Band - Stem Mix.mp3", "The Example Band - Stem Mix.ogg"]);
    assert_eq!(snapshot(&dirs.repo()), before, "the repository must not change");
    app.assert_no_real_audio();
    app.no_csp_violation();
}

/// Makes a folder writable again when dropped, so the next run can delete it.
struct Restore(PathBuf);
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

#[test]
#[ignore]
fn export_cancel_and_failure() {
    if !ready() {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dirs = export_dirs("export_cancel_and_failure");
    let before = snapshot(&dirs.repo());
    let dest = dirs.base().join("music");
    let ro = dirs.base().join("readonly");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::create_dir_all(&ro).unwrap();
    let _restore = Restore(ro.clone());
    let ans = answers(&dirs, &[format!("export-audio {}", dest.display()), format!("export-audio {}", ro.display())]);
    // The export takes as long as the audio (8.4 s): time to look at it and cancel.
    let (mut app, wid) = start(&dirs, &ans, &[("CALLIOPE_E2E_EXPORT_PACE", "1")]);
    pick(&mut app, &wid, "long intro", EX_LONG);
    target_line(&mut app, EX_LONG, "track");
    press(&app, &wid, BTN_MP3);
    app.wait_line(|l| l.contains("export phase=writing"), secs(15));
    sleep_ms(1500);
    shot("export-running");
    // The focus moved to Cancel when the export started.
    app.key(&wid, "Return");
    app.wait_line(|l| l.contains("export phase=cancelled"), secs(10));
    settle();
    shot("export-cancelled");
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0, "nothing left in the destination");
    assert_eq!(app.count("export done path="), 0);

    // A read-only destination: the failure is shown with the system's reason.
    if out("id", &["-u"]) == "0" {
        println!("skipping the read-only part: running as root");
    } else {
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        press(&app, &wid, BTN_MP3);
        let l = app.wait_line(|l| l.contains("export failed message=") || l.contains("result=refused"), secs(30));
        assert!(l.contains("Permission denied") || l.contains("ermission"), "{l}");
        settle();
        shot("export-failed");
        assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 0);
    }
    assert_eq!(snapshot(&dirs.repo()), before, "the repository must not change");
    app.assert_no_real_audio();
    app.no_csp_violation();
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
