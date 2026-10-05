base: f7894d9
task 1 | fsutil (atomic write, safe copy, name validation, trash) | implementer | done (orchestrator settled trash layout: track dir -> trash/<stamp>-<id>/, one trash/<stamp>-<id>-tablatures/ per save)
task 2 | track_meta (schema v1, validation, timestamps, ids) | implementer | done (orchestrator: lenient read / strict write for hand-made files)
