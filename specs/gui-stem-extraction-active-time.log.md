base: c969054
spec: specs/gui-stem-extraction.md (delta: requirement 9 rewritten, empty = audible < 15 s above -40 dBFS in 100 ms steps; criteria replaced); branch spec/gui-stem-extraction-active-time
task 1 | Fixtures: 16 s canned stems and the activity stems | implementer (local model ignored the task twice) | done (16 s stems > 32 KB made tiny_http serve chunked: three stems-crate raw test clients now dechunk; lib fixtures_streaminfo asserts 16 s; audible times checked independently)
task 2 | Stub separator mode boundary | implementer | done
task 3 | calliope_lib::flac_level | implementer | done
task 4 | Protocol: StemLevel alongside stem_peaks; client validates levels/peaks only on done | implementer | done (flake once: acceptance_stem_extraction url_flow_end_to_end_with_edits_and_temp_removal missed the 'working' phase under load; 3/3 alone, suite green on rerun)
task 5 | Server measures audible time; --no-stem-levels | implementer | done
