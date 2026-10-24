//! Library GUI end-to-end tests. They need an X display and are run with:
//! `npm run test:gui` (or
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
    dirs.json(id)["tablatures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

/// Files in every `trash/<stamp>-<id>-tablatures/` folder of the track, sorted.
fn trashed_tablatures(dirs: &Dirs, id: &str) -> Vec<String> {
    let mut v = Vec::new();
    for t in dirs
        .trash()
        .iter()
        .filter(|n| n.ends_with(&format!("{id}-tablatures")))
    {
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
    settle();
    // Changing the album is a rename for all its tracks: confirm with Yes (No is focused).
    app.key(&wid, "Tab Return");
    wait_log(&app, "rename kind=album", 1);
    app.wait_contains(&format!("saved id={ID1}"));
    settle();
    let v = dirs.json(ID1);
    assert_eq!(v["album"], "Brand New Album");
    assert_eq!(v["band"], "Amber Fields");
    assert_eq!(v["imported"], FIXTURE_TIME, "imported must be preserved");
    assert!(
        v["modified"].as_str().unwrap() > FIXTURE_TIME,
        "modified not updated: {}",
        v["modified"]
    );
    assert_eq!(
        v["tablatures"],
        serde_json::json!(["slow-burn.gp5", "slow-burn-solo.gp"])
    );
    assert_ne!(
        std::fs::read_to_string(dirs.track_json(ID1)).unwrap(),
        before
    );
    shot("library-saved");

    // A hand edit while in edit mode makes Save refuse and leaves the file as the owner wrote it.
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID1}"));
    click_band_label(&wid);
    app.key(&wid, "Tab ctrl+a");
    app.typ(&wid, "Never Saved");
    settle();
    shot("library-conflict-draft");
    let hand = std::fs::read_to_string(dirs.track_json(ID1))
        .unwrap()
        .replace("Brand New Album", "Hand Edited");
    std::fs::write(dirs.track_json(ID1), &hand).unwrap();
    app.key(&wid, "ctrl+s");
    settle();
    app.key(&wid, "Tab Return"); // confirm the band rename (Yes); the save then hits the conflict
    let l = app.wait_contains("error save_track");
    assert!(l.contains("conflict"), "{l}");
    settle();
    assert_eq!(
        std::fs::read_to_string(dirs.track_json(ID1)).unwrap(),
        hand,
        "conflicting save changed the file"
    );
    shot("library-conflict");
    // The draft is kept (still in edit mode, nothing reloaded); Escape discards and reloads.
    let views_before = app
        .all_lines()
        .iter()
        .filter(|l| l.contains(&format!("mode=view id={ID1}")))
        .count();
    app.key(&wid, "Escape");
    app.wait_line(
        |l| l.contains(&format!("mode=view id={ID1}")),
        Duration::from_secs(10),
    );
    settle();
    let views_after = app
        .all_lines()
        .iter()
        .filter(|l| l.contains(&format!("mode=view id={ID1}")))
        .count();
    assert_eq!(
        views_after,
        views_before + 1,
        "Escape after a conflict should leave edit mode once"
    );
    assert_eq!(
        std::fs::read_to_string(dirs.track_json(ID1)).unwrap(),
        hand,
        "discarding changed the file"
    );
    shot("library-conflict-discarded");
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
        &[
            format!("add-tablature {}", src.display()),
            format!("update-tablature {}", src2.display()),
        ],
    );
    let (mut app, wid, _) = App::start_lib(&dirs, &[("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)]);
    app.select_by_search(&wid, "after midnight", ID2);
    assert!(tablatures(&dirs, ID2).is_empty());
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    // Band, Album, Title, Composers, Year, Source, Copyright, 6 read-only (ID, Type, Position, Audio, Imported, Modified), list box, Add.
    app.key(&wid, &"Tab ".repeat(15));
    app.key(&wid, "Return");
    app.wait_contains("dialog kind=add-tablature result=picked");
    app.wait_contains("tab-staged add name=riff.gp5");
    settle();
    shot("library-tab-added");
    app.key(&wid, "ctrl+s");
    app.wait_contains(&format!("saved id={ID2}"));
    settle();
    assert_eq!(tablatures(&dirs, ID2), vec!["riff.gp5"]);
    assert_eq!(
        std::fs::read(dirs.track_dir(ID2).join("riff.gp5")).unwrap(),
        b"GP5-bytes"
    );
    assert!(src.is_file(), "the picked source must stay untouched");
    assert_eq!(std::fs::read(&src).unwrap(), b"GP5-bytes");

    // Update it with another file: the old one goes to the trash.
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    click_band_label(&wid);
    app.key(&wid, &"Tab ".repeat(13)); // the list box (Band is focused; 12 fields incl. Type and Position, then it)
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
    assert_eq!(
        std::fs::read(dirs.track_dir(ID2).join("riff2.gp5")).unwrap(),
        b"v2-bytes"
    );
    assert!(!dirs.track_dir(ID2).join("riff.gp5").exists());
    let old = trashed_tablatures(&dirs, ID2);
    assert_eq!(
        old,
        vec!["riff.gp5"],
        "old file must be in trash/*-tablatures/"
    );
    assert!(
        src.is_file() && src2.is_file(),
        "the picked sources must stay untouched"
    );

    // Remove it.
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    click_band_label(&wid);
    app.key(&wid, &"Tab ".repeat(13)); // the list box (Band is focused; 12 fields incl. Type and Position, then it)
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
    assert_eq!(
        trashed_tablatures(&dirs, ID2),
        vec!["riff.gp5", "riff2.gp5"]
    );
    assert!(src.is_file() && src2.is_file());
    assert_eq!(
        list_names(&dirs.track_dir(ID2)),
        vec!["backing.mp3", "track.json"]
    );
    app.no_csp_violation();
}

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let end = std::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if std::time::Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn dialog_open() -> bool {
    let o = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "^Add tablature$"])
        .output()
        .unwrap();
    !String::from_utf8_lossy(&o.stdout).trim().is_empty()
}

fn dialog_visible(wid: &str) -> bool {
    Command::new("xdotool")
        .args(["getwindowgeometry", wid])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("Geometry"))
        .unwrap_or(false)
        && dialog_open()
}

