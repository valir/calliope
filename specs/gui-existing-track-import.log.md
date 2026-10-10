base: 09d92c2
spec: specs/gui-existing-track-import.md; branch spec/gui-existing-track-import
decision | owner: video with several audio tracks keeps the first audio track only (one stem track); other options are a follow-up | done
task 1 | local_source: path clean-up and checks | implementer | done (acceptance test files that include picker.rs via #[path] now also include local_source.rs; file: -> / -> folder; '' -> invalid)
task 2 | pick_import_file and import_path replace import_file | implementer | done
task 3 | Frontend IPC wrappers | local (code correct; it claimed passing tests while 5 vitest mocks of import_file were stale, a plan file-list gap) + implementer (updated the two test files' mocks) | done
task 4 | The path row in the source page (Browse only fills the box; Extract/Enter imports) | implementer | done
task 5 | Headless acceptance: typed-path audio/video, no-audio, two audio streams, rejected paths | implementer | done
