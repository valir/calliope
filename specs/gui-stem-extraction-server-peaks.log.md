base: f62ecf9
spec: specs/gui-stem-extraction.md (delta: requirement 9 and its criterion, server-reported peaks); branch spec/gui-stem-extraction-server-peaks
task 1 | Shared peak scan calliope_lib::flac_peak | implementer | done (32-bit FLAC: neither flacenc nor claxon 0.4 supports it; arithmetic tested, a real 32-bit stem is undecodable and therefore kept)
task 2 | App silent_peak uses the shared scan; level_of_peak | implementer | done
task 3 | stem_peaks in the protocol: StemPeak, JobStatus.stem_peaks, client validation | implementer | done (flake seen once: acceptance_silent_stems cancel_while_the_check_runs_leaves_nothing, passed 3/3 alone)
task 4 | Stub separator mode undecodable | local (orchestrator fixed a doubled backslash) | done
task 5 | The server measures and reports stem peaks; --no-stem-peaks | implementer | done (shutdown during measuring fails the job; measuring 6 test stems takes ~1 ms)
task 6 | Import job skips stems the server reports silent | implementer | done (no separate cancel-right-after-done test; cancel path unchanged)
task 7 | GUI e2e: silent stems never fetched (server request log) | implementer | done
reviewer | APPROVED (0 blockers, 0 majors; minors: record the trust model in architecture.md; commit the tester files) | done
tester | 23 acceptance checks PASS, no production defects; test-harness race found in the wait helpers (import_job_tests.rs, acceptance_silent_stems.rs) | done
