base: c969054
spec: specs/gui-stem-extraction.md (delta: requirement 9 rewritten, empty = audible < 15 s above -40 dBFS in 100 ms steps; criteria replaced); branch spec/gui-stem-extraction-active-time
task 1 | Fixtures: 16 s canned stems and the activity stems | implementer (local model ignored the task twice) | done (16 s stems > 32 KB made tiny_http serve chunked: three stems-crate raw test clients now dechunk; lib fixtures_streaminfo asserts 16 s; audible times checked independently)
task 2 | Stub separator mode boundary | implementer | done
