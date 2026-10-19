base: ab99cc6
plan | architect: 13 tasks (routine 1, 2, 3, 8, 13); owner: Q1 remember the directory itself, Q2-Q3 as recommended | architect | done
task 1 | Stub separator seq: mode | local (gemma4: correct edits, then 40-turn limit; orchestrator restored a blank line in the README table) | done
task 2 | Settings field last_import_dir | local (gemma4: edits correct but it replaced the existing store_setters test; orchestrator restored it) | done
task 3 | Picker: directory kind, recorded start folders, Arc picker | implementer (local model output broke existing tests, discarded) | done
task 4 | local_source: starting folder, directory checks, audio listing | implementer | done
task 5 | Import job: helpers and batch types (no behaviour change) | implementer | done (committed with task 6: shared files)
task 6 | The batch runner | implementer | done (note for tasks 9-10 added to the plan)
task 7 | IPC: start folder, remembering, import_directory | implementer | done
task 8 | TS IPC types and wrappers | implementer (local model deleted 128 lines of ipc.ts, discarded) | done (committed with tasks 9-10)
task 9 | Import state: directory source and batch events | implementer | done
task 10 | Source page, batch progress and summary | implementer | done (UI names noted in the plan for tasks 11-12)
task 11 | Headless acceptance tests | implementer | done
