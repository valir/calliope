base: 0a8892c
task 1 | Import fixtures (ffmpeg-generated, 312 KB) | implementer | done
task 2 | Cargo workspace and calliope-common (+ crate-separation test) | implementer | done
  - for task 23: licence_record_lists_every_direct_dependency reads only the root Cargo.toml; extend it to workspace members
task 3 | track_meta schema v2 and in-memory migration | implementer | done (committed with task 4: shared files)
task 4 | Repository v2 (stems, original, export, never-rewrite byte check, library-v2 fixture) | implementer | done
task 5 | Import temp space and staged track creation | implementer | done (committed with 6-7: shared main.rs)
task 6 | tools module (discovery, versions) | implementer | done (unparseable version = usable with warning)
task 7 | media module (ffprobe metadata, FLAC conversion, 15-min limit, 1 GiB cap) | implementer | done
task 8 | Fake yt-dlp and download module | implementer | done (committed with 9: shared tests/support/README.md)
  - manual check: confirm real yt-dlp writes info.info.json for --output infojson:info (fake mimics it; run_download renames to info.json)
task 9 | Stub separator + real-model adapter (syntax-checked only, never run) | implementer | done (~/edge-ai untouched)
task 10 | calliope-stems server (tiny_http, workspace member) | implementer | done (30 tests incl. smoke on 127.0.0.1 with stub)
  - limit: no upload read timeout (tiny_http); a stalled client holds a queue slot until it disconnects
task 11 | Stems client + protocol conformance suite (real binary, stub, 127.0.0.1) | implementer | done (15 conformance tests, 3 runs green)
task 12 | Settings edge_ai_url (http only, no TLS client) and keep_original | implementer | done
task 13 | Import job orchestrator (real calliope-stems + fake yt-dlp + real ffmpeg) | implementer | done (23 tests, 3 runs green)
task 14 | Picker kinds, 12 IPC commands, Channel events, wiring, ACL | implementer | done (26 commands, still no core/plugin permissions; GUI tests green)
  - accepted: root-change vs job-start race window (job keeps its own root copy); Channel adapter + exit hook covered only by task 21 e2e
task 15 | ipc.ts types/wrappers, Library on v2 records | implementer | done (committed with 16-17: shared files)
task 16 | Library badges S/B and stem/original info | implementer | done (screenshot on :1 OK)
task 17 | TrackFields.svelte extraction | implementer | done
  - follow-ups: gui_library_e2e relies on Tab counts (fragile; one run hung 30 min in the real-dialog test); client-side draft does not know original.flac for tablature clash (Rust rejects it)
task 18 | Import view: menu, source page, download, resume prompt | implementer | done (committed with 19: shared files)
task 19 | Import edit pane, extraction progress, done/failed states, re-attach | implementer | done (full import on :1 with local calliope-stems + stub: saved track shown in Library)
task 20 | Settings Stem extraction + External tools cards, footer edge-AI status | implementer | done (data-unchecked variant mapped + guarded)
  - follow-up: footer edge-AI status not refreshed from job results (plan §2.7)
task 21 | GUI e2e import tests on :1 in a loopback-only namespace (13 tests, disk checks) | implementer | done (2 full runs green; real ~/.config etc. fingerprint unchanged)
  - follow-ups: import e2e navigates by Tab from a fixed click point (log-awaited); added frontend log lines for UI-side import errors/prompts
task 22 | Visual review on :1 | implementer | done (4 fixes; checklist below, all PASS after the fixes)
  - method: temporary scratch test (not committed) ran the real app + calliope-stems stub in unshare -rn, theme dark/light, sizes 1024x640, 1280x800 and maximised 1920x1200; shots in target/gui-shots/review-<theme>-<ok|fail>-<step>-<1024|1280|max>.png, plus Task 21's import-*.png (dark, 1280x800)
  - PASS Import source page (URL / audio), dark+light, 1024+max: target/gui-shots/review-dark-ok-source-url-1024.png, review-light-ok-source-audio-1024.png
  - PASS edit pane, dark+light (scrolls at 1024x640, Extract/Cancel reachable; fully visible at 1280x800 and max): review-dark-ok-edit-1024.png, review-light-ok-edit-1024.png, review-dark-ok-edit-max.png
  - PASS "Working..." spinner + percentage + Cancel, dark+light: review-dark-ok-working-1024.png, review-light-ok-working-1024.png; download progress bar: import-downloading.png; resume prompt: import-resume-prompt.png
  - PASS done state "Saved ... with 6 stems" + Show in Library, dark+light: review-dark-ok-done-1024.png, review-light-ok-done-1024.png
  - FAIL then PASS failed state: (a) light theme: amber-500 error text on white was ~2:1; fixed with text-amber-700 in light, text-primary in dark (import errors, Settings alerts and tool/edge-AI problem lines, "Change in Settings" link): review-light-fail-failed-1280.png; (b) at 1024x640 the message sat below the fold and the user saw only the footer; fixed: ImportEditPane scrolls the message into view (rAF, re-run when the keep-original line appears): review-dark-fail-failed-1024.png, review-light-fail-failed-1024.png
  - PASS Library stem track (S badge, selected row, detail pane), dark+light: review-dark-ok-library-stem-track-1024.png, review-light-ok-library-stem-track-1024.png
  - FAIL then PASS Settings cards (Stem extraction, External tools), dark+light, 1024 scrolled and max: review-dark-ok-settings-scrolled-1024.png, review-light-ok-settings-scrolled-1024.png, review-dark-ok-settings-max.png; the switch's off state was faint on dark cards, fixed with a muted border (switch.svelte)
  - PASS nothing clipped at 1024x640/1280x800/max in any state (only the intended scroll); inline URL error: import-url-invalid.png
  - notes: webview sometimes repaints late after a window resize (a black margin in one max shot: review-dark-fail-failed-max.png), harness timing, not an app issue; light-theme shots of the download/resume prompt were not taken (same components as the checked ones); the fail-state flow was only captured at 1024x640 start size
  - docs/ui.md "Decided by the team" updated with the choices; npm test (588 Rust + 248 vitest) and DISPLAY=:1 npm run test:gui (5+13+8+1) green
task 23 | Licences and READMEs | implementer | done
  - tests/frontend.rs: licence_record_lists_every_direct_dependency now reads every workspace member's Cargo.toml (and [target.*.dependencies]); docs/licences.md already had the crate rows, now also External tools (not bundled), Test-only tools, shadcn progress/switch
  - htdemucs_6s weights licence could not be determined from local files: recorded as "to be confirmed by the owner" (also the Demucs LICENSE file is not shipped inside audio-separator)
  - README.md Import section; src/calliope-stems/README.md; src/calliope-stems/deploy/calliope-stems.service (systemd-analyze verify --user: only "ExecStart not executable", the binary is not installed). Nothing installed or started.
tester | acceptance tests (server 22, hostile client 13, orchestrator/data-safety 198, GUI 4, vitest +37) | all AC PASS; 4 low findings as #[ignore] tests | done
reviewer | 0a8892c..HEAD | CHANGES REQUIRED (1 major: start_extraction wedges job on early error; minors) | done
fix round 1 | start_extraction rollback (major); yt-dlp size/live/duration filter; ffmpeg protocol whitelist + -t; client cancel/poll tolerance; error truncation; no import-tmp on prepare; fragment files; attach error; known limits documented | implementer | done (GUI 2 runs green)
