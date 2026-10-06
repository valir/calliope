# Report: Import and stem extraction (+ `calliope-stems` service)

**Status: DONE.** All acceptance criteria pass. The reviewer approved after 1 fix round. Nothing touched the LAN, the internet or the real model. Real-world checks are manual (below).

- Branch: `spec/gui-stem-extraction` (base `0a8892c`), not pushed or merged
- Plan: `specs/gui-stem-extraction.plan.md`, log: `specs/gui-stem-extraction.log.md`
- Deployment of `calliope-stems` is yours: see `src/calliope-stems/README.md`

## Acceptance criteria

| Criterion | Tests / evidence | Result |
|---|---|---|
| Import tab shows "Stem Extraction"; click opens the source page | vitest; GUI `qa_walkthrough` | PASS |
| URL option shows "enter url"; Extract validates; malformed gives accent "Entered URL is invalid" | vitest URL table (17 bad / 4 good); Rust; GUI | PASS |
| Valid URL shows "Download in progress", bar follows progress | vitest; GUI `import_url_happy_path` | PASS |
| Download error shows "Error <code> when attempting download" | Rust (403); GUI `import_url_http_403` | PASS |
| Download lands as a temp file in the repository | Rust (`import-tmp/url-*/download.*`) | PASS |
| Existing partial: "Incomplete download file…" with Resume Download / Start Over | Rust; vitest; GUI resume and start-over tests; SIGKILL-then-resume | PASS |
| Metadata extracted and shown in the edit pane, in edit mode | Rust (mp3/ogg/flac/mp4/URL info); vitest; GUI | PASS |
| Extract starts extraction; "Working..." only once the server confirms it started | vitest; Rust event order; GUI | PASS |
| Temp file removed; stems saved as a new track with the edited metadata | Rust; GUI disk checks (v2, `type: stem`, 6 FLAC stems) | PASS |
| Library marks stem tracks with an icon before the name | vitest; GUI screenshot ("S" badge) | PASS |
| Local Audio / Local Video file: Browse, metadata, edit pane | vitest; Rust; GUI `import_local_audio`, `import_local_video` | PASS |
| Video without audio gives "Selected file <file> has no audio track" | Rust; GUI | PASS |
| Req 4: edge-AI extraction | real `calliope-stems` binary + stub separator in every test; 22 server + 15 conformance tests | PASS |
| Req 6: metadata schema refactor | v2 + in-memory migration; v1 files byte-identical after scan, import, cancel and crash | PASS |
| Owner: 15-minute limit | client before upload; server 413; `-t` cap on conversion; yt-dlp duration filter | PASS |
| Owner: keep original configurable (default off) | GUI `import_keep_original` | PASS |
| Real model, real YouTube, real archserver | not run by design | MANUAL (below) |

Suites:
- Headless (`npm test`, display unset):
  - vitest: 287 passed
  - Rust workspace: about 800 tests, 0 failed
  - clippy: clean
- GUI tests (`DISPLAY=:1 npm run test:gui`): 5 + 13 import + 8 + 4 acceptance + 1 smoke, all green over multiple runs. They run inside a loopback-only network namespace, which the tests assert.

## Your decisions applied
- `calliope-stems` was built in this feature as a separate workspace crate implementing API v1. It has no authentication, which is acceptable on the LAN.
- URL import works for any http/https URL yt-dlp supports, for personal use. yt-dlp is installed by you, not bundled.
- "Keep the original mix with the stems" is a setting, off by default. When on, the track gets `original.flac` and an `original` field in `track.json`.
- The maximum length is 15 minutes, enforced by the app, by the server, and by limits passed to ffmpeg and yt-dlp.

## Design summary
- **Workspace:**
  - the GUI, built by plain `cargo build` / `npm run build:app` as before
  - `src/calliope-common` (protocol types, FLAC header parser, process runner, HTTP client)
  - `src/calliope-stems` (the server, on `tiny_http`)

  A test keeps them separate: the server never pulls in Tauri or GTK, and the GUI never depends on the server.
- **Import flow:** download (yt-dlp) or read a local file, then ffprobe metadata, convert to FLAC 44.1 kHz stereo, upload, poll, fetch 6 stems, and stage the track in `tracks/.staging-<id>/`. One atomic rename makes it appear in the Library. Progress reaches the UI over a Tauri Channel, and there is one job at a time.
- **Data safety:**
  - Existing v1 tracks are never rewritten unless you Save them in the Library.
  - Temp and staging cleanup deletes only Calliope-marked folders and never follows symlinks.
  - Your source files are only ever read.
  - A crash at any stage leaves the Library intact. The next import cleans Calliope's own leftovers, and an interrupted URL download can be resumed.
- **Security:**
  - URLs must be http/https.
  - All tools are run with argv lists, never a shell. yt-dlp gets `--ignore-config`, the URL after `--`, `--max-filesize 1G` and a live/duration filter.
  - ffmpeg and ffprobe get `-protocol_whitelist file,pipe`.
  - Server responses are validated: job ids, stem names, count, size and FLAC magic.
  - The frontend never sends paths.
  - The 26 app commands carry no core, plugin or event permissions. The CSP is unchanged, and `e2e-hooks` is still refused in release builds.

## Task log

