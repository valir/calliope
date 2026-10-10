base: 09d92c2
spec: specs/gui-existing-track-import.md; branch spec/gui-existing-track-import
decision | owner: video with several audio tracks keeps the first audio track only (one stem track); other options are a follow-up | done
task 1 | local_source: path clean-up and checks | implementer | done (acceptance test files that include picker.rs via #[path] now also include local_source.rs; file: -> / -> folder; '' -> invalid)
task 2 | pick_import_file and import_path replace import_file | implementer | done
