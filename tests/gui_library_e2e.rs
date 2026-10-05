//! Library GUI end-to-end tests. They need an X display and are run with:
//! `DISPLAY=:1 npm run test:gui` (or
//! `cargo test --features e2e-hooks --test gui_library_e2e -- --ignored --test-threads=1`).
//!
//! Every test works on a copy of `tests/fixtures/library-sample` under `target/gui-e2e/<test>/`
//! (the default root below the temp XDG_DATA_HOME) and checks the files on disk.

mod common;

use common::*;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const FIXTURE_TIME: &str = "2026-10-01T12:00:00Z";

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg")
}

fn answers(dirs: &Dirs, lines: &[String]) -> std::path::PathBuf {
    let p = dirs.base().join("answers");
    std::fs::write(&p, lines.join("\n") + "\n").unwrap();
    p
}

fn list_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

fn tablatures(dirs: &Dirs, id: &str) -> Vec<String> {
    dirs.json(id)["tablatures"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// Files in every `trash/<stamp>-<id>-tablatures/` folder of the track, sorted.
fn trashed_tablatures(dirs: &Dirs, id: &str) -> Vec<String> {
    let mut v = Vec::new();
    for t in dirs.trash().iter().filter(|n| n.ends_with(&format!("{id}-tablatures"))) {
        v.extend(list_names(&dirs.repo().join("trash").join(t)));
    }
    v.sort();
    v
}

#[test]
#[ignore]
fn library_starts_collapsed() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("library_starts_collapsed");
    let (app, wid, lib) = App::start_lib(&dirs, &[]);
    assert_eq!(tracks_in(&lib), 6, "{lib}");
    assert_eq!(field(&lib, "problems"), "0", "{lib}");
    settle();
    shot("library-collapsed");
    // Ctrl+F, Shift+Tab to "Expand all", Enter.
    app.key(&wid, "ctrl+f");
    app.key(&wid, "shift+Tab");
    app.key(&wid, "Return");
    settle();
    shot("library-expanded");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn search_filters() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("search_filters");
    let (app, wid, _) = App::start_lib(&dirs, &[]);
    settle();
    app.key(&wid, "ctrl+f");
    app.typ(&wid, "nght");
    settle();
    shot("library-search");
    // Clear: select all and delete.
    app.key(&wid, "ctrl+a BackSpace");
    settle();
    shot("library-search-cleared");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn edit_and_save() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("edit_and_save");
    let before = std::fs::read_to_string(dirs.track_json(ID1)).unwrap();
    let (mut app, wid, _) = App::start_lib(&dirs, &[]);
    app.select_by_search(&wid, "slow burn", ID1);
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID1}"));
    settle();
    shot("library-edit");
    // Band, then Album.
    app.key(&wid, "Tab Tab ctrl+a");
    app.typ(&wid, "Brand New Album");
    settle();
    app.key(&wid, "ctrl+s");
    app.wait_contains(&format!("saved id={ID1}"));
    settle();
    let v = dirs.json(ID1);
    assert_eq!(v["album"], "Brand New Album");
    assert_eq!(v["band"], "Amber Fields");
    assert_eq!(v["imported"], FIXTURE_TIME, "imported must be preserved");
    assert!(v["modified"].as_str().unwrap() > FIXTURE_TIME, "modified not updated: {}", v["modified"]);
    assert_eq!(v["tablatures"], serde_json::json!(["slow-burn.gp5", "slow-burn-solo.gp"]));
    assert_ne!(std::fs::read_to_string(dirs.track_json(ID1)).unwrap(), before);
    shot("library-saved");

    // A hand edit while in edit mode makes Save refuse and leaves the file as the owner wrote it.
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID1}"));
    click_band_label(&wid);
    app.key(&wid, "Tab ctrl+a");
    app.typ(&wid, "Never Saved");
    settle();
    shot("library-conflict-draft");
    let hand = std::fs::read_to_string(dirs.track_json(ID1)).unwrap().replace("Brand New Album", "Hand Edited");
    std::fs::write(dirs.track_json(ID1), &hand).unwrap();
    app.key(&wid, "ctrl+s");
    let l = app.wait_contains("error save_track");
    assert!(l.contains("conflict"), "{l}");
    settle();
    assert_eq!(std::fs::read_to_string(dirs.track_json(ID1)).unwrap(), hand, "conflicting save changed the file");
    shot("library-conflict");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn tablature_add_and_remove() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("tablature_add_and_remove");
    let src_dir = dirs.base().join("outside");
    std::fs::create_dir_all(&src_dir).unwrap();
    let src = src_dir.join("riff.gp5");
    std::fs::write(&src, b"GP5-bytes").unwrap();
    let src2 = src_dir.join("riff2.gp5");
    std::fs::write(&src2, b"v2-bytes").unwrap();
    let ans = answers(
        &dirs,
        &[format!("add-tablature {}", src.display()), format!("update-tablature {}", src2.display())],
    );
    let (mut app, wid, _) = App::start_lib(&dirs, &[("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)]);
    app.select_by_search(&wid, "after midnight", ID2);
    assert!(tablatures(&dirs, ID2).is_empty());
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    // Band, Album, Title, Composers, Year, Source, Copyright, 4 read-only, list box, Add.
    app.key(&wid, &"Tab ".repeat(13));
    app.key(&wid, "Return");
    app.wait_contains("dialog kind=add-tablature result=picked");
    app.wait_contains("tab-staged add name=riff.gp5");
    settle();
    shot("library-tab-added");
    app.key(&wid, "ctrl+s");
    app.wait_contains(&format!("saved id={ID2}"));
    settle();
    assert_eq!(tablatures(&dirs, ID2), vec!["riff.gp5"]);
    assert_eq!(std::fs::read(dirs.track_dir(ID2).join("riff.gp5")).unwrap(), b"GP5-bytes");
    assert!(src.is_file(), "the picked source must stay untouched");
    assert_eq!(std::fs::read(&src).unwrap(), b"GP5-bytes");

    // Update it with another file: the old one goes to the trash.
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    click_band_label(&wid);
    app.key(&wid, &"Tab ".repeat(11)); // the list box
    app.key(&wid, "Down Tab Tab"); // select the row, then Add, Update
    app.key(&wid, "Return");
    app.wait_contains("dialog kind=update-tablature result=picked");
    app.wait_contains("tab-staged update name=riff2.gp5");
    settle();
    shot("library-tab-updated");
    app.key(&wid, "ctrl+s");
    app.wait_contains(&format!("saved id={ID2}"));
    settle();
    assert_eq!(tablatures(&dirs, ID2), vec!["riff2.gp5"]);
    assert_eq!(std::fs::read(dirs.track_dir(ID2).join("riff2.gp5")).unwrap(), b"v2-bytes");
    assert!(!dirs.track_dir(ID2).join("riff.gp5").exists());
    let old = trashed_tablatures(&dirs, ID2);
    assert_eq!(old, vec!["riff.gp5"], "old file must be in trash/*-tablatures/");
    assert!(src.is_file() && src2.is_file(), "the picked sources must stay untouched");

    // Remove it.
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    click_band_label(&wid);
    app.key(&wid, &"Tab ".repeat(11)); // the list box
    app.key(&wid, "Down");
    settle();
    shot("library-tab-selected");
    // Add, Update, Export, Remove.
    app.key(&wid, "Tab Tab Tab Tab");
    app.key(&wid, "Return");
    settle();
    shot("library-tab-confirm");
    app.key(&wid, "Tab Return"); // No is focused: Tab to Yes
    app.wait_contains("tab-staged remove name=riff2.gp5");
    settle();
    app.key(&wid, "ctrl+s");
    app.wait_contains(&format!("saved id={ID2}"));
    settle();
    assert!(tablatures(&dirs, ID2).is_empty());
    assert!(!dirs.track_dir(ID2).join("riff2.gp5").exists());
    assert_eq!(trashed_tablatures(&dirs, ID2), vec!["riff.gp5", "riff2.gp5"]);
    assert!(src.is_file() && src2.is_file());
    assert_eq!(list_names(&dirs.track_dir(ID2)), vec!["backing.mp3", "track.json"]);
    app.no_csp_violation();
}

#[test]
#[ignore]
fn add_opens_real_file_dialog() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("add_opens_real_file_dialog");
    let (mut app, wid, _) = App::start_lib(&dirs, &[]);
    app.select_by_search(&wid, "after midnight", ID2);
    let json_before = std::fs::read(dirs.track_json(ID2)).unwrap();
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    app.key(&wid, &"Tab ".repeat(13));
    app.key(&wid, "Return");
    let dlg = out("xdotool", &["search", "--sync", "--name", "^Add tablature$"]);
    let dlg = dlg.lines().next().expect("dialog window").to_string();
    settle();
    sleep_ms(800);
    shot_of("library-dialog", "Add tablature");
    app.key(&dlg, "Escape");
    app.wait_contains("dialog kind=add-tablature result=cancelled");
    // The dialog is gone.
    sleep_ms(500);
    let left = Command::new("xdotool").args(["search", "--onlyvisible", "--name", "^Add tablature$"]).output().unwrap();
    assert!(String::from_utf8_lossy(&left.stdout).trim().is_empty(), "dialog window still open");
    settle();
    shot("library-dialog-cancelled");
    assert!(!app.all_lines().iter().any(|l| l.contains("tab-staged")), "nothing must be staged");
    assert_eq!(std::fs::read(dirs.track_json(ID2)).unwrap(), json_before);
    app.no_csp_violation();
}

#[test]
#[ignore]
fn delete_track_to_trash() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("delete_track_to_trash");
    let (mut app, wid, _) = App::start_lib(&dirs, &[]);
    app.select_by_search(&wid, "lanterns", ID4);
    // The 11 fields, Edit, Export, Delete.
    app.key(&wid, &"Tab ".repeat(14));
    app.key(&wid, "Return");
    settle();
    shot("library-delete-confirm");
    app.key(&wid, "Tab Return");
    app.wait_contains(&format!("deleted id={ID4}"));
    let l = app.wait_contains("library root=");
    assert_eq!(tracks_in(&l), 5, "{l}");
    settle();
    assert!(!dirs.track_dir(ID4).exists(), "folder still in tracks/");
    let trash = dirs.trash();
    assert!(trash.iter().any(|n| n.ends_with(ID4) && !n.ends_with("-tablatures")), "{trash:?}");
    let t = trash.iter().find(|n| n.ends_with(ID4)).unwrap();
    assert!(dirs.repo().join("trash").join(t).join("track.json").is_file());
    assert!(dirs.track_dir(ID1).is_dir());
    shot("library-deleted");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn settings_change_root() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("settings_change_root");
    // A second root with only track 6 (no marker, so the app asks before using it).
    let second = dirs.base().join("second-root");
    copy_dir(
        &dirs.track_dir(ID6),
        &second.join("tracks").join(ID6),
    );
    let ans = answers(&dirs, &[format!("repository-root {}", second.display())]);
    let (mut app, wid, lib) = App::start_lib(&dirs, &[("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)]);
    assert_eq!(tracks_in(&lib), 6);
    app.key(&wid, "alt+6");
    app.wait_view("settings");
    settle();
    // The Theme card comes first; "Choose folder" is the first repository button.
    // Tab order: theme radio group, Choose folder, Use default.
    app.key(&wid, "Tab Tab");
    app.key(&wid, "Return");
    app.wait_contains("dialog kind=repository-root result=picked");
    settle();
    shot("library-root-confirm");
    app.key(&wid, "Tab Return");
    app.wait_contains(&format!("repository root={}", second.display()));
    settle();
    let settings = std::fs::read_to_string(dirs.app_config().join("settings.json")).expect("settings.json");
    assert!(settings.contains("repository_root") && settings.contains(second.to_str().unwrap()), "{settings}");
    app.key(&wid, "alt+1");
    let l = app.wait_contains("library root=");
    assert_eq!(tracks_in(&l), 1, "{l}");
    assert!(l.contains(second.to_str().unwrap()), "{l}");
    settle();
    shot("library-second-root");
    // The default root is untouched and the second root now has its marker.
    assert_eq!(list_names(&dirs.repo().join("tracks")).len(), 6);
    assert!(second.join("calliope-repository.json").is_file());
    app.no_csp_violation();
}

#[test]
#[ignore]
fn library_min_size() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("library_min_size");
    let (mut app, wid, _) = App::start_lib(&dirs, &[]);
    size(&wid, 1024, 640);
    let (_, (w, h)) = geometry(&wid);
    assert!(w >= 1024 && h >= 640, "{w}x{h}");
    app.select_by_search(&wid, "open water", ID5);
    settle();
    shot("library-min-view");
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID5}"));
    settle();
    shot("library-min-edit");
    app.no_csp_violation();
}

/// Focuses the Band field by clicking its label (a deterministic Tab start for a 1280x800 window).
fn click_band_label(wid: &str) {
    let s = Command::new("xdotool")
        .args(["mousemove", "--window", wid, "655", "35", "click", "1"])
        .status()
        .unwrap();
    assert!(s.success());
    sleep_ms(300);
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
