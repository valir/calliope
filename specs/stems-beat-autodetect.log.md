base: f03dcfd
task 1 | Beats types, validation and client | implementer | done (client calls to be covered by task 4 conformance tests)
task 2 | Stub beat tracker | implementer | done
task 3 | calliope-stems beat jobs, routes, flags | implementer | done
task 4 | Conformance and smoke tests for beats, server README | implementer | done
task 5 | Real beat-this adapter, unit example, licence records | implementer | done
task 6 | Tempo map with detected changes | implementer | done
task 7 | player.json schema 2 | implementer | done
task 8 | Beats to tempo sections | implementer | done
task 9 | Mix waveform peaks and debounce | implementer | done
task 10 | Editor waveform scheduling, worker, peaks event | implementer | done
task 11 | Tempo store and Player release | implementer | done (wiring of PlayerTempoStore and editor open via store left to task 13)
task 12 | Detection job and editor integration | implementer | done (incl. fix: a final 1-bar section at the end of the track is now detected)
task 13 | IPC commands, TS wrappers, store/exit wiring | implementer | done
task 14 | Remove the Player Detect button | local (missed one test title; fixed by the orchestrator) | done
task 15 | Remove the Editor BPM & sections tab | local (plan missed App.test.ts; fixed by the orchestrator in a follow-up commit) | done
task 16 | Waveform geometry for sections and drag | implementer | done
task 17 | Shared WaveformView with sections, drag and keys; Player sections and reopen | implementer | done (player e2e click coordinates updated: layout shifted after task 14)
task 18 | Editor frontend state | implementer | done
task 19 | Mix lane UI | implementer | done (4th lane clipped at 1280x800, left for the task 20 visual review)
task 20 | GUI e2e, visual review, docs | implementer | done
reviewer | APPROVED; 9 minors (notably: unit example hard-requires the beat adapter; typed change just before a detected change can make bases non-monotonic; no backend guard on tempo move during detection) | reviewer | done
tester | A1-A17 PASS; D1 set_change_offset(i64::MIN) panics; D2 typed BPM in the stretched beat leaves tick bases out of order (92/5880 sweep cases) | tester | done
fix round 1 | D1, D2 (typed BPM in the stretched beat applies at the detected change; bases kept increasing); missing beat adapter only warns; move refused during detection; release checks writability first; first-waveform retry; beats input.flac removed when done; drag reset when segments shrink | implementer | done
reviewer (fix round 1) | APPROVED; 2 optional minors | reviewer | done
tester (fix round 1) | D1, D2 fixed; no new defects | tester | done
report | written | orchestrator | done
