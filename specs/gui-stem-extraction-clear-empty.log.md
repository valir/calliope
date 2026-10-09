base: 64e3ec4
spec: specs/gui-stem-extraction.md (delta: requirement 8 and its criterion, empty stems); branch spec/gui-stem-extraction-clear-empty
task 1 | Quiet stem fixtures | local (edits; orchestrator ran the script) | done
task 2 | Stub separator modes sparse and silent | local (orchestrator fixed a quoting bug and the header comment) | done
task 3 | The measure: silent_peak + SILENT_STEM_DBFS | implementer | done (plan off by one at the 24-bit boundary: 26527 is -50.00004 dBFS, silent; 26528 first audible)
task 4 | Staging::discard_stem_part | implementer | done
