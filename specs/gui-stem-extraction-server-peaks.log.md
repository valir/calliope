base: f62ecf9
spec: specs/gui-stem-extraction.md (delta: requirement 9 and its criterion, server-reported peaks); branch spec/gui-stem-extraction-server-peaks
task 1 | Shared peak scan calliope_lib::flac_peak | implementer | done (32-bit FLAC: neither flacenc nor claxon 0.4 supports it; arithmetic tested, a real 32-bit stem is undecodable and therefore kept)
task 2 | App silent_peak uses the shared scan; level_of_peak | implementer | done
task 3 | stem_peaks in the protocol: StemPeak, JobStatus.stem_peaks, client validation | implementer | done (flake seen once: acceptance_silent_stems cancel_while_the_check_runs_leaves_nothing, passed 3/3 alone)
task 4 | Stub separator mode undecodable | local (orchestrator fixed a doubled backslash) | done
