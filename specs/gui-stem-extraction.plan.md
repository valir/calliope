# Plan: stem extraction import (Import view) and the `calliope-stems` server

## 1. Summary

The Import view gets its first import function, **Stem Extraction**. The user picks a source:
a URL (YouTube or any other http/https page that `yt-dlp` supports), a local audio file
(mp3/flac/ogg) or a local video file. Calliope downloads or reads it into a temporary folder
inside the repository, converts it to FLAC with `ffmpeg`, reads its metadata (`ffprobe` /
`yt-dlp`'s info file) into an edit pane, and after the user clicks **Extract** it sends the audio
to the edge-AI server on the LAN. The returned stems are saved as a new track of type `stem`
(optionally with the original mix). For that, `track.json` moves to **schema version 2**;
existing version-1 tracks are migrated in memory and only rewritten when the user saves them.
The Library marks stem tracks with an "S" badge.

The edge-AI side is built in this feature too: a separate Rust binary **`calliope-stems`**
(a workspace member under `src/`, no Tauri) that implements "calliope-stems API v1" and runs a
configurable separator command (on archserver: audio-separator with `htdemucs_6s`). The owner
deploys it by hand. All tests run it on 127.0.0.1 with a stub separator, never with the real model.

## 2. Design

### 2.1 What is reused

- IPC pattern, ACL (three lists kept in sync by `tests/frontend.rs`), `calliope-ui:` log lines,
  `ScriptedPicker` + `e2e-hooks`, release guard, GUI e2e harness (`tests/common/mod.rs`).
- Repository rules: lenient read / strict write, `fsutil::write_atomic`, `copy_no_clobber`,
  `validate_file_name`, `fnv1a64_hex`, revision checks, the `RepoState` mutex, nothing a user
  put in the repository is ever deleted.
- `track_meta::migrate` hook (empty until now), `new_id()` (UUIDv7), `now_rfc3339()`,
  `validate_for_write`, the `Repository::create_track` pattern (its `dead_code` allows go away).
- Frontend: `track-draft.ts` field rules (the import edit pane uses the same validation),
  shadcn button/input/label/radio-group/card/alert-dialog, `ConfirmDialog`.
- The architecture's rule for more crates: "the root becomes a Cargo workspace and the new
  crates go in `src/<crate>/`" (gui-skeleton decision).

### 2.2 Crates and separation (owner decision Q1)

```
Cargo.toml                       root package calliope-gui (unchanged) + [workspace]
                                 members = ["src/calliope-common", "src/calliope-stems"]
                                 (default-members = the root package, so `cargo build` and
                                 `npm run build:app` still build only the GUI)
src/*.rs, src/ui/                calliope-gui (Tauri)
src/calliope-common/             lib crate `calliope_common`, NO Tauri/GTK, no HTTP server
  Cargo.toml                     deps: serde, serde_json, libc; optional ureq (feature "client")
  src/lib.rs
  src/stems_api.rs               API v1 types (Health, JobCreated, JobStatus, JobState, ErrorBody),
                                 constants (API_VERSION=1, MAX_DURATION_S=900, MAX_STEMS=16),
                                 job-id / stem-name validation, FLAC magic + STREAMINFO parser
                                 (sample rate, channels, total samples → duration)
  src/stems_client.rs            the HTTP client (feature "client", ureq 3 without TLS)
  src/process.rs                 child runner: argv only, line readers, own process group,
                                 PDEATHSIG, cancel (SIGTERM → SIGKILL after 2 s)
src/calliope-stems/              bin crate `calliope-stems`, NO Tauri/GTK
  Cargo.toml                     deps: calliope-common, tiny_http, serde, serde_json, uuid;
                                 dev-deps: calliope-common (feature "client"), tempfile
  src/main.rs, src/cli.rs        hand-written flag parser (project convention), --help/--version
  src/config.rs, src/server.rs, src/jobs.rs, src/separator.rs, src/workdir.rs
  tests/conformance.rs           protocol conformance suite (real binary + stub separator)
  separators/audio-separator.sh  adapter for the real model (never run by tests)
  deploy/calliope-stems.service  example systemd user unit (owner installs it by hand)
```

- **The GUI never depends on server code** and the server never depends on Tauri/GTK: both
  depend only on `calliope-common` (protocol types, client behind a feature, process runner).
  A static test (`tests/frontend.rs`) checks with `cargo metadata` that `calliope-stems` and
  `calliope-common` have no `tauri*`/`gtk*`/`webkit*` in their dependency graph and that the
  root package doesn't depend on `calliope-stems`.
- Why a workspace member, not a second `[[bin]]` in the root package: a second bin would share
  the root package's dependencies (Tauri, GTK) and `build.rs` (frontend checks), so the server
  couldn't be built on a headless box without them. A separate crate also makes it easy to
  move the server into its own spec/repository later (the owner's remark).
- Suites now run with `--workspace`: `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`.

### 2.3 Flow and states (one import job at a time)

```
Import view ──"Stem Extraction"──▶ select source page
  (o) URL ............ [enter url ______________] [Extract]
                       <accent text: "Entered URL is invalid" / "Error 403 when attempting download">
                       [=====-----] Download in progress 45 % (12.1 of 27.0 MB)   [Cancel]
  ( ) Local Audio File [Browse]
  ( ) Local Video File [Browse]

job phases (Rust state machine, src/import_job.rs):
  url:   prepare ─(partial found?)─▶ prompt "Incomplete download file from the same URL found"
                                      [Resume Download] [Start Over]
         downloading(bytes,total) ─▶ preparing(audio→FLAC, probe, ≤ 15 min) ─▶ ready(metadata)
  file:  (dialog) ─▶ preparing ─▶ ready(metadata)       errors: "Selected file <file> has no audio track"
  ready ─▶ edit pane (edit mode, fields prefilled)  [Extract] [Cancel]
  Extract ─▶ uploading(sent,total) ─▶ queued ─▶ working ("Working..." spinner, % if known)
          ─▶ receiving(stems n/m) ─▶ saving ─▶ saved(track)  → temp folder removed
  any running phase: [Cancel]; failure: phase failed{stage, message, http_status?}
```

- **One job at a time.** The GPU processes one song at a time anyway; a queue adds UI and
  recovery complexity for little gain. Starting a second job returns "An import is already
  running". The user may switch views; the job keeps running and the Import view shows its
  state again when re-opened (module-level store, plus `get_import_job` / `watch_import`
  after a webview reload).
- **Cancel**:
  - during download: yt-dlp is stopped, its `.part` file is **kept** (resume later); back to
    the source page with "Download stopped. You can resume it later."
  - during preparing: ffmpeg is stopped, its partial output removed; back to the source page.
  - during uploading/queued/working/receiving: the server job is cancelled (`DELETE`), partial
    stems are removed, and the user is back in the edit pane with the metadata kept, so
    Extract can be retried.
  - in the edit pane ("Cancel"): confirmation "Discard this import? The downloaded or prepared
    audio will be removed." Yes removes the job's temp folder (Calliope's own).
- **Failures** keep the temp audio: a failed extraction returns to the edit pane with the error
  shown and Extract enabled (retry). A failed download shows the error under the URL box.
- **App close during a job**: on `RunEvent::Exit` Calliope cancels the job (kills the child
  process group, best-effort `DELETE` to the server, 2 s cap) and exits. Temp files stay: a URL
  download can be resumed through the prompt; other leftovers are cleaned at the next import
  start (§2.5).
- While a job runs, changing the repository root is refused ("An import is running").
- **Length limit (owner decision Q4): 15 minutes**, checked by Calliope after probing (before
  any upload) — "Tracks longer than 15 minutes are not supported" — and again by the server.

### 2.4 `track.json` schema version 2 and migration

```json
{
  "schema_version": 2,
  "id": "0199c1a2-...",
  "type": "stem",
  "band": "The Example Band", "album": "Test Pressings", "title": "Glass Harbour",
  "composers": [], "year": 2021, "source_url": "https://media.example/watch?v=abc", "copyright": null,
  "audio": null,
  "original": "original.flac",
  "stems": [
    {"name": "vocals", "file": "stems/vocals.flac"},
    {"name": "drums",  "file": "stems/drums.flac"},
    {"name": "bass",   "file": "stems/bass.flac"},
    {"name": "guitar", "file": "stems/guitar.flac"},
    {"name": "piano",  "file": "stems/piano.flac"},
    {"name": "other",  "file": "stems/other.flac"}
  ],
  "stem_model": "htdemucs_6s",
  "tablatures": [],
  "imported": "2026-10-06T18:00:00Z", "modified": "2026-10-06T18:00:00Z"
}
```

| Field | Type | Rules |
|---|---|---|
| `type` | `"backing"` or `"stem"` | required in v2. Any other value: the track is a problem ("unknown track type"), never rewritten |
| `audio` | string or null | `backing`: required, plain file name (as v1). `stem`: may be null/absent |
| `original` | string or null | optional; plain file name of the original full mix (FLAC) kept by the import when the "Keep the original mix" setting is on (Q3) |
| `stems` | array of `{name, file}` | default `[]`. `stem` tracks need ≥ 1. `name`: `^[a-z0-9][a-z0-9_-]{0,31}$`, unique. `file`: exactly `stems/<plain file name>` (v1 file-name rules for the part after `stems/`), unique case-insensitively, at most 16 |
| `stem_model` | string or null | ≤ 100 chars, informational |

- All file names in a track (audio, original, stem files, tablatures) are unique
  case-insensitively.
- **Migration v1 → v2 is in memory only** (`track_meta::migrate`): adds `"type": "backing"`,
  `"stems": []`, sets `schema_version` 2. A v1 file is never rewritten by a scan, list, export or
  import; its revision stays the hash of its bytes.
- **Lazy write**: a Library Save writes schema 2. Older builds then show that one track as
  "written by a newer Calliope" and never touch it (existing behaviour). The repository marker
  stays at `schema_version: 1`: the layout additions (`import-tmp/`, dot-named staging folders)
  are invisible to older builds, which scan only `tracks/` and skip dot names.
- A v1 file that already contains a `type` key: `"backing"` is accepted; anything else makes the
  track a problem, never rewritten.
- `missing` also lists stem files and `original` when not on disk. Library Save of a stem track
  keeps `audio`, `original`, `stems` verbatim. Export copies `original` and `stems/` too.
  Delete is unchanged (the whole folder goes to the trash).
- `TrackRecord` (IPC) gains `type`, `stems`, `stem_model`, `original`; `audio` becomes
  `string | null`.

### 2.5 Temporary space and atomic track creation (data safety)

```
<root>/
  import-tmp/                     Calliope's own working space (answers the spec's open question)
    url-<16 hex>/                 one per URL; <16 hex> = FNV-1a 64 of the normalised URL
      job.json                    {"kind":"url","url":"<normalised url>","created":"<rfc3339>"}
      download.<ext>.part         yt-dlp's partial download (resumable)
      download.<ext>              the complete download
      info.json                   yt-dlp's metadata file
      audio.flac                  normalised audio (FLAC, 44.1 kHz, stereo) sent to the server
    file-<uuidv7>/                one per local-file import
      job.json                    {"kind":"audio-file"|"video-file","name":"<bare file name>","created":...}
      audio.flac
  tracks/
    .staging-<new id>/            the new track being assembled (scan skips dot names)
      .calliope-staging           marker: "this folder is Calliope's own"
      stems/<name>.flac.part → stems/<name>.flac
      original.flac               only with "Keep the original mix" (renamed from audio.flac)
      track.json
    <new id>/                     ← one rename of the whole staging folder at the end
```

- **User source files are only ever read**: passed to `ffprobe`/`ffmpeg` as `file:<absolute
  path>` inputs; nothing is written next to them.
- **Atomic track creation**: stems are downloaded into `tracks/.staging-<id>/stems/` (each as
  `.part`, checked, renamed); with "keep original" on, the job's `audio.flac` is renamed (same
  file system) into staging as `original.flac`; `track.json` is written with `write_atomic`;
  the marker is removed; then `tracks/.staging-<id>` is renamed to `tracks/<id>` under the
  `RepoState` mutex, after checking `tracks/<id>` doesn't exist (never replaces).
- **Temp removal**: after the track is saved, the job's `import-tmp/<job>/` folder is removed
  (spec). Also removed: a discarded import's folder, and the staging folder of a failed or
  cancelled extraction (an `original.flac` already moved there is moved back first, so a retry
  still has its audio).
- **What Calliope may delete** (and nothing else): folders directly in `<root>/import-tmp/`
  whose name matches `^(url-[0-9a-f]{16}|file-[0-9a-z-]{36})$` and that contain `job.json`, and
  `tracks/.staging-<id>` folders containing `.calliope-staging`. Each is checked with
  `symlink_metadata` to be a real folder; `std::fs::remove_dir_all` does not follow symlinks
  inside. A symlinked `import-tmp` is refused with an error.
- **Stale leftovers**: at each new import start, `file-*` and `.staging-*` folders not
  belonging to the running job are removed; `url-*` folders are kept for resuming unless older
  than 14 days (`job.json` `created`).
- **Resume prompt**: `prepare_url_import(url)` looks up `import-tmp/url-<key>/` and requires its
  `job.json` URL to equal the normalised URL. If it holds a `.part`, a complete `download.*` or
  `audio.flac`, the UI shows the prompt. **Resume Download** runs yt-dlp with `--continue` (or
  skips to preparing/ready if the download or `audio.flac` is complete). **Start Over** clears the
  folder's contents and runs yt-dlp with `--no-continue`.
- A configured root must exist with status `ok`/`empty` (the default root is created as in
  `list_tracks`); otherwise "The track repository folder is missing. Check Settings > Track
  repository."

### 2.6 External tools on the laptop

| Tool | Used for | Found by | Version check | Licence | Bundled? |
|---|---|---|---|---|---|
| `yt-dlp` | URL download (audio only) + metadata (`info.json`) | `PATH` | `--version` (date), ≥ 2023.01 | Unlicense | **No**; the owner installs it (`pacman -S yt-dlp`) |
| `ffmpeg` | decode any source / extract the first audio stream → FLAC 44.1 kHz stereo | `PATH` | `-version`, major ≥ 5 | LGPL-2.1+/GPL (Arch build GPL-3.0) | **No** (`pacman -S ffmpeg`) |
| `ffprobe` | audio-stream check, duration, tags | next to `ffmpeg`, then `PATH` | same | same | **No** |

- They run as separate processes, never linked, so their licences put no obligations on
  Calliope while they are not bundled (recorded in `docs/licences.md`).
- `src/tools.rs` (GUI): `find_in_path(name, path_var)` (pure; `gui.rs` passes `PATH` once),
  version parsing, and runs them with `calliope_common::process` (argv arrays, never a shell;
  own process group + `PR_SET_PDEATHSIG`; cancel = SIGTERM to the group, SIGKILL after 2 s.
  PDEATHSIG fires when the spawning *thread* exits; the job thread waits for its child, so it
  holds).
- Missing tool → "yt-dlp was not found. Install it (Arch: sudo pacman -S yt-dlp) and try
  again."; too old → "yt-dlp 2022.01.01 is too old (need 2023.01 or newer)." Settings shows
  the versions in an "External tools" card.
- **yt-dlp invocation** (`src/download.rs`), every value a separate argv entry:
  ```
  yt-dlp --ignore-config --no-playlist --no-colors --newline --no-mtime
         --format bestaudio/best --continue|--no-continue
         --write-info-json --no-write-thumbnail --no-write-comments
         --progress-template "download:calliope-progress %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s"
         --paths <job dir> --paths temp:<job dir>
         --output download.%(ext)s --output infojson:info
         -- <validated url>
  ```
  `--ignore-config` keeps the user's yt-dlp config (`--exec`, other paths…) out; `--` stops
  option parsing before the URL. Progress lines → `(downloaded, total|None)`. Errors: stderr
  `ERROR:` lines; `HTTP Error (\d{3})` → **"Error <code> when attempting download"**; otherwise
  "Download failed: <yt-dlp's message, max 300 chars>". The implementer checks these flags
  against the current yt-dlp README (it isn't installed here; the fake mimics it).
- **ffprobe**: `ffprobe -v error -print_format json -show_format -show_streams file:<abs>`.
  No audio stream → "Selected file <file> has no audio track"; unreadable → "Selected file
  <file> could not be read as audio or video".
- **ffmpeg**: `ffmpeg -nostdin -hide_banner -loglevel error -n -i file:<abs> -map 0:a:0 -vn -sn
  -dn -c:a flac -ar 44100 -ac 2 -f flac <job dir>/.audio.flac.part`, then rename to `audio.flac`.
- **Metadata mapping** (`src/media.rs` / `src/download.rs`, pure, unit-tested) into `TrackEdits`:

  | Field | file tags (ffprobe `format.tags`, case-insensitive; ogg/flac also stream tags) | yt-dlp `info.json` |
  |---|---|---|
  | band | `artist`, else `album_artist` | `artist`, else `creator`; else X of a `"X - Y"` title |
  | album | `album` | `album` |
  | title | `title`, else the file name without extension | `track`, else `title` (or Y of `"X - Y"`) |
  | composers | `composer`, split on `;` and `/` | `composer`/`composers` |
  | year | first 4 digits of `date`/`year` (1..9999) | `release_year`, else `release_date`, else `upload_date` |
  | source_url | none | `webpage_url`, else the entered URL |
  | copyright | `copyright` | `license` |

  Values are trimmed, control characters removed, cut to the field limits; an empty title
  falls back to the file name / URL. For URLs, info values win and file tags fill gaps.
- Limits: 15 minutes (Q4), 1 GiB per audio file.

### 2.7 "calliope-stems API v1" (protocol; owner: "looks good as a starter")

Plain HTTP/1.1, JSON bodies UTF-8, base URL from Calliope's settings (e.g.
`http://archserver:8765`). Types and validation live in `calliope_common::stems_api`, shared by
client and server.

| Request | Response |
|---|---|
| `GET /v1/health` | `200 {"service":"calliope-stems","api":1,"version":"<server version>","models":["htdemucs_6s"],"default_model":"htdemucs_6s","busy":false,"max_duration_s":900,"max_upload_bytes":314572800}` |
| `POST /v1/jobs?model=htdemucs_6s`, `Content-Type: audio/flac`, `Content-Length: n`, body = FLAC | `202 {"job":"<id>","state":"queued"}`; `400` bad model; `411` no length; `413` too large or longer than the limit; `415` not FLAC / unreadable STREAMINFO; `503` queue full |
| `GET /v1/jobs/<id>` | `200 {"job","state":"queued"\|"running"\|"done"\|"failed"\|"cancelled","progress":0.42\|null,"stems":["vocals",...]\|null,"error":null\|"text"}`; `404` unknown (e.g. after a server restart) |
| `GET /v1/jobs/<id>/stems/<name>` | `200`, `Content-Type: audio/flac`, `Content-Length`; `404`; `409` not done |
| `DELETE /v1/jobs/<id>` | `204`; cancels a queued/running job (kills the separator) and deletes its files |

- Errors are `{"error":"text"}` with 4xx/5xx. Unknown paths `404`, wrong methods `405`.
- **Client rules** (`calliope_common::stems_client`): health must say `service ==
  "calliope-stems"` and `api == 1` (else "incompatible"); model = `default_model`; connect 5 s,
  request 30 s (upload: 30 s without progress); poll every 1 s; no state change for 60 min →
  "The edge-AI server stopped responding"; `404` while polling → "The edge-AI server restarted;
  try again"; upload streamed with `Content-Length` through a counting reader (progress,
  cancel); ids/stem names validated, ≤ 16 stems, JSON ≤ 64 KiB, each stem ≤ 1 GiB and must start
  with `fLaC`; `DELETE` after all stems are saved (best effort).
