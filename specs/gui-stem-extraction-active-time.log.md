base: c969054
spec: specs/gui-stem-extraction.md (delta: requirement 9 rewritten, empty = audible < 15 s above -40 dBFS in 100 ms steps; criteria replaced); branch spec/gui-stem-extraction-active-time
task 1 | Fixtures: 16 s canned stems and the activity stems | implementer (local model ignored the task twice) | done (16 s stems > 32 KB made tiny_http serve chunked: three stems-crate raw test clients now dechunk; lib fixtures_streaminfo asserts 16 s; audible times checked independently)
task 2 | Stub separator mode boundary | implementer | done
task 3 | calliope_lib::flac_level | implementer | done
task 4 | Protocol: StemLevel alongside stem_peaks; client validates levels/peaks only on done | implementer | done (flake once: acceptance_stem_extraction url_flow_end_to_end_with_edits_and_temp_removal missed the 'working' phase under load; 3/3 alone, suite green on rerun)
task 5 | Server measures audible time; --no-stem-levels | implementer | done
task 6 | calliope-stems --measure FILE... | implementer | done (matches the orchestrator's independent numbers on both real tracks exactly; flake once: import_job cancel_during_extraction_goes_back_to_the_edit_pane, 6/6 on rerun)
task 7 | The app judges by audible time (stem_levels, check_stem/check_reported; sparse stub = silent/bursts/phrases; peak-rule tests ported or deleted; acceptance_server_peaks.rs -> acceptance_server_levels.rs) | implementer | done
task 8 | Frontend wording: Dropped empty stems, DroppedStem {name, audible_ms} | implementer (local model wrote a summary instead of edits) | done (flake once under parallel load: calliope-stems aborted_uploads_release_their_queue_slot_and_folder, passed on rerun)
task 9 | Remove stem_peaks and flac_peak (stems acceptance_server_peaks.rs -> acceptance_server_levels.rs) | implementer | done
task 10 | GUI e2e: import_drops_empty_stems (server log line), import_boundary_stems | implementer | done (flake once: import_job empty_stems_are_dropped_from_the_saved_track 'no saved event'; GUI suites all green)
reviewer | APPROVED (0 blockers, 0 majors; minors: no 32-bit full-scale lib test, no cancel inside one stem's scan, old 8/24-bit acceptance boundaries not ported, tracks < 15 s are all-empty by req 9) | done
tester | 22 acceptance checks PASS (independent ffmpeg oracle), no production defects; 4 flakes traced to test-harness timing (fast stub skips Working; snapshot before terminal event; 503 before aborted uploads are read) | done
fix round 1 | Test-harness races (fast stub skips Working; wait for terminal event; retry 503 after aborted uploads); 8/24-bit full-scale test, 32-bit arithmetic test; plan text dropped=piano,other | implementer | done (20/20 under load each; headless 2476 passed)
reviewer (fix round) | APPROVED (1 minor: muddled 8-bit test comment) | done
report | specs/gui-stem-extraction-active-time.report.md | done
