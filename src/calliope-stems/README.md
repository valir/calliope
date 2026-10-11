# calliope-stems

The edge-AI stem separation server for Calliope. It implements "calliope-stems API v1": the
GUI uploads a FLAC file, the server runs a separator command on it (on the owner's server:
Demucs `htdemucs_6s` through `audio-separator`, on the GPU) and the GUI downloads six FLAC stems
(vocals, drums, bass, guitar, piano, other). It is a workspace member of the Calliope repository
(`src/calliope-stems`), has no Tauri or GTK dependency and is not used by the GUI at build time
(the GUI only shares `calliope-lib` with it).

## Build

```bash
cargo build --release -p calliope-stems      # target/release/calliope-stems
cargo test -p calliope-stems                 # unit, smoke and protocol conformance tests (stub separator, 127.0.0.1)
```

## Run

```bash
calliope-stems --separator PATH [options]
```

`--separator` is required (there is no default). Flags can be written `--flag value` or `--flag=value`.

| Flag | Default | Meaning |
|---|---|---|
| `--separator PATH` | none (required) | the separator executable, run as `PATH <input.flac> <out_dir> <model>` |
| `--listen ADDR:PORT` | `0.0.0.0:8765` | address to bind (port 0 picks a free port) |
| `--work-dir DIR` | `$XDG_STATE_HOME/calliope-stems`, else `~/.local/state/calliope-stems` | job files |
| `--model NAME` | `htdemucs_6s` | model name advertised on `/v1/health` and passed to the separator |
| `--max-upload-mb N` | 300 | larger uploads are refused with 413 |
| `--max-duration-s N` | 900 | longer audio (15 minutes) is refused with 413 |
| `--queue N` | 2 | jobs allowed to wait behind the running one |
| `--separator-timeout-min N` | 30 | a separator running longer is killed |
| `--retention-hours N` | 24 | finished jobs are deleted after this |
| `--beat-tracker PATH` | none (beats off) | the beat tracker executable, run as `PATH <input.flac> <out_dir>`; without it `POST /v1/beats` answers 501 and `/v1/health` lists only the `stems` task |
| `--beat-timeout-min N` | 10 | a beat tracker running longer is killed |
| `--no-stem-levels` | off | do not measure the stems' audible time; the job status then has no `stem_levels` |
| `--help`, `--version` | | |
| `--licenses` | | prints the licences of the third-party code in the binary (`THIRD-PARTY-NOTICES.txt`; regenerate with `npm run notices:stems` in `src/calliope-gui` after changing dependencies) |

`calliope-stems --measure FILE...` is a diagnostic: it starts no server and needs no `--separator`. For each
FLAC file it prints `<file> audible_ms=<n> windows=<w> level_dbfs=-40 peak_dbfs=<x>` (100 ms windows above
-40 dBFS, the exact measure the server and the app use), or `<file> error=<msg>`. It exits 0 when every file
was measured, 1 otherwise.

The service has no authentication and no TLS: it is meant for a trusted home network. Do not
expose it to the internet. The GUI talks plain `http://` to it (Settings > Stem extraction).

## HTTP API v1

* `GET /v1/health`: JSON with `service` (`calliope-stems`), `api` (1), `version`, `models`,
  `default_model`, `busy`, `max_duration_s`, `max_upload_bytes` and `tasks` (`["stems"]`, or
  `["stems","beats"]` with a beat tracker; absent on older servers = stems only)
* `POST /v1/jobs[?model=NAME]`: the FLAC as the request body (`Content-Type: audio/flac`, a
  `Content-Length` is required); returns the job id (413 too large or too long, 415 not FLAC,
  503 queue full)
* `GET /v1/jobs/<id>`: status and progress. When the job is `done` the status also has
  `stem_levels`, measured by the server (see below)
* `GET /v1/jobs/<id>/stems/<name>`: one finished stem as FLAC
* `POST /v1/beats`: like `POST /v1/jobs` (same body and 411/413/415/503), without a model; returns
  `202 {"job","state":"queued"}`. `501` when the server has no `--beat-tracker`
* `GET /v1/jobs/<id>/beats`: the beats document of a done beats job (see below); 409 if the job is
  not done (or failed), 404 if the job is unknown or is not a beats job
* `DELETE /v1/jobs/<id>`: cancel (kills the separator or tracker) and remove the job's files

