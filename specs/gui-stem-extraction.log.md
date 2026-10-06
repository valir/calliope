base: 0a8892c
task 1 | Import fixtures (ffmpeg-generated, 312 KB) | implementer | done
task 2 | Cargo workspace and calliope-common (+ crate-separation test) | implementer | done
  - for task 23: licence_record_lists_every_direct_dependency reads only the root Cargo.toml; extend it to workspace members
task 3 | track_meta schema v2 and in-memory migration | implementer | done (committed with task 4: shared files)
task 4 | Repository v2 (stems, original, export, never-rewrite byte check, library-v2 fixture) | implementer | done
task 5 | Import temp space and staged track creation | implementer | done (committed with 6-7: shared main.rs)
task 6 | tools module (discovery, versions) | implementer | done (unparseable version = usable with warning)
task 7 | media module (ffprobe metadata, FLAC conversion, 15-min limit, 1 GiB cap) | implementer | done