- **"Working..." is shown when the server reports `running`** (spec: "edge-ai confirms
  processing start"); `queued` shows "Waiting for the edge-AI server...".
- Footer: `Edge-AI: not configured | connected | unreachable | incompatible`, from a health
  check at start-up (only when configured, 2 s timeout), after Settings changes / "Test
  connection", and from job results.

### 2.8 The `calliope-stems` server

- **Stack**: Rust, `tiny_http` (small synchronous HTTP server, one thread per request is fine
  for one client), `serde_json`, `uuid` (v4 job ids), `calliope-common`. No async runtime.
- **Command line** (hand-written parser, like the GUI; `--help`, `--version`):

  | Flag | Default | Meaning |
  |---|---|---|
  | `--listen ADDR:PORT` | `0.0.0.0:8765` | address to bind; port `0` = any free port; prints `calliope-stems listening addr=<addr:port>` on stderr once bound |
  | `--work-dir DIR` | `$XDG_STATE_HOME/calliope-stems` (`~/.local/state/calliope-stems`) | job files |
  | `--separator PATH` | **required, no default** | the separator executable (§ contract below). Required so that nothing (a test, a typo) can start the real model by accident |
  | `--model NAME` | `htdemucs_6s` | the model name advertised and passed to the separator |
  | `--max-upload-mb N` | `300` | larger uploads → 413 (15 min of 44.1 kHz stereo FLAC is ~100-160 MB) |
  | `--max-duration-s N` | `900` | longer audio (from STREAMINFO) → 413 |
  | `--queue N` | `2` | jobs waiting behind the running one; more → 503 |
  | `--separator-timeout-min N` | `30` | a separator running longer is killed → failed |
  | `--retention-hours N` | `24` | finished/failed/cancelled jobs are deleted after this |

  Max concurrent jobs is **1** (fixed: one GPU).
- **Auth: none** (stated default, owner-approved). The LAN has no firewall by the owner's
  choice; the exposure is bounded (FLAC only, size/duration caps, queue of 2, no file paths from
  clients, nothing executed from client input). A shared token can be added in API v2 if the
  server is ever reachable from outside the home LAN.
- **Separator contract**: `<separator> <input.flac> <out_dir> <model>`, run with
  `calliope_common::process` (argv array, own process group, PDEATHSIG), cwd = the job folder,
  stdin null. It must write `<out_dir>/<stem>.flac` files and exit 0. Optional stdout lines
  `progress <0..1>` update the job's progress; every other line is logged. After exit 0 the
  server checks the outputs: names follow the stem rule, ≤ 16 files, each starts with `fLaC`,
  nothing else in the folder; otherwise the job fails ("separator produced invalid output").
  Stems are listed vocals, drums, bass, guitar, piano, other first, then others alphabetically.
- **Adapter for the real model** `src/calliope-stems/separators/audio-separator.sh`: does the
  separation part of the owner's `~/edge-ai/stems/backing` (frees VRAM by asking Ollama to
  unload models when less than 2.5 GB is free, then runs
  `$STEMS_HOME/.venv/bin/audio-separator <in> -m <model>.yaml --model_file_dir $STEMS_HOME/models
  --output_dir <out> --output_format FLAC --custom_output_names {...six names...}`), without the
  mixing. `STEMS_HOME` defaults to `~/edge-ai/stems`. No progress lines (the UI shows the
  spinner without a percentage). Tests only syntax-check it (`bash -n`); it is never executed by
  any test or agent.
