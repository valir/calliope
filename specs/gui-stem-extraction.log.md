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
task 13 | Import job orchestrator (real calliope-stems + fake yt-dlp + real ffmpeg) | implementer | done (23 tests, 3 runs green)
task 14 | Picker kinds, 12 IPC commands, Channel events, wiring, ACL | implementer | done (26 commands, still no core/plugin permissions; GUI tests green)
  - accepted: root-change vs job-start race window (job keeps its own root copy); Channel adapter + exit hook covered only by task 21 e2e
task 15 | ipc.ts types/wrappers, Library on v2 records | implementer | done (committed with 16-17: shared files)
task 16 | Library badges S/B and stem/original info | implementer | done (screenshot on :1 OK)
task 17 | TrackFields.svelte extraction | implementer | done
  - follow-ups: gui_library_e2e relies on Tab counts (fragile; one run hung 30 min in the real-dialog test); client-side draft does not know original.flac for tablature clash (Rust rejects it)
task 18 | Import view: menu, source page, download, resume prompt | implementer | done (committed with 19: shared files)
task 19 | Import edit pane, extraction progress, done/failed states, re-attach | implementer | done (full import on :1 with local calliope-stems + stub: saved track shown in Library)