Beats jobs share the one queue and worker with stem jobs (one GPU). The status of a beats job
has `"kind":"beats"` and `"stems":null`; the status of a stem job has no `kind` key. `stems/<name>`
of a beats job is 404. No `stem_levels` are measured for beats jobs.

### The beats document

```json
{"beats":[0.5,1.0,1.5,2.0],"downbeats":[0.5],"tracker":"beat_this 1.1.0 final0 cuda"}
```

Seconds, finite, `0 <= t <= duration + 1`, strictly increasing, at most 20000 beats, every
downbeat equal (within 1e-6 s) to a beat, `tracker` at most 100 characters, at most 1 MiB of
JSON. The server checks it against the upload's duration (`calliope_lib::stems_api::check_beats`);
a tracker whose output fails the check fails the job ("invalid output"). The log has
`job id=… kind=beats state=…` and `job id=… beats=<n> downbeats=<m> tracker="…"`.

### `stem_levels`

After the separator succeeds the server decodes every stem once (in stem order, a few seconds
per song; the job stays `running` meanwhile) and reports how long it is audible: the number of
100 ms windows whose RMS (loudest channel) is above -40 dBFS, times 100 ms
(`calliope_lib::flac_level`):

```json
{"job":"…","state":"done","progress":1.0,"stems":["vocals","drums","bass","guitar","piano","other"],"error":null,
 "stem_levels":[{"name":"vocals","audible_ms":200600,"level_dbfs":-40,"window_ms":100,"peak_dbfs":-1.23},
                {"name":"piano","audible_ms":0,"level_dbfs":-40,"window_ms":100,"peak_dbfs":null}]}
```

* `audible_ms`, `level_dbfs` and `window_ms` (integers) are the data: the level and window the
  time was counted at are always stated, so a client uses a report only if it matches its own
  measure. `peak_dbfs` is only for humans (`null` for digital silence); clients ignore it.
* Not measured is not silent. A stem the server could not decode has **no entry** (the job still
  succeeds, and the server log says `stem=<name> audible=unknown`). The key is absent (never
  `null`) while the job is not `done`, on servers without this feature and with
  `--no-stem-levels`.
* The log has one line per stem (`job id=… stem=… audible_ms=… windows=… level_dbfs=-40
  peak_dbfs=…`) and a summary `job id=… measured=<n>/<total> ms=<ms>`.

`src/calliope-stems/tests/conformance.rs` is the executable specification of every endpoint,
status code and limit.

## Known limits

* No read timeout on uploads and one thread per connection: a few stalled uploads (a declared
  `Content-Length` that never completes) can occupy every queue slot until the clients close.
  Accepted: the server is for a trusted LAN and has no authentication (owner decision). The
  ignored test `finding_stalled_uploads_lock_the_queue` shows it.
* If the Calliope app is killed (SIGKILL) mid-job, the server does not notice: it keeps
  separating until its own job timeout, then the result is dropped.
* `PR_SET_PDEATHSIG` (used by the GUI for its child processes) only covers direct children.

## The separator contract

The server never loads a model itself. It runs the `--separator` executable once per job, in
the job's folder:

```
<separator> <input.flac> <out_dir> <model>
```

* `<input.flac>`: the uploaded audio; `<out_dir>`: an existing, empty folder; `<model>`: the
  `--model` value (a plain name: letters, digits, `_`, `.`, `-`).
* The separator must write `<out_dir>/<stem>.flac` for each of `vocals`, `drums`, `bass`,
  `guitar`, `piano`, `other` (real FLAC files) and exit 0. Any other exit status fails the job
  and the failure (with what the separator wrote on stderr) is reported to the GUI.
* stdout may carry `progress <0..1>` lines (for example `progress 0.5`); the server relays them
  as job progress. Everything else is logged.
* The server kills the separator (the whole process group) on cancel, on timeout
  (`--separator-timeout-min`) and when the server itself exits.

Two implementations live in this repository:

* `separators/audio-separator.sh`: the real adapter (`audio-separator` with the Demucs
  `htdemucs_6s` model on the GPU). It looks for `$STEMS_HOME/.venv/bin/audio-separator` and
  `$STEMS_HOME/models/` (default `~/edge-ai/stems`) and, if less than 2500 MiB of VRAM are free,
  asks a local Ollama (`$OLLAMA`, default `http://127.0.0.1:11434`) to unload its models first.