- **Job lifecycle** (in memory; `src/jobs.rs`): POST validates headers, streams the body to
  `<work>/jobs/<id>/input.flac.part` with the size cap, checks the FLAC magic and STREAMINFO
  (duration known and ≤ limit), renames to `input.flac`, enqueues (or 503 and deletes). One
  worker thread takes jobs in order: `running` → separator → `done`/`failed`. `DELETE` on a
  queued job removes it; on a running job kills the separator group, then removes the files.
  A janitor thread (every 10 min) deletes jobs older than the retention. Job folders contain a
  marker `.calliope-stems-job`; at start-up the server deletes every `jobs/<id>/` folder that
  has the marker (state is not persisted: clients get `404` and say "restarted") and never
  touches anything else in the work dir. On SIGTERM (systemd stop) it kills a running separator
  and exits.
- **Logs** (stderr → journald under systemd), prefix `calliope-stems: `: `listening addr=…
  model=… separator=…`, `request method=… path=… status=… bytes=… ms=… peer=…`, `job id=…
  state=… [duration_s=…] [error=…]`, `separator id=… line=…` (separator output, truncated to
  300 chars), `cleanup removed=<n>`.
- **Deployment is manual** (§5): the owner builds it on archserver, copies the binary and the
  adapter, installs the example user unit. Agents never start it with the real separator and
  never install anything.

### 2.9 Rust modules (GUI crate)

| Module | Responsibility | Tauri? |
|---|---|---|
| `src/track_meta.rs` (extend) | schema v2, `TrackType`, `StemEntry`, `original`, migrate v1→v2, rules §2.4 | no |
| `src/repository.rs` (extend) | v2 records; stems/original in `missing`/export/save rules; `begin_staged_track(id) -> Staging`, `Staging::stem_part_path`, `Staging::adopt_original(src)`, `Staging::commit(meta)`, `Staging::abandon()`; cleanup of stale `.staging-*` | no |
| `src/import_tmp.rs` (new) | `import-tmp/` layout §2.5 | no |
| `src/tools.rs` (new) | find executables, versions; uses `calliope_common::process` | no |
| `src/media.rs` (new) | ffprobe → `Probe`; tags → `TrackEdits`; ffmpeg FLAC conversion; 15-min check | no |
| `src/download.rs` (new) | URL validation/normalisation/key (`url`), yt-dlp args, progress/error parsing, `info.json` mapping | no |
| `src/import_job.rs` (new) | job state machine §2.3, `ImportState`, events to a `Fn(ImportEvent)` sink, cancel, shutdown, snapshot | no |
| `src/settings.rs` (extend) | `edge_ai_url: Option<String>` (`http://host[:port][/path]`, no userinfo/query/fragment, ≤ 200 chars), `keep_original: bool` (default false) | no |
| `src/picker.rs` (extend) | `DialogKind::ImportAudio` ("import-audio", "Audio files": mp3 flac ogg), `ImportVideo` ("import-video", "Video files": mp4 m4v mkv webm mov avi); extension re-check | `TauriPicker` only |
| `src/ipc.rs` (extend) | commands §2.10; `Channel<ImportEvent>` sink adapter | yes |
| `src/gui.rs` (extend) | manage `ImportState` + tool paths (PATH captured once), start-up health check, `RunEvent::Exit` → `ImportState::shutdown(2 s)` | yes |

The edge-AI client itself is `calliope_common::stems_client` (feature `client`), used by
`import_job.rs` and the health check. Pure modules take tool paths, roots, server URLs and
`PATH` as parameters (no env lookups inside).

### 2.10 IPC commands and events (new)

| Command | Args | Returns |
|---|---|---|
| `set_edge_ai_url` | `url: string \| null` | `Settings` (error text if invalid) |
| `set_keep_original` | `keep: boolean` | `Settings` |
| `check_edge_ai` | none | `EdgeAiStatus {state: "not-configured"\|"connected"\|"unreachable"\|"incompatible", message, models}` |
| `check_tools` | none | `ToolsStatus {yt_dlp, ffmpeg, ffprobe: {found, version \| null, ok, message}}` (no paths) |
| `prepare_url_import` | `url` | `UrlPrep {status: "invalid"\|"ready"\|"partial"\|"busy", message, partial_bytes}` |
| `start_url_import` | `url, resume: bool, events: Channel` | `JobSnapshot` |
| `import_file` | `kind: "audio"\|"video", events: Channel` | `JobSnapshot \| null` (null = dialog cancelled); opens the dialog itself |
| `start_stem_extraction` | `job, edits: TrackEdits` | `JobSnapshot` (edits validated strictly; `keep_original` read from settings now) |
| `cancel_import` | `job` | `JobSnapshot` |
| `discard_import` | `job` | `null` |
| `get_import_job` | none | `JobSnapshot \| null` |
| `watch_import` | `events: Channel` | `JobSnapshot \| null` (re-attach after a reload) |

`get_settings` gains `edge_ai_url` and `keep_original`. `set_repository_root` /
`reset_repository_root` fail with "An import is running" during a job.