#[test]
#[ignore]
fn add_opens_real_file_dialog() {
    if !ready() {
        return;
    }
    let dirs = lib_dirs("add_opens_real_file_dialog");
    // HOME points at an empty temp folder so the real dialog never lists the real $HOME.
    let home = dirs.base().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // X authorisation normally lives in the real $HOME; keep pointing at it (path only).
    let xauth = std::env::var_os("XAUTHORITY")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".Xauthority")))
        .unwrap_or_default();
    let (mut app, wid, _) = App::start_lib(
        &dirs,
        &[("HOME", home.as_path()), ("XAUTHORITY", xauth.as_path())],
    );
    app.select_by_search(&wid, "after midnight", ID2);
    let json_before = std::fs::read(dirs.track_json(ID2)).unwrap();
    app.key(&wid, "ctrl+e");
    app.wait_contains(&format!("mode=edit id={ID2}"));
    app.key(&wid, &"Tab ".repeat(15));
    app.key(&wid, "Return");
    let dlg = out(
        "xdotool",
        &["search", "--sync", "--name", "^Add tablature$"],
    );
    let dlg = dlg.lines().next().expect("dialog window").to_string();
    // Poll until the dialog is mapped and has a size, instead of a fixed sleep.
    assert!(
        wait_until(Duration::from_secs(10), || dialog_visible(&dlg)),
        "dialog window never became visible"
    );
    settle();
    shot_of("library-dialog", "Add tablature");
    app.key(&dlg, "Escape");
    app.wait_contains("dialog kind=add-tablature result=cancelled");
    // The dialog is gone.
    assert!(
        wait_until(Duration::from_secs(10), || !dialog_open()),
        "dialog window still open"
    );
    settle();
    shot("library-dialog-cancelled");
    assert!(
        !app.all_lines().iter().any(|l| l.contains("tab-staged")),
        "nothing must be staged"
    );
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
    // The 13 fields (with Type and Position), Edit, Export, Delete.
    app.key(&wid, &"Tab ".repeat(16));
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
    assert!(
        trash
            .iter()
            .any(|n| n.ends_with(ID4) && !n.ends_with("-tablatures")),
        "{trash:?}"
    );
    let t = trash.iter().find(|n| n.ends_with(ID4)).unwrap();
    assert!(dirs
        .repo()
        .join("trash")
        .join(t)
        .join("track.json")
        .is_file());
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
    copy_dir(&dirs.track_dir(ID6), &second.join("tracks").join(ID6));
    let ans = answers(&dirs, &[format!("repository-root {}", second.display())]);
    let (mut app, wid, lib) = App::start_lib(&dirs, &[("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)]);
    assert_eq!(tracks_in(&lib), 6);
    app.key(&wid, "alt+7");
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
    let settings =
        std::fs::read_to_string(dirs.app_config().join("settings.json")).expect("settings.json");
    assert!(
        settings.contains("repository_root") && settings.contains(second.to_str().unwrap()),
        "{settings}"
    );
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
    // The pane's field area scrolls (it is taller than the window since the Type row was added)
    // and keeps its offset between edits: scroll it to the top first.
    let s = Command::new("xdotool")
        .args([
            "mousemove",
            "--window",
            wid,
            "900",
            "300",
            "click",
            "--repeat",
            "8",
            "4",
        ])
        .status()
        .unwrap();
    assert!(s.success());
    sleep_ms(300);
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

// ---- reordering, deleting and renaming from the tree (gui-library-album-order) ----

/// The tree at 1280x800 with everything expanded: row `i` (0 = the first band) is 36 px high and
/// its centre is at `row_y(i)`; clicks and drags stay inside the 320 px tree column.
const TREE_X: i32 = 420;
fn row_y(i: i32) -> i32 {
    180 + 36 * i
}
/// The upper half of row `i` (the drop gap before it).
fn upper(i: i32) -> i32 {
    row_y(i) - 9
}
/// The toolbar's Delete icon button.
const TREE_DELETE: (i32, i32) = (549, 88);

/// Waits until at least `n` log lines contain `what`.
fn wait_log(app: &App, what: &str, n: usize) {
    assert!(
        wait_until(Duration::from_secs(15), || app.count(what) >= n),
        "no {n}x {what:?}; got {:?}",
        app.all_lines()
    );
}

/// Ctrl+F, Shift+Tab, Enter: "Expand all".
fn expand_all(app: &App, wid: &str) {
    app.key(wid, "ctrl+f");
    app.key(wid, "shift+Tab Return");
    settle();
}

fn positions(dirs: &Dirs, ids: &[&str]) -> Vec<Option<u64>> {
    ids.iter().map(|id| dirs.positions(id).0).collect()
}

fn files(dirs: &Dirs, ids: &[&str]) -> Vec<Vec<u8>> {
    ids.iter()
        .map(|id| std::fs::read(dirs.track_json(id)).unwrap())
        .collect()
}

#[test]
#[ignore]
fn library_reorder_tracks() {
    if !ready() {
        return;
    }
    let dirs = seed_dirs(
        "library_reorder_tracks",
        &[
            seed("t-a", "Band", "Order", "A"),
            seed("t-b", "Band", "Order", "B"),
            seed("t-c", "Band", "Order", "C"),
            seed("t-d", "Other", "Elsewhere", "D"),
        ],
    );
    let abc = ["t-a", "t-b", "t-c"];
    let (mut app, wid, lib) = App::start_lib(&dirs, &[]);
    assert_eq!(tracks_in(&lib), 4, "{lib}");
    expand_all(&app, &wid);
    shot("library-order-initial");
    assert_eq!(positions(&dirs, &abc), [None, None, None]);

    // Drag C to the upper half of A: the marker shows while dragging.
    app.drag_xy(&wid, (TREE_X, row_y(4)), (TREE_X, upper(2)), || {
        wait_log(&app, "drag target key=t:t-c gap=0", 1);
        settle();
        shot("library-order-drag");
    });
    wait_log(&app, "reorder tracks album=", 1);
    assert!(
        app.all_lines()
            .iter()
            .any(|l| l.contains("id=t-c from=3 to=1 via=drag written=3 failed=0")),
        "{:?}",
        app.all_lines()
    );
    settle();
    assert_eq!(
        positions(&dirs, &abc),
        [Some(2), Some(3), Some(1)],
        "C=1, A=2, B=3"
    );
    shot("library-order-dragged");

    // Click A (now the second track), Alt+Up: A moves to the top and keeps the focus.
    app.click(&wid, TREE_X, row_y(3));
    app.wait_contains("select id=t-a");
    app.key(&wid, "alt+Up");
    wait_log(&app, "reorder tracks album=", 2);
    assert!(
        app.all_lines()
            .iter()
            .any(|l| l.contains("id=t-a from=2 to=1 via=key")),
        "{:?}",
        app.all_lines()
    );
    settle();
    assert_eq!(
        positions(&dirs, &abc),
        [Some(1), Some(3), Some(2)],
        "A=1, C=2, B=3"
    );
    let fixed = files(&dirs, &abc);
    // The same key on the first track changes nothing, and the focus is still on A.
    app.key(&wid, "alt+Up");
    wait_log(&app, "reorder none key=t:t-a via=key reason=first", 1);
    settle();
    assert_eq!(files(&dirs, &abc), fixed);
    shot("library-order-first");

    // A dragged onto another album's row changes nothing.
    app.drag_xy(&wid, (TREE_X, row_y(2)), (TREE_X, row_y(7)), || {});
    wait_log(&app, "reorder none key=t:t-a via=drag reason=outside", 1);
    settle();
    assert_eq!(files(&dirs, &abc), fixed);

    // With search text the move keys do nothing.
    app.key(&wid, "ctrl+f");
    app.typ(&wid, "a");
    settle();
    app.key(&wid, "Down Down Down");
    app.key(&wid, "alt+Down");
    wait_log(&app, "reorder none", 3);
    assert!(app.count("reason=search") >= 1, "{:?}", app.all_lines());
    assert_eq!(files(&dirs, &abc), fixed);
    assert_eq!(app.count("reorder tracks"), 2);
    shot("library-order-search");
    app.no_csp_violation();

    // After a restart (light theme): "1. A", "2. C", "3. B".
    app.close_gracefully(&wid);
    drop(app);
    std::fs::write(
        dirs.app_config().join("settings.json"),
        r#"{"theme":"light"}"#,
    )
    .unwrap();
    let (app, wid, _) = App::start_lib(&dirs, &[]);
    expand_all(&app, &wid);
    shot("library-order-restarted");
    // The marker in the light theme: B dragged to the top.
    app.drag_xy(&wid, (TREE_X, row_y(4)), (TREE_X, upper(2)), || {
        wait_log(&app, "drag target key=t:t-b gap=0", 1);
        settle();
        shot("library-order-drag-light");
    });
    wait_log(&app, "reorder tracks album=", 1);
    settle();
    assert_eq!(
        positions(&dirs, &abc),
        [Some(2), Some(1), Some(3)],
        "B=1, A=2, C=3"
    );
    // The minimum window size: toolbar, numbers and the marker line.
    size(&wid, 1024, 640);
    settle();
    shot("library-order-min");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn library_reorder_albums() {
    if !ready() {
        return;
    }
    let pos = |id, band, album, title, p| Seed {
        position: Some(p),
        ..seed(id, band, album, title)
    };
    let dirs = seed_dirs(
        "library_reorder_albums",
        &[
            pos("t-a1", "Band", "Alpha", "a-one", 1),
            pos("t-a2", "Band", "Alpha", "a-two", 2),
            pos("t-b1", "Band", "Beta", "b-one", 1),
            pos("t-b2", "Band", "Beta", "b-two", 2),
            pos("t-g1", "Band", "Gamma", "g-one", 1),
            pos("t-g2", "Band", "Gamma", "g-two", 2),
            seed("t-n1", "Band", "", "loose"),
            seed("t-o1", "Other", "Elsewhere", "o-one"),
        ],
    );
    let album_pos = |dirs: &Dirs| -> Vec<Option<u64>> {
        ["t-a1", "t-b1", "t-g1"]
            .iter()
            .map(|id| dirs.positions(id).1)
            .collect()
    };
    let tracks = ["t-a1", "t-a2", "t-b1", "t-b2", "t-g1", "t-g2"];
    let (mut app, wid, _) = App::start_lib(&dirs, &[]);
    expand_all(&app, &wid);
    shot("library-albums-initial");

    // Rows: 0 Band, 1 Alpha, 2-3, 4 Beta, 5-6, 7 Gamma, 8-9, 10 (no album), 11, 12 Other, 13, 14.
    app.drag_xy(&wid, (TREE_X, row_y(7)), (TREE_X, upper(1)), || {
        wait_log(&app, "drag target key=", 1);
        settle();
        shot("library-albums-drag");
    });
    wait_log(&app, "reorder albums band=", 1);
    assert!(
        app.all_lines()
            .iter()
            .any(|l| l.contains("from=3 to=1 via=drag written=6 failed=0")),
        "{:?}",
        app.all_lines()
    );
    settle();
    assert_eq!(
        album_pos(&dirs),
        [Some(2), Some(3), Some(1)],
        "G=1, A=2, B=3"
    );
    assert_eq!(
        positions(&dirs, &tracks),
        [Some(1), Some(2), Some(1), Some(2), Some(1), Some(2)],
        "track order kept"
    );

    // Alt+Down on Alpha (rows: 0 Band, 1 Gamma, 2-3, 4 Alpha): G, B, A.
    app.click(&wid, TREE_X, row_y(14));
    app.wait_contains("select id=t-o1");
    app.key(&wid, "Home Down Down Down Down");
    app.key(&wid, "alt+Down");
    wait_log(&app, "reorder albums band=", 2);
    settle();
    assert_eq!(
        album_pos(&dirs),
        [Some(3), Some(2), Some(1)],
        "G=1, B=2, A=3"
    );
    assert_eq!(
        positions(&dirs, &tracks),
        [Some(1), Some(2), Some(1), Some(2), Some(1), Some(2)]
    );
    shot("library-albums-moved");
    let fixed = files(&dirs, &tracks);

    // Alpha (now row 7) dragged onto the other band's album: nothing changes.
    app.drag_xy(&wid, (TREE_X, row_y(7)), (TREE_X, row_y(13)), || {});
    wait_log(&app, "reason=outside", 1);
    settle();
    assert_eq!(files(&dirs, &tracks), fixed);
    app.no_csp_violation();

    // After a restart: Gamma, Beta, Alpha, "(no album)" last.
    app.close_gracefully(&wid);
    drop(app);
    let (app, wid, _) = App::start_lib(&dirs, &[]);
    expand_all(&app, &wid);
    shot("library-albums-restarted");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn library_delete_from_tree() {
    if !ready() {
        return;
    }
    let dirs = seed_dirs(
        "library_delete_from_tree",
        &[
            seed("t-a", "Band", "Order", "A"),
            seed("t-b", "Band", "Order", "B"),
            seed("t-c", "Band", "Order", "C"),
            seed("t-d", "Band", "Order", "D"),
            seed("t-s", "Band", "Solo", "S"),
            seed("t-e", "Other", "Elsewhere", "E"),
        ],
    );
    let order = ["t-a", "t-b", "t-c", "t-d"];
    let (mut app, wid, lib) = App::start_lib(&dirs, &[]);
    assert_eq!(tracks_in(&lib), 6, "{lib}");
    expand_all(&app, &wid);
    shot("library-delete-initial");

    // A single track: click S (row 7), Delete, Yes.
    app.click(&wid, TREE_X, row_y(7));
    app.wait_contains("select id=t-s");
    app.key(&wid, "Delete");
    settle();
    shot("library-delete-track-confirm");
    app.key(&wid, "Tab Return");
    app.wait_contains("deleted id=t-s");
    let l = app.wait_contains("library root=");
    assert_eq!(tracks_in(&l), 5, "{l}");
    settle();
    assert!(!dirs.track_dir("t-s").exists());
    assert!(
        dirs.trash().iter().any(|n| n.ends_with("t-s")),
        "{:?}",
        dirs.trash()
    );

    // The album "Order" (4 tracks): the toolbar button asks, Escape cancels.
    app.key(&wid, "Home Down");
    app.click(&wid, TREE_DELETE.0, TREE_DELETE.1);
    settle();
    shot("library-delete-album-confirm");
    app.key(&wid, "Escape");
    wait_log(&app, "delete cancelled key=", 1);
    settle();
    assert_eq!(dirs.trash().len(), 1);
    for id in order {
        assert!(dirs.track_dir(id).is_dir());
    }

    // Again, but one track changes on disk before the confirmation: it stays, the others go.
    app.click(&wid, TREE_DELETE.0, TREE_DELETE.1);
    settle();
    // A real change: whitespace and `exports` don't count for the revision.
    let mut changed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dirs.track_json("t-b")).unwrap()).unwrap();
    changed["copyright"] = serde_json::json!("changed on disk");
    let changed = changed.to_string().into_bytes();
    std::fs::write(dirs.track_json("t-b"), &changed).unwrap();
    app.key(&wid, "Tab Return");
    wait_log(
        &app,
        "deleted album band=\"Band\" album=\"Order\" tracks=4 failed=1",
        1,
    );
    settle();
    assert_eq!(
        app.count("calliope-ui: deleted id=t-"),
        4,
        "t-s and three of the album"
    );
    assert!(dirs.track_dir("t-b").is_dir());
    assert_eq!(std::fs::read(dirs.track_json("t-b")).unwrap(), changed);
    for id in ["t-a", "t-c", "t-d"] {
        assert!(!dirs.track_dir(id).exists(), "{id}");
    }
    assert_eq!(dirs.trash().len(), 4, "{:?}", dirs.trash());
    shot("library-delete-album-partial");

    // A band cannot be deleted: the button is disabled and the key does nothing.
    app.key(&wid, "Home");
    settle();
    shot("library-delete-band-disabled");
    app.key(&wid, "Delete");
    wait_log(&app, "reason=band", 1);
    assert_eq!(dirs.trash().len(), 4);

    // The confirmation at the minimum window size.
    size(&wid, 1024, 640);
    app.key(&wid, "Down");
    app.click(&wid, TREE_DELETE.0, TREE_DELETE.1);
    settle();
    shot("library-delete-album-confirm-min");
    app.key(&wid, "Escape");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn library_rename_album_for_all() {
    if !ready() {
        return;
    }
    let pos = |id, album, title, p, ap| Seed {
        position: Some(p),
        album_position: Some(ap),
        ..seed(id, "Metalica", album, title)
    };
    let m = "Masterof Puppets";
    let r = "Ride the Lightning";
    let dirs = seed_dirs(
        "library_rename_album_for_all",
        &[
            pos("t-m1", m, "Battery", 1, 1),
            pos("t-m2", m, "Master of Puppets", 2, 1),
            pos("t-m3", m, "The Thing That Should Not Be", 3, 1),
            pos("t-m4", m, "Welcome Home", 4, 1),
            pos("t-m5", m, "Disposable Heroes", 5, 1),
            pos("t-m6", m, "Leper Messiah", 6, 1),
            pos("t-m7", m, "Orion", 7, 1),
            pos("t-m8", m, "Damage Inc", 8, 1),
            pos("t-r1", r, "Fight Fire with Fire", 1, 2),
            pos("t-r2", r, "Creeping Death", 2, 2),
        ],
    );
    let master: Vec<String> = (1..=8).map(|i| format!("t-m{i}")).collect();
    let master: Vec<&str> = master.iter().map(String::as_str).collect();
    let all = [&master[..], &["t-r1", "t-r2"]].concat();
    let before = files(&dirs, &all);
    let (mut app, wid, _) = App::start_lib(&dirs, &[]);
    app.select_by_search(&wid, "battery", "t-m1");
    app.key(&wid, "ctrl+e");
    app.wait_contains("mode=edit id=t-m1");
    app.key(&wid, "Tab Tab ctrl+a");
    app.typ(&wid, "Master of Puppets");
    settle();
    app.key(&wid, "ctrl+s");
    settle();
    shot("library-rename-confirm");
    // No (Escape): nothing is saved and the pane stays in edit mode.
    let views = app.count("mode=view id=t-m1");
    app.key(&wid, "Escape");
    wait_log(&app, "rename cancelled", 1);
    settle();
    assert_eq!(files(&dirs, &all), before);
    assert_eq!(app.count("mode=view id=t-m1"), views, "still editing");
    shot("library-rename-cancelled");

    // Yes: all 8 tracks move, with the same positions.
    app.key(&wid, "ctrl+s");
    settle();
    app.key(&wid, "Tab Return");
    wait_log(&app, "rename kind=album", 1);
    assert!(
        app.all_lines()
            .iter()
            .any(|l| l.contains("tracks=8") && l.contains("failed=0")),
        "{:?}",
        app.all_lines()
    );
    settle();
    for (i, id) in master.iter().enumerate() {
        let v = dirs.json(id);
        assert_eq!(v["album"], "Master of Puppets", "{id}");
        assert_eq!(v["position"], i as u64 + 1, "{id}");
        assert_eq!(v["album_position"], 1, "{id}");
    }
    assert_eq!(dirs.json("t-r1")["album"], "Ride the Lightning");
    shot("library-rename-album-done");

    // The band: "Metalica" -> "Metallica" moves both albums, orders kept.
    app.key(&wid, "ctrl+e");
    app.wait_line(|l| l.contains("mode=edit id=t-m1"), Duration::from_secs(10));
    click_band_label(&wid);
    app.key(&wid, "ctrl+a");
    app.typ(&wid, "Metallica");
    settle();
    app.key(&wid, "ctrl+s");
    settle();
    shot("library-rename-band-confirm");
    app.key(&wid, "Tab Return");
    wait_log(&app, "rename kind=band", 1);
    assert!(
        app.all_lines()
            .iter()
            .any(|l| l.contains("rename kind=band") && l.contains("tracks=10")),
        "{:?}",
        app.all_lines()
    );
    settle();
    for id in &all {
        assert_eq!(dirs.json(id)["band"], "Metallica", "{id}");
    }
    assert_eq!(positions(&dirs, &["t-m5", "t-r2"]), [Some(5), Some(2)]);
    assert_eq!(dirs.positions("t-r1").1, Some(2));
    shot("library-rename-band-done");
    app.no_csp_violation();
}