* `src/calliope-gui/tests/support/stub-separator` (the shared test stubs live in the GUI crate's `tests/`): copies canned stems, with failure
  modes. All automated tests use the stub; no test runs the real model.

## The beat tracker contract

With `--beat-tracker`, the server runs the executable once per beats job, in the job's folder:

```
<tracker> <input.flac> <out_dir>
```

* It must write `<out_dir>/beats.json` (the beats document above) and exit 0.
* stdout may carry `progress <0..1>` lines; the server relays them as job progress.
* Any other exit status, a missing or invalid `beats.json`, or running longer than
  `--beat-timeout-min` fails the job; the message (and the tail of stderr) goes to the client.
* The server kills the tracker's whole process group on cancel, on timeout and when it exits.

`src/calliope-gui/tests/support/stub-beat-tracker` is the test stub (modes `ok`, `steady`,
`bar-end`, `few`, `fail`, `slow`, `hang`, `bad-output`); the real adapter is
`trackers/beat-this.sh` (Beat This! with the `final0` checkpoint, in its own virtualenv). The
hidden `--beat-timeout-ms` (at least 100) exists for the tests only.

## Manual deployment (by the owner)

Nothing in this repository installs or starts the service. On the server (archserver), in a
checkout of this repository:

1. Build: `cargo build --release -p calliope-stems`
2. Install the two executables:
   ```bash
   install -m755 target/release/calliope-stems ~/.local/bin/
   install -m755 src/calliope-stems/separators/audio-separator.sh ~/.local/bin/calliope-separate-audio-separator
   ```
3. Install the example unit (check the paths and `STEMS_HOME` in it first):
   ```bash
   mkdir -p ~/.config/systemd/user
   cp src/calliope-stems/deploy/calliope-stems.service ~/.config/systemd/user/
   ```
4. Start it and enable it at login: `systemctl --user daemon-reload && systemctl --user enable --now calliope-stems`
5. To keep it running without a login session: `loginctl enable-linger <user>` (may ask for a password).
6. Check: `journalctl --user -u calliope-stems -f` shows `listening addr=0.0.0.0:8765`, and
   `curl http://<server>:8765/v1/health` from the laptop answers with `"service":"calliope-stems"`.

Updating: repeat steps 1-2, then `systemctl --user restart calliope-stems`.

### Enabling beat detection (by the owner)

On the server, in the checkout:

1. `git pull && cargo build --release -p calliope-stems`
2. Create the environment (separate from the Demucs one):
   ```bash
   uv venv --python 3.12 ~/edge-ai/beats/.venv
   uv pip install --python ~/edge-ai/beats/.venv/bin/python \
       --index-url https://download.pytorch.org/whl/cu128 torch==2.11.0 torchaudio==2.11.0
   uv pip install --python ~/edge-ai/beats/.venv/bin/python beat-this==1.1.0 soundfile
   mkdir -p ~/edge-ai/beats/models
   curl -fL -o ~/edge-ai/beats/models/final0.ckpt \
       https://cloud.cp.jku.at/public.php/dav/files/7ik4RrBKTS273gp/final0.ckpt
   ```
3. Install the binaries:
   ```bash
   install -m755 target/release/calliope-stems ~/.local/bin/
   install -m755 src/calliope-stems/trackers/beat-this.sh ~/.local/bin/calliope-beats-beat-this
   ```
4. Smoke test: `BEATS_HOME=~/edge-ai/beats calliope-beats-beat-this song.flac /tmp/bt && head -c 300
   /tmp/bt/beats.json` prints progress lines, then the JSON with `beats`, `downbeats` and `tracker` (which
   names the device).
5. In `~/.config/systemd/user/calliope-stems.service` add `Environment=BEATS_HOME=%h/edge-ai/beats` and
   `--beat-tracker %h/.local/bin/calliope-beats-beat-this` (as in the example unit), then
   `systemctl --user daemon-reload && systemctl --user restart calliope-stems`.
6. From the laptop: `curl http://archserver:8765/v1/health` shows `"tasks":["stems","beats"]`.
7. Optional: compare the licences in `~/edge-ai/beats/.venv/lib/python3.12/site-packages/*.dist-info/METADATA`
   with `docs/licences.md`.

The Python environment, `audio-separator` and the model weights are the owner's own install
(`~/edge-ai/stems`); they are not part of this repository. Their licences are listed in
`docs/licences.md`.