`JobSnapshot` = `{job, source: {kind: "url"|"audio-file"|"video-file", label}, phase,
downloaded, total, sent, stems_done, stems_total, progress, duration_s, metadata:
TrackEdits|null, error: {stage: "download"|"prepare"|"server"|"save", message, http_status:
number|null}|null, track: TrackRecord|null}`. `label` is the URL or the bare file name.

`ImportEvent` (`#[serde(tag = "phase")]`, lowercase): `downloading {downloaded, total}`,
`preparing`, `ready {metadata, duration_s}`, `uploading {sent, total}`, `queued`, `working
{progress}`, `receiving {done, total}`, `saving`, `saved {track}`, `failed {stage, message,
http_status}`, `cancelled {back_to: "source"|"edit"}`. Throttled to ≤ 10 per second.

- **Why a `tauri::ipc::Channel`, not `emit`/`listen`**: `listen` needs `core:event`
  permissions, which the capability doesn't grant. Channels are command arguments and need none
  (Tauri 2.12.1 exempts the channel-data fetch from the ACL; small messages arrive by callback).
- New `calliope-ui:` lines: `import mode=stem-extraction`, `import source=url|audio|video`,
  `import phase=<phase> job=<job>`, `import error stage=<s> message=<m>`, `import saved id=<id>
  stems=<n> original=<bool>`. Rust: `calliope: dialog kind=import-audio|import-video result=…`,
  `calliope: import job=<job> phase=<phase>`, `calliope: tool <name> found=<bool> version=<v>`.

### 2.11 Frontend

- `src/ui/lib/url-check.ts`: `checkUrl(text)` mirroring Rust (http/https, host, no spaces, no
  userinfo, ≤ 2000 chars, via `new URL`). Rust re-validates.
- `src/ui/lib/import-state.svelte.ts`: module-level store (`mode`, `source`, `url`, `urlError`,
  `prompt`, `job`, `draft`, errors); actions call `ipc.ts`; events update the snapshot.
- `src/ui/views/ImportView.svelte` replaces the placeholder. `menu`: heading + a large
  "Stem Extraction" button (later "Import Backing Track" goes next to it). `stem`: "Back" + steps:
  - `SourcePage.svelte`: radios URL / Local Audio File / Local Video File; URL → input "enter
    url" + Extract (Enter works); error label **under the box in accent colour** (`text-primary`,
    amber; `role="alert"`); progress bar "Download in progress" (percent + MB; indeterminate when
    the total is unknown) + Cancel; the resume prompt as an inline panel with the exact text and
    "Resume Download" / "Start Over". Audio/Video → "Browse"; errors in the same label; "Preparing
    audio..." while ffmpeg runs.
  - `ImportEditPane.svelte`: `TrackFields.svelte` in edit mode, prefilled; read-only "Source"
    and "Length" lines; a note "The original mix will be kept" / "will not be kept" with a link
    button to Settings; **Extract** and **Cancel** (Escape = Cancel with the confirmation).
  - `ExtractProgress.svelte`: "Sending to the edge-AI server" bar, "Waiting for the edge-AI
    server...", spinner **"Working..."** (Lucide `loader-circle` + `animate-spin`; percent when
    known), "Receiving stems 3 of 6", Cancel.
  - Done: "Saved <title> with 6 stems." + "Show in Library" (switches view, selects the track) +
    "Import another track".
- `TrackFields.svelte` (extracted from `TrackPane.svelte`, `idPrefix` prop): the field grid.
- Library: badge before each track name: **"S"** (stem, amber outline) / **"B"** (backing,
  neutral), our own design, `aria-label` "Stem track"/"Backing track". TrackPane read-only rows:
  "Type", "Audio file" ("none" for stem tracks), "Original" (when present), "Stems".
- Settings: the Edge-AI placeholder becomes a real **"Stem extraction"** card: "Edge-AI server
  address" (placeholder `http://archserver:8765`), Save, "Test connection", status line, and the
  switch/checkbox **"Keep the original mix with the stems"** (default off; Q3). A new "External
  tools" card lists yt-dlp/ffmpeg/ffprobe versions or install hints. `SETTINGS_PLACEHOLDERS`
  loses the edge-ai entry.
- Footer: `Ready · Edge-AI: <status>`; during a job the left text shows the phase.

### 2.12 Security

- The frontend never sends paths: URLs are validated strings; local files come from Rust
  dialogs (`import_file` opens the dialog and keeps the path in Rust); snapshots carry bare
  names only.
- URL validation in Rust (`url` crate): `http`/`https` only, host required, no userinfo,
  ≤ 2000 chars, no whitespace/control characters; the normalised URL goes to yt-dlp after `--`.
- External tools: argv arrays, `--ignore-config`, `file:` inputs, output only into the job folder.
- Client ↔ server: ids/names validated before use in URLs or file names, size caps, FLAC magic.
- Server: never takes a path or command from a client; job ids are server-generated UUIDs;
  stem names are validated before use as file names; uploads capped before writing; the work
  dir is the only place it writes, and it deletes only marker folders.
- ACL: the 12 new commands go into `generate_handler!`, `COMMANDS` and
  `capabilities/main.json`; still no core/plugin permissions. `ScriptedPicker` learns the two
  new kinds (`e2e-hooks` only).

### 2.13 Dependencies added

| Crate | Where | Licence | Why |
|---|---|---|---|
| `ureq` 3 (`default-features = false`) | calliope-common (feature `client`) | MIT OR Apache-2.0 | small blocking HTTP client; no TLS on the LAN |
| `url` 2 (already in the tree) | calliope-gui | MIT OR Apache-2.0 | URL validation |
| `libc` 0.2 (already in the tree) | calliope-common | MIT OR Apache-2.0 | `prctl(PR_SET_PDEATHSIG)`, `kill(-pgid)` |
| `tiny_http` 0.12 | calliope-stems | MIT OR Apache-2.0 | HTTP server |
| `serde`, `serde_json`, `uuid`, `tempfile` | calliope-common / calliope-stems | as recorded | already used by the GUI |

