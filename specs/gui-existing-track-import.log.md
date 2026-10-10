base: 09d92c2
spec: specs/gui-existing-track-import.md; branch spec/gui-existing-track-import
decision | owner: video with several audio tracks keeps the first audio track only (one stem track); other options are a follow-up | done
task 1 | local_source: path clean-up and checks | implementer | done (acceptance test files that include picker.rs via #[path] now also include local_source.rs; file: -> / -> folder; '' -> invalid)
task 2 | pick_import_file and import_path replace import_file | implementer | done
task 3 | Frontend IPC wrappers | local (code correct; it claimed passing tests while 5 vitest mocks of import_file were stale, a plan file-list gap) + implementer (updated the two test files' mocks) | done
task 4 | The path row in the source page (Browse only fills the box; Extract/Enter imports) | implementer | done
task 5 | Headless acceptance: typed-path audio/video, no-audio, two audio streams, rejected paths | implementer | done
task 6 | GUI e2e: browse helper (Browse fills, Return imports), 4 typed-path tests, screenshots reviewed | implementer | done
task 7 | User docs (README import step 1) | implementer (local model pasted the instructions verbatim; reverted) + orchestrator wording fix | done
reviewer | APPROVED (0 blockers, 0 majors; minors: control chars not re-checked after file:// percent-decoding, file:// query/fragment silently dropped, TOCTOU check-then-open undocumented, empty-box message duplicated in TS) | done
tester | both ACs and req 1-3 PASS (6 headless + 22 vitest new); defects: README non-ASCII ellipsis from task 7 breaks no_non_ascii_in_readme; percent-encoded control chars in file:// not rejected as invalid | done
fix round 1 | README ASCII; file:// control chars/length re-checked after decoding, query/fragment rejected; check-then-open window noted in architecture.md | implementer | done (both defect tests pass; headless suite green)
reviewer (fix round) | APPROVED (nit: pin that plain paths with # or ? stay valid) | done
report | specs/gui-existing-track-import.report.md | done
