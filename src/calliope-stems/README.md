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
| `--help`, `--version` | | |
| `--licenses` | | prints the licences of the third-party code in the binary (`THIRD-PARTY-NOTICES.txt`; regenerate with `npm run notices:stems` in `src/calliope-gui` after changing dependencies) |

The service has no authentication and no TLS: it is meant for a trusted home network. Do not
expose it to the internet. The GUI talks plain `http://` to it (Settings > Stem extraction).

## HTTP API v1

* `GET /v1/health`: JSON with `service` (`calliope-stems`), `api` (1), `version`, `models`,
  `default_model`, `busy`, `max_duration_s` and `max_upload_bytes`
* `POST /v1/jobs[?model=NAME]`: the FLAC as the request body (`Content-Type: audio/flac`, a
  `Content-Length` is required); returns the job id (413 too large or too long, 415 not FLAC,
  503 queue full)
* `GET /v1/jobs/<id>`: status and progress
* `GET /v1/jobs/<id>/stems/<name>`: one finished stem as FLAC
* `DELETE /v1/jobs/<id>`: cancel (kills the separator) and remove the job's files

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

The Python environment, `audio-separator` and the model weights are the owner's own install
(`~/edge-ai/stems`); they are not part of this repository. Their licences are listed in
`docs/licences.md`.