| Task | Done by |
|---|---|
| 1 fixtures · 2 workspace + calliope-common | implementer |
| 3–4 schema v2 + repository v2 | implementer |
| 5–7 temp space/staging, tools, media | implementer |
| 8–9 download + fake yt-dlp, stub separator + real adapter (syntax-checked only) | implementer |
| 10 calliope-stems server · 11 client + conformance · 12 settings | implementer |
| 13 import job orchestrator · 14 IPC/Channel/ACL | implementer |
| 15–17 frontend v2, badges, TrackFields · 18–19 Import view · 20 Settings cards | implementer |
| 21 import GUI e2e · 22 visual review · 23 licences + READMEs + user unit | implementer |
| fix round 1 | implementer |

No task was marked routine. The local model has failed every task beyond a few lines.

## Review
- **First review:** CHANGES REQUIRED. The major: an early error in `start_extraction` left the job stuck as "running". The tester found no acceptance failures and 4 low findings.
- **Fix round 1:**
  - The job state is rolled back on early errors.
  - yt-dlp has size, live and duration limits.
  - ffmpeg has a protocol whitelist and a `-t` cap.
  - Stem downloads can be cancelled.
  - A few failed status polls are tolerated.
  - Server error text is truncated.
  - `prepare_url` creates nothing on disk.
  - yt-dlp fragment files are ignored.
  - A re-attach failure is shown in the UI.
- **Re-review:** APPROVED. The re-test passed with 9 new probes.

## Known limits (documented in `docs/architecture.md` and the server README)
- **Uploads:** the server has no upload read timeout and starts one thread per connection, so stalled uploads can hold queue slots. This follows from your decision that the service is LAN-only with no auth.
- **Force-kill:** if Calliope is force-killed (SIGKILL), grandchildren such as yt-dlp's ffmpeg can survive, and the server keeps separating until its timeout (30 min). A normal close cancels everything within about 2 s.
- **yt-dlp plugin folders:** `--no-plugin-dirs` isn't used, because the minimum yt-dlp version may not support it.

## Follow-ups (minor)
- `STEM_TIMEOUT` is 5 minutes for the whole stem. A stem of about 300 MB over slow Wi-Fi could time out; consider 10–15 minutes.
- The footer's edge-AI status doesn't refresh from job results.
- The GUI e2e tests navigate by Tab counts from a fixed click point. They work and wait on log lines, but they're fragile. Consider stable selectors.
- The Library's client-side tablature clash check doesn't know about `original.flac` (Rust rejects the clash anyway).
- A rare spawn-failure path resets a "failed" phase to "ready" (cosmetic).

## Licences (`docs/licences.md`)
- **Bundled crates and npm packages:** all permissive. A test now covers every workspace crate.
- **External, not bundled:**

  | Tool | Licence |
  |---|---|
  | ffmpeg / ffprobe | GPL-3.0 (run as a separate program only) |
  | yt-dlp | Unlicense |
  | audio-separator | MIT |
  | PyTorch | BSD-3-Clause, plus NVIDIA libraries |

- **To be confirmed by you:**
  - the licence of the htdemucs_6s model weights (downloaded from Meta's `dl.fbaipublicfiles.com`)
  - the Demucs code licence (no LICENSE file was found locally)

## How to try it
```sh
npm ci && npm run build:app                      # GUI
cargo build --release -p calliope-stems          # server (on archserver)
DISPLAY=:1 npm run test:gui                      # GUI e2e incl. import; screenshots in target/gui-shots/import-*.png
```
Server deployment: follow `src/calliope-stems/README.md` (install the binary and `separators/audio-separator.sh` into `~/.local/bin`, copy `deploy/calliope-stems.service` to `~/.config/systemd/user/`, `systemctl --user enable --now calliope-stems`, `loginctl enable-linger`). Agents did not deploy anything.

## Manual checks
On archserver (the server):
- [ ] `calliope-stems --help` lists the flags; without `--separator` it refuses to start
- [ ] After deployment, `curl http://192.168.2.20:8765/v1/health` from the laptop shows `calliope-stems` and `htdemucs_6s`
- [ ] A real song (≤ 15 min): the job shows in `journalctl --user -u calliope-stems`, `nvidia-smi` shows the GPU busy, and the job folder is gone afterwards
- [ ] `systemctl --user restart calliope-stems` during a job: Calliope says the server restarted; Extract again works

On the laptop (use a copy of `tests/fixtures/library-v2` first):
- [ ] Settings > External tools shows the yt-dlp and ffmpeg versions
- [ ] Settings > Stem extraction: `http://archserver:8765` → Save → Test connection shows connected; so does the footer
- [ ] Your real v1 repository opens with no problems and no `track.json` changes (`md5sum` before and after)
- [ ] Import a real mp3: the fields are prefilled, Extract gives 6 stems that sound right, your mp3 is untouched, and `import-tmp/` is empty
- [ ] With "Keep the original mix" on, the next import also has `original.flac`
- [ ] YouTube link: the progress bar moves, band and title are sensible, and full extraction works. Also confirm that yt-dlp writes `info.info.json` for `--output infojson:info`; the fake mimics this, but it was never checked against the real tool.
- [ ] Resume: start a long download, Cancel, enter the same URL again; Resume continues, and Start Over begins at 0
- [ ] A video imports; a silent video gives "Selected file <name> has no audio track"; a 20-minute file gives the 15-minute message and nothing is uploaded
- [ ] Close Calliope during "Working...": it quits within a few seconds, no yt-dlp or ffmpeg is left, and the server log shows the job cancelled
- [ ] Readable from 1–2 m in a dim room; a keyboard-only run works
- [ ] Edit and Save an old v1 track: it now has `"schema_version": 2` and `"type": "backing"`, with everything else unchanged
- [ ] Confirm the licences of the htdemucs_6s weights and the Demucs code

Once you've verified it, tick **gui-stem-extraction** in the roadmap of `specs/overview.md`.
