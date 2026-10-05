base: f7894d9
task 1 | fsutil (atomic write, safe copy, name validation, trash) | implementer | done (orchestrator settled trash layout: track dir -> trash/<stamp>-<id>/, one trash/<stamp>-<id>-tablatures/ per save)
task 2 | track_meta (schema v1, validation, timestamps, ids) | implementer | done (orchestrator: lenient read / strict write for hand-made files)
task 3 | repository (scan, save transaction, delete, export) | implementer | done (27 temp-repo tests; orchestrator audited every remove/rename: only own copies/part/tmp files)
task 4 | Sample fixture tests/fixtures/library-sample (6 invented tracks) | implementer | done
task 5 | Settings repository_root | implementer | done
task 6 | Picker (dialog plugin, token registry, scripted picker, release guard) | implementer | done (guard verified: release+e2e-hooks fails)
task 7 | IPC commands, app state, ACL manifest and capability | implementer | done (test:gui on :1 passes under the new capability; real ~/.local/share/calliope untouched)
task 8 | Frontend ipc.ts and its test | implementer (local failed: output limit) | done
task 9 | Fuzzy matching and tree building (pure TS) | implementer | done
task 10 | Track draft model and button states (pure TS) | implementer | done (remove+re-add same name becomes replace -> old file trashed)
task 11 | shadcn textarea + alert-dialog, ConfirmDialog | implementer | done (no new variant mappings needed)
task 12 | Library left column (toolbar, tree, search, problems) | implementer | done (screenshot on :1 with fixture copy OK)
task 13 | Track pane and Tablature Files panel | implementer | done (27 component tests AC6-AC19; screenshots on :1)
  - for task 16 visual review: disabled primary "Edit" button still reads amber/active next to Save; tree/search have no visible "locked" cue in edit mode
task 14 | Settings Track repository card | implementer | done (switch blocked while editing)
fix | first run: missing DEFAULT root now created via list_tracks (frontend skipped it); e2e checks marker+tracks/ on fresh XDG | implementer | done
task 15 | GUI e2e on :1 (8 tests with disk checks) | implementer | done (2 full runs + 2 extra runs green; orchestrator re-ran: 5+8+1 pass)
  - note: harness duplicated into tests/common/mod.rs (gui_e2e.rs untouched); real-dialog test lists the real $HOME read-only, then Escape
task 16 | Visual review on :1 | implementer | done (all PASS after 5 small CSS fixes; screenshots in /tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/t16-*.png and target/gui-shots/library-*.png)
  - PASS two columns visible, 1024x640 dark+light (t16-dark-min-lib/view/edit, t16-light-min-view/edit)
  - PASS tree readable at 18 px, chevrons clear (t16-dark-min-expanded)
  - PASS selected track amber. FIXED: was grey bg-accent; now amber tint + amber left edge (TrackTree.svelte; same for selected tablature row in TablaturePanel.svelte)
  - PASS read-only vs editable. FIXED: view-mode fields were boxed like edit mode; now borderless/transparent in view mode, boxed in edit mode (TrackPane.svelte viewLook)
  - PASS disabled buttons look disabled. FIXED: disabled primary (Edit/Save) stayed amber at 50%; now muted bg + muted text; disabled outline (Cancel/Export) now muted text, no fill (button.svelte)
  - PASS action bar at the bottom, not cut off at 1024x640 (t16-dark-min-tabs) and maximised 1920x1160 (t16-light-max-edit)
  - PASS tablature list 4 rows + scrollbar for track 5 (t16-dark-min-tabs; scrollbar is faint)
  - PASS "new"/"updated" markers readable. FIXED: now amber (amber-700 in light) medium weight (library-tab-updated.png)
  - PASS confirm dialog centred and readable (library-delete-confirm.png, library-tab-confirm.png)
  - PASS real GTK dialog shows "Tablature files" filter (library-dialog.png)
  - PASS edit mode locked cue. FIXED: toolbar+tree at 50% opacity and hint "Finish or cancel editing to browse" (LibraryView.svelte; library-min-edit.png, t16-light-min-edit)
  - PASS no overlap at 1024x640 / maximised; no CSP violations seen in app stderr
  - note: hint shifts the tree down one line in edit mode (accepted); an empty white tooltip box appeared next to the nav after hovering Playlists in a mis-sized window (t16 scratch run, not Library; unverified, native webkit tooltip)
