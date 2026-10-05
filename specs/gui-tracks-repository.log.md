base: f7894d9
task 1 | fsutil (atomic write, safe copy, name validation, trash) | implementer | done (orchestrator settled trash layout: track dir -> trash/<stamp>-<id>/, one trash/<stamp>-<id>-tablatures/ per save)
task 2 | track_meta (schema v1, validation, timestamps, ids) | implementer | done (orchestrator: lenient read / strict write for hand-made files)
task 3 | repository (scan, save transaction, delete, export) | implementer | done (27 temp-repo tests; orchestrator audited every remove/rename: only own copies/part/tmp files)
task 4 | Sample fixture tests/fixtures/library-sample (6 invented tracks) | implementer | done
task 5 | Settings repository_root | implementer | done
task 6 | Picker (dialog plugin, token registry, scripted picker, release guard) | implementer | done (guard verified: release+e2e-hooks fails)
task 7 | IPC commands, app state, ACL manifest and capability | implementer | done (test:gui on :1 passes under the new capability; real ~/.local/share/calliope untouched)
task 8 | Frontend ipc.ts and its test | implementer (local failed: output limit) | done
