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
