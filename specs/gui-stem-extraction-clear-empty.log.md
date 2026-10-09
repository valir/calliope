base: 64e3ec4
spec: specs/gui-stem-extraction.md (delta: requirement 8 and its criterion, empty stems); branch spec/gui-stem-extraction-clear-empty
task 1 | Quiet stem fixtures | local (edits; orchestrator ran the script) | done
task 2 | Stub separator modes sparse and silent | local (orchestrator fixed a quoting bug and the header comment) | done
task 3 | The measure: silent_peak + SILENT_STEM_DBFS | implementer | done (plan off by one at the 24-bit boundary: 26527 is -50.00004 dBFS, silent; 26528 first audible)
task 4 | Staging::discard_stem_part | implementer | done
task 5 | Import job: check, drop, log, all-silent failure, dropped in the result | implementer | done
task 6 | Frontend: dropped stems on the Import finished page and in the log | implementer | done
task 7 | GUI e2e import_drops_silent_stems | implementer | done
reviewer | APPROVED (0 blockers, 0 majors; 2 minors: no cancel check inside silent_peak, no job-level test of an undecodable stem) | done
tester | 24 acceptance checks PASS, no defects (acceptance_silent_stems.rs, acceptance_silent_stems.test.ts) | done
