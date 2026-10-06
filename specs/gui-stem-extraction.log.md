base: 0a8892c
task 1 | Import fixtures (ffmpeg-generated, 312 KB) | implementer | done
task 2 | Cargo workspace and calliope-common (+ crate-separation test) | implementer | done
  - for task 23: licence_record_lists_every_direct_dependency reads only the root Cargo.toml; extend it to workspace members
task 3 | track_meta schema v2 and in-memory migration | implementer | done (committed with task 4: shared files)
task 4 | Repository v2 (stems, original, export, never-rewrite byte check, library-v2 fixture) | implementer | done
task 5 | Import temp space and staged track creation | implementer | done (committed with 6-7: shared main.rs)
task 6 | tools module (discovery, versions) | implementer | done (unparseable version = usable with warning)
task 7 | media module (ffprobe metadata, FLAC conversion, 15-min limit, 1 GiB cap) | implementer | done
task 8 | Fake yt-dlp and download module | implementer | done (committed with 9: shared tests/support/README.md)
  - manual check: confirm real yt-dlp writes info.info.json for --output infojson:info (fake mimics it; run_download renames to info.json)
task 9 | Stub separator + real-model adapter (syntax-checked only, never run) | implementer | done (~/edge-ai untouched)
task 10 | calliope-stems server (tiny_http, workspace member) | implementer | done (30 tests incl. smoke on 127.0.0.1 with stub)
  - limit: no upload read timeout (tiny_http); a stalled client holds a queue slot until it disconnects
task 11 | Stems client + protocol conformance suite (real binary, stub, 127.0.0.1) | implementer | done (15 conformance tests, 3 runs green)
task 12 | Settings edge_ai_url (http only, no TLS client) and keep_original | implementer | done
