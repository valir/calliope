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