Frontend: the shadcn-svelte `progress` component (MIT, copied) and a `switch` or `checkbox`
component (MIT, copied). No new npm packages. Test-only: `python3` runs the fake yt-dlp.
External, not bundled: yt-dlp, ffmpeg/ffprobe (laptop); audio-separator, Demucs/htdemucs
model, PyTorch (archserver, the owner's existing install).

## 3. Tasks

All commands run from `/home/vali/src/calliope`. Headless suite: `npm test`. GUI tests:
`DISPLAY=:1 npm run test:gui`. **No task may read or write `~/.local/share/calliope`,
`~/.config/app.calliope.gui`, `~/edge-ai`, any real user folder, the internet, or any LAN host.
No task may run `audio-separator`, the adapter script, the real model, `systemctl`, or install
anything.** `calliope-stems` is only ever started with `--listen 127.0.0.1:0` (or `127.0.0.1:8765`
inside the e2e network namespace) and `--separator tests/support/stub-separator`. yt-dlp is
always `tests/support/bin/yt-dlp`; fake URLs use the reserved `.example` domain; the real local
`ffmpeg`/`ffprobe` run on committed fixtures. **Nothing is routine**: every task is `routine: no`.

### Task 1: Import test fixtures
- **files**: `tests/fixtures/import/make-fixtures.sh` (new), `tests/fixtures/import/*` (generated, committed), `tests/fixtures/import/README.md`
- **does**: A bash script using `ffmpeg` lavfi sources (sine, `testsrc`), invented content
  only, generating: `tagged.mp3` (5 s; title "Glass Harbour", artist "The Example Band", album
  "Test Pressings", date "2021-03-04", composer "Ann Example; Bo Sample", copyright "(c) 2021 The
  Example Band"), `untagged.flac` (5 s), `tagged.ogg` (Vorbis, stream tags), `with-audio.mp4`
  (2 s testsrc + sine, title "Clip Title"), `no-audio.mp4`, `not-audio.mp3` (a text file),
  `download.webm` (Opus 5 s, for the fake yt-dlp), `info.json` (canned: title "The Example Band -
  Night Drive", webpage_url `https://media.example/watch?v=abc123`, upload_date "20200115"),
  `stems/{vocals,drums,bass,guitar,piano,other}.flac` (1 s, mono 8 kHz, different tones),
  `long.flac` (16 min of silence, mono 8 kHz, to test the 15-minute limit; keep it small) and
  `not-flac.flac` (an Ogg file renamed). Total < 600 KB.
- **done when**: the script is idempotent; `ffprobe` shows the tags; `no-audio.mp4` has no audio
  stream; `long.flac` is > 900 s; `du -sk tests/fixtures/import` < 600.
- **test**: `bash tests/fixtures/import/make-fixtures.sh && ffprobe -v error -show_entries format_tags -of json tests/fixtures/import/tagged.mp3`
- **routine: no**

### Task 2: Cargo workspace and the `calliope-common` crate
- **files**: `Cargo.toml` (workspace, default-members), `src/calliope-common/Cargo.toml`, `src/calliope-common/src/{lib.rs,stems_api.rs,process.rs}`, `package.json` (`test`: `cargo test --workspace`, clippy `--workspace`), `tests/frontend.rs` (dependency-separation static test)
- **does**: §2.2. `stems_api`: types of §2.7 (serde, `#[serde(rename_all = "lowercase")]` states),
  `API_VERSION`, `MAX_DURATION_S = 900`, `MAX_STEMS = 16`, `is_valid_job_id`, `is_valid_stem_name`,
  `flac_info(reader) -> Result<FlacInfo{sample_rate, channels, total_samples, duration_s}>`
  (magic `fLaC`, first metadata block = STREAMINFO, 34 bytes, total samples 0 = unknown → error).
  `process`: `spawn(program, args, cwd, on_stdout_line, on_stderr_line) -> Running` with
  `cancel()`/`wait()`, process group + PDEATHSIG on Linux, `Child::kill` elsewhere. Unit tests:
  STREAMINFO of the task-1 fixtures (`untagged.flac` 5 s, `long.flac` > 900 s, `not-flac.flac`
  error), name/id tables, serde round trips, process: lines delivered, cancel kills a script and
  its grandchild `sleep 60` (checked with `kill -0`), missing program → clear error. Static
  test: `cargo metadata` shows no `tauri`/`gtk`/`webkit` package reachable from
  `calliope-common` or `calliope-stems`, and `calliope-gui` doesn't depend on `calliope-stems`.
- **done when**: `cargo test --workspace` passes; `cargo build` (root) builds only the GUI;
  `npm run build:app` unchanged; clippy `--workspace` clean.
- **test**: `npm test`
- **routine: no** (build layout, signals)

### Task 3: `track_meta` schema v2 and migration
- **files**: `src/track_meta.rs`
- **does**: `CURRENT_SCHEMA = 2`; `TrackType {Backing, Stem}` (field `type` via `#[serde(rename =
  "type")] track_type`); `StemEntry {name, file}`; `audio: Option<String>`; `original:
  Option<String>`; `stems`; `stem_model`. `migrate(value, 1)` per §2.4 (in memory). `parse`
  accepts 1 and 2. `validate_for_read`/`validate_for_write` per §2.4 (writes 2 always). Tests:
  v1 sample → backing v2 with identical other fields; v1 with `"type":"stem"` → error; stem
  without stems → error; `stems/../x.flac`, `stems/a/b.flac`, `x.flac`, duplicate names, original
  equal to a tablature → errors; unknown fields round-trip; a migrated v1 serialises with
  `"schema_version": 2` and `"type": "backing"`.
- **done when**: `cargo test track_meta` passes; clippy clean.
- **test**: `cargo test --workspace track_meta && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no** (data format)

### Task 4: Repository v2: records, stems, original, save/export, never-rewrite guarantee
- **files**: `src/repository.rs`, `src/repository_tests.rs`, `tests/acceptance_tracks_repository.rs` (only expectations that change from schema 1 to 2), `tests/fixtures/library-v2/` (new)
- **does**: `TrackRecord` gains `track_type` (`type`), `stems`, `stem_model`, `original`; `audio:
  Option<String>`. `missing`, save, export per §2.4. Fixture `library-v2/` (marker 1): two v1
  backing tracks, one v2 backing track, one v2 stem track
  `0199c0a0-0000-7000-8000-000000000101` with the six fixture stems and an `original.flac`.
  Tests (each also checks an "outside" folder is unchanged): a scan leaves **every byte** of
  every `track.json` unchanged; v1 revision = FNV of its bytes; saving a v1 track writes schema
  2 + `type: backing`, keeps unknown fields; the stem track lists 6 stems and the original;
  `missing` reports a deleted stem; export copies `stems/` and `original.flac`; delete moves the
  whole folder to the trash; existing library-sample tests pass (only "schema 1 after save"
  assertions change, named in the commit message).
- **done when**: `cargo test --workspace` passes; clippy clean.
- **test**: `npm test`
- **routine: no** (data safety)

### Task 5: Import temp space and staged track creation
- **files**: `src/import_tmp.rs` (new), `src/repository.rs`, `src/repository_tests.rs`, `src/main.rs`
- **does**: §2.5 exactly (`ImportTmp::open`, URL key + `job.json` check, `find_partial`,
  `new_file_dir`, `clear_for_start_over`, `remove_job_dir`, `clean_stale(now, keep)`;
  `begin_staged_track`, `adopt_original` (rename, and move back on abandon), `commit`,
  `abandon`). Tests: commit gives a scan-visible stem track (with and without original); a scan
  during staging doesn't list it; commit refuses when `tracks/<id>` exists; abandon/remove refuse
  symlinks, folders without marker/`job.json`, names outside the patterns; abandon after
  `adopt_original` puts `audio.flac` back; `clean_stale` keeps the running job and fresh url-*
  folders; a symlinked `import-tmp` is refused; a user file placed in `import-tmp/` by hand
  survives every cleanup.
- **done when**: tests pass; clippy clean.
- **test**: `npm test`
- **routine: no** (deletion code)

### Task 6: `tools` module: discovery and versions
- **files**: `src/tools.rs` (new), `src/main.rs`
- **does**: §2.6 `find_in_path(name, path_var)`, ffprobe next to ffmpeg first, version parsing
  (yt-dlp `YYYY.MM.DD[.n]`; ffmpeg `n9.0.2`, `7.1`, `N-…` git builds count as new), a `Tools`
  struct with `check() -> ToolsStatus`. Tests with scripts in a temp dir standing in for the
  tools (no real tool needed), a version table, and a PATH with a non-executable file.
- **done when**: `cargo test tools` passes; clippy clean.
- **test**: `cargo test --workspace tools && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no**

### Task 7: `media` module: probe, metadata mapping, FLAC conversion, 15-minute limit
- **files**: `src/media.rs` (new), `src/main.rs`
- **does**: `probe`, `edits_from_tags`, `to_flac` (part file, rename, removed on error/cancel,
  via `calliope_common::process`), error texts §2.6, refuse > `MAX_DURATION_S` ("Tracks longer
  than 15 minutes are not supported"). Tests with the real ffmpeg/ffprobe on task-1 fixtures:
  mp3 tags → band/album/title/composers/year as in task 1; `untagged.flac` → title "untagged";
  ogg stream tags; `no-audio.mp4` → "Selected file no-audio.mp4 has no audio track";
  `not-audio.mp3` → "could not be read"; `long.flac` → the 15-minute error; `with-audio.mp4` →
  `audio.flac` with `fLaC`, 44.1 kHz stereo; sources byte-identical afterwards; pure mapping
  tests for odd tags.
- **done when**: `cargo test media` passes; clippy clean.
- **test**: `cargo test --workspace media && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no**

### Task 8: Fake yt-dlp and the `download` module
- **files**: `tests/support/bin/yt-dlp` (new, executable Python 3), `tests/support/README.md` (new), `src/download.rs` (new), `src/main.rs`, `Cargo.toml` (`url = "2"`)
- **does**: The fake: `--version` → `2025.09.26`; refuses (exit 2, "fake yt-dlp: real network
  URLs are not allowed") any host not ending in `.example`; requires `--ignore-config` and `--`;
  honours `--paths`, `--output`, `--continue/--no-continue`; by URL path: `/watch?v=ok` copies
  `download.webm` in 5 chunks with progress lines in the template format and writes `info.json`;
  `/http403` → `ERROR: [generic] Unable to download webpage: HTTP Error 403: Forbidden`, exit 1;
  `/offline` → `ERROR:` without a code; `/slow` → chunks every 300 ms; `/no-total` → `NA`
  totals; resumes from an existing `.part` with `--continue`; appends its argv as a JSON line to
  `$FAKE_YTDLP_LOG`. The module: `validate_url`, `normalise`, `url_key`, `ytdlp_args`,
  `parse_progress`, `parse_error`, `edits_from_info`, `run_download`. Tests: URL table
  (`file:///etc/passwd`, `javascript:alert(1)`, `ftp://x`, `https://`, `http://user:pw@h/`,
  `https://a b.example`, `-https://x`, 2001 chars → invalid; fragment dropped, query kept); full
  download via the fake (progress increases; `download.webm` + `info.json`); 403 → "Error 403
  when attempting download"; cancel `/slow` keeps a `.part`; resume continues from its size (fake
  log) and `--no-continue` starts at 0; info mapping → band "The Example Band", title "Night
  Drive", year 2020, the webpage URL.
- **done when**: `cargo test download` passes; clippy clean.
- **test**: `cargo test --workspace download && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no** (security-relevant parsing)

### Task 9: Stub separator and the real-model adapter
- **files**: `tests/support/stub-separator` (new, executable bash), `src/calliope-stems/separators/audio-separator.sh` (new), `tests/support/README.md`
- **does**: The stub follows the separator contract (§2.8): copies
  `tests/fixtures/import/stems/*.flac` into `<out_dir>`, prints `progress 0.25` … `progress 1`,
  exit 0; behaviour from the env var `STUB_SEPARATOR_MODE` (or, for the e2e tests, a file
  `<work dir>/stub-mode`): `ok`, `fail` (exit 3 with a message), `slow` (2 s per step),
  `hang` (sleeps until killed), `bad-output` (writes `README.txt`), `not-flac` (a stem without the
  magic); it logs its argv to `$STUB_SEPARATOR_LOG`. The adapter per §2.8, with a header
  comment stating the contract, `set -euo pipefail`, quoting for paths with spaces.
- **done when**: `bash -n` passes for both; running the stub by hand into a temp dir produces 6
  FLAC files; a test in task 10 uses it. The adapter is never executed.
- **test**: `bash -n tests/support/stub-separator && bash -n src/calliope-stems/separators/audio-separator.sh`
- **routine: no**

### Task 10: The `calliope-stems` server
- **files**: `src/calliope-stems/Cargo.toml`, `src/calliope-stems/src/{main.rs,cli.rs,config.rs,server.rs,jobs.rs,separator.rs,workdir.rs}`, `Cargo.toml` (workspace member)
- **does**: §2.7-2.8: flags, `--help`/`--version` (version from the crate version), required
  `--separator`, binding + the `listening addr=` line, routes and status codes, streaming upload
  with caps, FLAC/STREAMINFO checks (15-minute limit), the single worker + queue, separator runs
  via `calliope_common::process` with progress parsing and timeout, output validation, `DELETE`
  (queued/running/finished), janitor, start-up cleanup of marker folders only, SIGTERM handling,
  logs. Unit tests: CLI parsing table (missing `--separator` → exit 2 with a message), route
  table, STREAMINFO checks, output validation (bad names, extra files, non-FLAC), start-up
  cleanup keeps unmarked files, retention.
- **done when**: `cargo test -p calliope-stems` passes; `cargo build --release -p calliope-stems`
  works without Tauri/GTK dev headers being involved (checked by task 2's static test); clippy
  clean.
- **test**: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no** (server, concurrency, untrusted input)

### Task 11: Stems client and the protocol conformance suite
- **files**: `src/calliope-common/src/stems_client.rs` (feature `client`), `src/calliope-common/Cargo.toml`, `src/calliope-stems/tests/conformance.rs` (new), `src/calliope-stems/Cargo.toml` (dev-deps)
- **does**: The client (§2.7 rules): `StemsClient::new(base)`, `health`, `submit(file, &cancel,
  on_sent)`, `status`, `fetch_stem(job, name, dest_part)`, `delete`. The conformance suite starts
  the real binary (`CARGO_BIN_EXE_calliope-stems`) with `--listen 127.0.0.1:0`, a temp work dir
  and the stub separator, and checks with raw HTTP (std `TcpStream`) **and** with the client:
  health fields; POST without length 411, non-FLAC 415, `not-flac.flac` 415, `long.flac` 413,
  over-size 413 (with `--max-upload-mb 1`), bad model 400; ok flow queued → running (progress
  rises) → done with 6 stems in the documented order, stem bodies equal the fixtures, `DELETE`
  removes the job folder; `fail` → failed with the message; `bad-output`/`not-flac` → failed;
  third concurrent POST with `--queue 1` → 503; `DELETE` on `hang` kills the stub (its pid gone)
  and the state is cancelled; unknown job 404, stem before done 409, wrong method 405; a
  restart → old job 404 and its folder removed; retention with `--retention-hours 0` removes
  finished jobs on the next janitor run (janitor interval overridable by a hidden test flag);
  the server never writes outside its work dir. Client-only tests: closed port → unreachable
  within 6 s; wrong `service` → incompatible; cancel during upload stops and sends `DELETE`.
- **done when**: `cargo test -p calliope-stems --test conformance` passes in < 60 s; clippy clean.
- **test**: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no** (protocol, validation)

### Task 12: Settings: `edge_ai_url` and `keep_original`
- **files**: `src/settings.rs`
- **does**: the two fields, setters, validation (§2.9), lenient load (an invalid stored value →
  warning + default, other fields kept). Tests: `http://archserver:8765`,
  `http://192.168.2.20:8765/` (trailing slash dropped), `http://h:8765/prefix` valid; `https://…`
  ("only http:// is supported on the LAN"), `ftp://`, `http://u:p@h`, `http://h/?q`, `http://h/#f`
  invalid; empty → `None`; `keep_original` defaults to false and round-trips.
- **done when**: `cargo test settings` passes; clippy clean.
- **test**: `cargo test --workspace settings && cargo clippy --workspace --all-targets -- -D warnings`
- **routine: no**

### Task 13: Import job orchestrator
- **files**: `src/import_job.rs` (new), `src/main.rs`, `Cargo.toml` (`calliope-common` with feature `client`)
- **does**: `ImportState` (one job), a job thread per import running §2.3: url path (validate →
  `ImportTmp` → download unless complete → probe/convert (15 min) → info + tags → ready), file
  path (probe + convert the picked path → ready), extraction (strict edits → health → submit →
  poll → fetch into `Staging` → `adopt_original` when `keep_original` → commit under the repo
  lock (a closure) → remove the temp folder → saved). Cancel/failure semantics of §2.3,
  `shutdown(timeout)`, `snapshot()`, `replace_sink()`, throttling, `clean_stale` at each start.
  Headless tests use the real ffmpeg, the fake yt-dlp and the **real `calliope-stems` binary**
  (`target/debug/calliope-stems`, built by `cargo test --workspace`; the helper fails with "run
  cargo build -p calliope-stems" if missing) on `127.0.0.1:0` with the stub separator: URL ok flow
  → a scan-visible stem track with 6 stems, metadata = the edits, `import-tmp/url-*` gone;
  `keep_original` on → `original.flac` in the track and in `track.json`; mp3 and mp4 flows;
  `no-audio.mp4` exact text; `long.flac` refused before any upload (the server log shows no
  POST); 403 → failed stage download, http_status 403; resume after cancel; Start Over; cancel
  during `hang` → back to edit, `DELETE` reached the server, no staging left, temp audio kept;
  `fail` → back to edit with the message; `shutdown` during `/slow` returns within 3 s and no
  yt-dlp is left; a second start → "An import is already running"; sources byte-identical;
  nothing outside the temp root changed.
- **done when**: `cargo test import_job` passes (< 90 s); clippy clean.
- **test**: `npm test`
- **routine: no** (concurrency, cancellation)

### Task 14: Picker kinds, IPC commands, Channel events, app wiring, ACL
- **files**: `src/picker.rs`, `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json`, `tests/frontend.rs`
- **does**: `DialogKind::ImportAudio/ImportVideo` (filters, extension re-check, `ScriptedPicker`
  kinds). The 12 commands of §2.10 (async, `spawn_blocking` for IO), the Channel sink adapter,
  root-change refusal during a job, start-up health check thread (only when configured),
  `RunEvent::Exit` → `shutdown(2 s)` (`build()` + `run(|app, ev| …)`). All commands in the three
  lists; extend the static test to fail on any `core:` or plugin permission. Unit tests for the
  `do_*` helpers.
- **done when**: `npm test` passes; `cargo build --release` works; `cargo build --release
  --features e2e-hooks` still fails with the guard message.
- **test**: `npm test`
- **routine: no** (cross-cutting, security)

### Task 15: Frontend IPC types and wrappers; Library adapts to v2 records
- **files**: `src/ui/lib/ipc.ts`, `src/ui/lib/ipc-import.test.ts` (new), `src/ui/lib/fixture-tracks.ts`, `src/ui/lib/track-draft.ts`, affected `*.test.ts`
- **does**: TS mirrors of every new type/command (§2.10); `Channel` from `@tauri-apps/api/core`
  only here (wrappers take an `onEvent` callback); `TrackRecord.audio: string | null`, `type`,
  `stems`, `stem_model`, `original`; `Settings.edge_ai_url`, `keep_original`. Fixture tracks get
  `type: 'backing'`, plus one stem track. `mockIPC` tests for argument names and shapes.
- **done when**: `npm run check && npm run test:ui` pass (all existing tests green).
- **test**: `npm run check && npm run test:ui`
- **routine: no**

### Task 16: Library: track-type badges and stem info in the pane
- **files**: `src/ui/components/library/TrackTree.svelte`, `src/ui/components/library/TrackPane.svelte`, `src/ui/views/LibraryView.test.ts`, `src/ui/views/TrackPane.test.ts`
- **does**: §2.11 Library bullet.
- **done when**: vitest: a stem track row has "S" with `aria-label="Stem track"` before its name,
  a backing track "B"; the pane shows "Stem track", the six stem names and the original file;
  existing tests pass.
- **test**: `npm run check && npm run test:ui`
- **routine: no**

### Task 17: Extract `TrackFields.svelte` from `TrackPane.svelte`
- **files**: `src/ui/components/TrackFields.svelte` (new), `src/ui/components/library/TrackPane.svelte`, `src/ui/components/TrackFields.test.ts` (new)
- **does**: move the editable field grid and its error lines into `TrackFields.svelte` (props
  `draft`, `editing`, `errors`, `oninput`, `idPrefix`); TrackPane keeps identical DOM ids.
- **done when**: existing Library/TrackPane tests pass unchanged; the new test renders the fields
  in edit and view mode with an `import-` prefix.
- **test**: `npm run check && npm run test:ui`
- **routine: no** (refactor of tested UI)

### Task 18: Import view: menu, source page, download, resume prompt
- **files**: `src/ui/views/ImportView.svelte`, `src/ui/components/import/SourcePage.svelte` (new), `src/ui/lib/import-state.svelte.ts` (new), `src/ui/lib/url-check.ts` (+ test), `src/ui/lib/import-state.test.ts`, `src/ui/views/ImportView.test.ts` (new), `src/ui/lib/components/ui/progress/*`, `src/ui/lib/views.ts` (+ test)
- **does**: §2.11 menu and source page with the exact labels: "Stem Extraction"; "URL", "Local
  Audio File", "Local Video File"; "enter url"; "Extract"; "Browse"; "Entered URL is invalid",
  "Error <code> when attempting download", "Selected file <file> has no audio track",
  "Tracks longer than 15 minutes are not supported"; "Download in progress"; "Incomplete download
  file from the same URL found", "Resume Download", "Start Over". Error label `text-primary`
  under the box. Log lines §2.10.
- **done when**: vitest (ipc mocked): the button; click → source page; URL radio → box; malformed
  + Extract → the text, no start call; valid → `prepare_url_import` then `start_url_import` and the
  bar; progress events move the bar; `failed` with 403 → the text under the box; `partial` → the
  prompt; Resume/Start Over pass `resume=true/false`; audio/video radios → "Browse"; the no-audio
  and 15-minute texts; the Import placeholder text is gone.
- **test**: `npm run check && npm run test:ui`
- **routine: no**

### Task 19: Import view: edit pane, extraction progress, done/failed
- **files**: `src/ui/components/import/ImportEditPane.svelte`, `src/ui/components/import/ExtractProgress.svelte` (new), `src/ui/lib/import-state.svelte.ts`, `src/ui/views/ImportView.svelte`, tests, `src/ui/components/StatusFooter.svelte`
- **does**: §2.11 edit pane (prefilled, edit mode, `track-draft.ts` validation before
  `start_stem_extraction`, keep-original note), progress steps with "Working..." on `working`,
  done panel with "Show in Library", Cancel/discard with confirmation, footer phase text.
- **done when**: vitest: `ready` → the pane in edit mode with each value in its field; edits +
  Extract send the edited `TrackEdits`; empty title blocks Extract with the field error; `queued`
  → "Waiting for the edge-AI server..."; `working` → "Working..." + spinner; `saved` → done
  panel; `cancelled {back_to:'edit'}` keeps the values; discard asks then calls
  `discard_import`; footer follows the phase.
- **test**: `npm run check && npm run test:ui`
- **routine: no**

### Task 20: Settings: Stem extraction and External tools cards; footer status
- **files**: `src/ui/views/SettingsView.svelte`, `src/ui/components/StemSettings.svelte` (new), `src/ui/components/ToolsSettings.svelte` (new), tests, `src/ui/lib/views.ts`, `src/ui/lib/app-state.svelte.ts`, `src/ui/components/StatusFooter.svelte`, `src/ui/lib/components/ui/switch/*` (or checkbox)
- **does**: §2.11 Settings bullets (address Save/Test, keep-original switch, tools card) and the
  footer status.
- **done when**: vitest: invalid address → the Rust error inline; Save + Test → "Connected
  (htdemucs_6s)"; unreachable → the message; the switch calls `set_keep_original` and reflects
  `get_settings`; footer updates; tools card shows a missing yt-dlp's install hint.
- **test**: `npm run check && npm run test:ui`
- **routine: no**

### Task 21: GUI e2e tests for the import on display :1
- **files**: `tests/gui_import_e2e.rs` (new), `tests/common/mod.rs`, `package.json` (`test:gui`: `cargo build -p calliope-stems` first, then also `--test gui_import_e2e`)
- **does**: `start_import_app(dirs, stub_mode)` launches the e2e binary as `unshare -rn sh -c 'ip
  link set lo up && target/debug/calliope-stems --listen 127.0.0.1:8765 --work-dir <dir>/stems-work
  --separator tests/support/stub-separator 2> <dir>/stems.log & exec <app>'` (the stub mode is
  written to `<dir>/stems-work/stub-mode`), so **the app and the server have only loopback** (no
  internet, no LAN); temp XDG dirs; pre-written `settings.json` (`edge_ai_url:
  "http://127.0.0.1:8765"`, root under `target/gui-e2e/…`); `PATH` = `tests/support/bin:` + system
  path; `CALLIOPE_E2E_DIALOG_ANSWERS` for `import-audio`/`import-video`. Tests (log lines, disk,
  screenshots `target/gui-shots/import-*.png`): (a) URL happy path: Alt+2 → "Stem Extraction" →
  URL → `https://media.example/watch?v=ok` → Enter → progress → edit pane (title "Night Drive") →
  change the album → Extract → "Working..." → saved; the new `track.json` has `type: stem`, 6
  stems, the edited album, no `original`; no `import-tmp/url-*`; Library (Alt+1) shows the "S"
  badge. (b) "Entered URL is invalid". (c) `/http403` → "Error 403 when attempting download". (d)
  `/slow`, Cancel, Extract again → prompt → Resume → done (`--continue` in the fake log). (e)
  `tagged.mp3` with Keep original switched on in Settings → fields prefilled → Extract → saved with
  `original.flac`. (f) `no-audio.mp4` → "Selected file no-audio.mp4 has no audio track". (g) close
  the window during `hang` → the app exits within 5 s, no `yt-dlp`/`ffmpeg` child survives, the
  server log shows the `DELETE` and the stub was killed. Every test: no `csp-violation`, fixture
  sources unchanged.
- **done when**: `DISPLAY=:1 npm run test:gui` passes (all GUI tests) twice in a row.
- **test**: `DISPLAY=:1 npm run test:gui`
- **routine: no** (GUI)

### Task 22: Visual review on :1
- **files**: small style fixes; `docs/ui.md` ("Decided by the team")
- **does**: review screenshots of every import step, the Settings cards and the Library badges
  at 1280x800, 1024x640 and maximised 1920x1200, dark and light: readable at 1-2 m, accent error
  text visible in both themes, progress and spinner clear, nothing clipped. Record the choices.
- **done when**: reviewed, fixes committed; `npm test` and the GUI suite pass.
- **test**: `npm test && DISPLAY=:1 npm run test:gui`
- **routine: no** (visual)

### Task 23: Licences, README and the server's deployment kit
- **files**: `docs/licences.md`, `README.md`, `src/calliope-stems/deploy/calliope-stems.service` (new), `src/calliope-stems/README.md` (new), `tests/frontend.rs` (licence test covers the member manifests)
- **does**: licence rows for `ureq`, `url`, `libc`, `tiny_http` and every direct dependency of
  the two new crates (licences read from `cargo metadata`); shadcn `progress` and `switch`
  rows; "External tools (not bundled)" (yt-dlp Unlicense; ffmpeg/ffprobe LGPL-2.1+/GPL; on
  archserver: audio-separator (MIT), Demucs (MIT) and the htdemucs_6s weights (check and record
  their licence from the model's source), PyTorch (BSD-3) — the owner's install, used through the
  adapter, never bundled); "Test-only tools" (python3 for the fake yt-dlp). The licence test reads
  the member `Cargo.toml`s too. Server README + example unit (`systemctl --user`):
  ```
  [Unit]
  Description=Calliope stems service (calliope-stems API v1)
  After=network-online.target
  [Service]
  Environment=STEMS_HOME=%h/edge-ai/stems
  ExecStart=%h/.local/bin/calliope-stems --listen 0.0.0.0:8765 --work-dir %h/.local/state/calliope-stems --separator %h/.local/bin/calliope-separate-audio-separator --model htdemucs_6s
  Restart=on-failure
  [Install]
  WantedBy=default.target
  ```
  and the manual install steps of §5. Root README: laptop prerequisites (`pacman -S yt-dlp
  ffmpeg`), Settings > Stem extraction, where temp files live.
- **done when**: `npm test` passes (licence record test); `systemd-analyze verify --user
  src/calliope-stems/deploy/calliope-stems.service` reports no syntax errors other than the
  missing ExecStart binary (or the step is skipped with a note if `systemd-analyze` can't run
  in the sandbox). Nothing is installed.
- **test**: `npm test`
- **routine: no**

## 4. Test strategy

- **Whole headless suite**: `npm test` (= svelte-check, vitest, vite build, `cargo build -p
  calliope-stems`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`). **GUI suite**: `DISPLAY=:1 npm run test:gui` (builds `calliope-stems`, adds
  `gui_import_e2e`).
- **Stand-ins** (no test reaches the internet or the LAN, none runs the real model):
  - edge-AI server → the **real `calliope-stems` binary** bound to 127.0.0.1 with
    `tests/support/stub-separator` (canned stems, failure modes);
  - the separator model → the stub (the real adapter is only syntax-checked);
  - YouTube / yt-dlp → `tests/support/bin/yt-dlp` (refuses non-`.example` hosts);
  - ffmpeg/ffprobe → the real local tools on self-generated fixtures;
  - GUI e2e: app and server inside `unshare -rn` with only loopback up.
- **Protocol conformance** (`src/calliope-stems/tests/conformance.rs`): every endpoint, status
  code and limit, both with raw HTTP and with the shared client, so client and server can't drift.
- **Unit (Rust)**: STREAMINFO parser, process runner, schema v2/migration, repository v2 +
  never-rewrite byte checks, temp space/staging (symlinks, hand-placed files), tools, media
  mapping + conversion + 15-min limit, URL table + yt-dlp parsing + resume, settings, server CLI/
  routes/output validation/cleanup, job orchestration incl. cancel/shutdown.
- **Frontend (vitest + jsdom)**: URL check, import store (ipc mocked), every step and label,
  Library badges, Settings cards, footer.
- **End-to-end (display :1)**: task 21 flows with on-disk assertions and screenshots.
- **Data safety everywhere**: "outside" folders unchanged; source fixtures byte-identical; v1
  `track.json` bytes unchanged after scan/list/import; the server writes only in its work dir.
- **Separation**: static `cargo metadata` test (no Tauri/GTK in the server or common crate; GUI
  doesn't depend on the server).

## 5. Deployment

- **Laptop** (calliope-gui): local build as before (`npm ci && npm run build:app`); install
  `yt-dlp` and `ffmpeg` (`sudo pacman -S yt-dlp ffmpeg`).
- **archserver** (`calliope-stems`), **manual, by the owner** (agents never deploy, install or
  start it against the real model):
  1. `cargo build --release -p calliope-stems` in `~/src/calliope`.
  2. `install -m755 target/release/calliope-stems ~/.local/bin/` and
     `install -m755 src/calliope-stems/separators/audio-separator.sh ~/.local/bin/calliope-separate-audio-separator`.
  3. `mkdir -p ~/.config/systemd/user && cp src/calliope-stems/deploy/calliope-stems.service ~/.config/systemd/user/`
  4. `systemctl --user daemon-reload && systemctl --user enable --now calliope-stems`
  5. So it runs without a login session: `loginctl enable-linger vali` (may ask for your password).
  6. `journalctl --user -u calliope-stems -f` shows `listening addr=0.0.0.0:8765`.
  Updating: repeat 1-2, then `systemctl --user restart calliope-stems`.

## 6. Manual checks (owner)

On archserver (the server):
1. [ ] `calliope-stems --help` lists the flags; without `--separator` it refuses to start.
2. [ ] After the deployment steps, `curl http://192.168.2.20:8765/v1/health` from the laptop
   shows `"service":"calliope-stems"` and `htdemucs_6s`.
3. [ ] A real song (≤ 15 min) through Calliope: `journalctl --user -u calliope-stems` shows the
   job running, `nvidia-smi` shows the GPU busy, Ollama models are unloaded only when VRAM is
   short; after the import the job folder under `~/.local/state/calliope-stems/jobs/` is gone.
4. [ ] `systemctl --user restart calliope-stems` during a job: Calliope reports "The edge-AI
   server restarted; try again"; Extract again works.

On the laptop, with a scratch repository (a copy of `tests/fixtures/library-v2`):
5. [ ] Settings > External tools shows the yt-dlp and ffmpeg versions.
6. [ ] Settings > Stem extraction: address `http://archserver:8765` (or `http://192.168.2.20:8765`)
   → Save → Test connection → "Connected (htdemucs_6s)"; the footer says connected.
7. [ ] Library shows the stem track with "S" and backing tracks with "B"; your real v1
   repository opens with no problems and no `track.json` changes (`md5sum` before/after).
8. [ ] Import a real mp3: fields prefilled from its tags → Extract → "Working..." → a new track
   with six stems that sound right; your mp3 is untouched; `<root>/import-tmp/` is empty.
9. [ ] With "Keep the original mix" on, the next import also has `original.flac` and
   `"original"` in `track.json`.
10. [ ] YouTube link: progress bar moves; sensible band/title; full extraction works.
11. [ ] Resume: start a long download, Cancel, Extract the same URL → the prompt; Resume continues;
   Start Over begins at 0.
12. [ ] A video (phone clip/mkv) → audio extracted; a silent video → "Selected file <name> has no
   audio track"; a 20-minute file → the 15-minute message, nothing uploaded.
13. [ ] Close Calliope during "Working...": it quits within a few seconds; `pgrep yt-dlp ffmpeg`
   finds nothing; the server log shows the job cancelled.
14. [ ] Readable from 1-2 m in a dim room; a keyboard-only run (Tab, Enter, Escape).
15. [ ] Edit and Save an old v1 track: it now has `"schema_version": 2` and `"type": "backing"`,
   everything else unchanged.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| Import tab shows "Stem Extraction" | 18 | vitest ImportView; e2e (a) |
| Click → stem extraction mode, "select source" page | 18 | vitest; e2e (a) |
| URL option → "enter url" box | 18 | vitest; e2e (a) |
| Extract → URL format checked | 8, 18 | `download` URL table; `url-check.test.ts`; vitest |
| Malformed → accent text "Entered URL is invalid" under the box | 18, 22 | vitest (text + class); e2e (b) + screenshot |
| Valid → "Download in progress" bar | 13, 14, 18 | vitest; e2e (a) |
| Download progress updates the bar | 8, 13, 18 | `download` progress; vitest; e2e (a) |
| Download error → "Error <HTTP code> when attempting download" | 8, 13, 18 | `download` 403; `import_job` 403; vitest; e2e (c) |
| Complete download → temp file inside the repository | 5, 8, 13 | `import_job` URL flow (asserts `import-tmp/url-*/download.webm`) |
| Existing temp file → prompt + Resume Download / Start Over | 5, 8, 13, 18 | `import_tmp` find_partial; `import_job` resume/start-over; vitest; e2e (d) |
| Download finished + metadata → metadata initialised | 7, 8, 13 | `download` info mapping; `media` tags; `import_job` |
| Download finished → edit pane in edit mode | 19 | vitest; e2e (a) |
| Extracted metadata fills the fields | 19 | vitest; e2e (a) ("Night Drive") |
| Extract → stem extraction starts | 10, 11, 13, 19 | conformance; `import_job`; vitest; e2e (a) |
| Processing finished → temp file removed | 5, 13 | `import_job` (no `import-tmp/url-*`); e2e (a) |
| Edge-AI confirms start → "Working..." spinner | 10, 11, 13, 19 | conformance (running state); `import_job` events; vitest; e2e (a) screenshot |
| Complete → stems saved under a track folder with the edited metadata | 4, 5, 13 | `import_job` (scan + metadata = edits); e2e (a) disk check |
| Library identifies stem tracks with a distinctive icon | 16 | vitest; e2e (a) screenshot |
| "Local Audio File" → "Browse" | 18 | vitest; e2e (e) |
| "Local Video File" → "Browse" | 18 | vitest; e2e (f) |
| Video/audio metadata → repository format | 7, 13 | `media` tag tests (mp3, ogg, mp4); `import_job` |
| Video with audio → audio extracted to a temp file | 7, 13 | `media` to_flac on `with-audio.mp4`; `import_job` mp4 flow |
| Video without audio → "Selected file <file> has no audio track" | 7, 13, 18 | `media`; `import_job`; vitest; e2e (f) |
| Audio file metadata → repository format | 7 | `media` mp3/ogg/flac |
| Audio file → edit pane with the extracted metadata | 19 | vitest; e2e (e) |
| Req 1 YouTube link | 8, 13 | fake-based tests; manual check 10 |
| Req 2 mp3/flac/ogg | 7, 14 | `media` on all three; picker filter test |
| Req 4 edge-AI infrastructure | 9, 10, 11, 13 | conformance suite; `import_job`; manual checks 1-4, 8 |
| Req 5 stems in the repository with type "stem" | 3, 4, 5, 13 | `track_meta`, `repository`, `import_job` |
| Req 6 metadata format refactored | 3, 4 | schema v2 + migration; never-rewrite byte test |
| Req 7 metadata extracted and edited before extraction | 7, 8, 19 | as above |
| Owner Q3 keep original (setting, default off) | 3, 5, 12, 13, 20 | `import_tmp` adopt_original; `import_job` keep on/off; vitest; e2e (e) |
| Owner Q4 15-minute limit (client and server) | 2, 7, 10, 11 | `media` long.flac; conformance 413; `import_job` (no POST) |

## 8. Assumptions (defaults chosen; tell us to change any)

- **Temp files** (spec open question): `<root>/import-tmp/`, one folder per URL (resumable) or
  per local-file import, removed after the track is saved.
- **Icons** (spec open question): letter badges **"S"** (stem, amber outline) and **"B"**
  (backing, neutral), our own design.
- Audio is normalised to FLAC 44.1 kHz stereo before upload; stems are stored as delivered
  (FLAC) in `stems/<name>.flac`; the original (when kept) as `original.flac`.
- Non-HTTP download errors show "Download failed: <reason>".
- One import at a time on the laptop; the server runs 1 job with a queue of 2.
- Picking a local file starts preparing at once (no extra click).
- After a cancelled or failed extraction the user is back in the edit pane with metadata and
  audio kept; "Cancel" there (confirmed) discards the import.
- Duplicate tracks (same band/title) are allowed.
- Saving a v1 track writes schema 2 (older builds then show it read-only).
- The edge-AI URL is `http://` only and unauthenticated; the server listens on `0.0.0.0:8765`.
- Server job state is in memory; a restart drops jobs (clients are told to retry).
- Server upload cap 300 MB, separator timeout 30 min, retention 24 h.
- The views registry calls this feature `gui-stem-extracting` (as the overview roadmap does);
  the spec file is `gui-stem-extraction.md`. The placeholder texts using the name go away.

## 9. Open questions

None.

## 10. Owner decisions (2026-10-06)

- **Q1 (edge-AI service)**: build it in this feature as the separate Rust binary
  `calliope-stems`, implementing "calliope-stems API v1" ("looks good as a starter"; may get its
  own spec later, so it is kept cleanly separated: a workspace member under `src/`, no Tauri, the
  GUI doesn't depend on it). It wraps a configurable separator command (on archserver:
  audio-separator with `htdemucs_6s`). Tests use a stub separator on 127.0.0.1 only; the real
  model never runs in tests. Deployment is manual by the owner (README + example systemd user
  unit). The Python fake server is dropped; the GUI e2e tests run the real binary with the stub.
- **Q2 (URL import)**: yes, as specified, for personal use; any http/https URL yt-dlp supports;
  yt-dlp is installed by the owner, not bundled.
- **Q3 (original mix)**: configurable in Settings ("Keep the original mix with the stems",
  default off); when on, stored as FLAC in the track folder and recorded as `original` in
  `track.json` v2.
- **Q4 (length)**: at most 15 minutes, enforced by Calliope before upload and by the server.
