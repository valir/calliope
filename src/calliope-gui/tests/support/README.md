# Test support: stand-ins for external programs

Nothing here reaches the internet, the LAN or the real model.

## `bin/yt-dlp` (fake yt-dlp, Python 3)

Put the directory on `PATH` (or pass the file's path) instead of the real yt-dlp.

- `--version` prints `2025.09.26`.
- Refuses (exit 2, `fake yt-dlp: real network URLs are not allowed`) any host that does not end
  in `.example`. Also requires `--ignore-config`, `--paths`, and exactly one URL after `--`.
- Honours `--paths <dir>` / `--paths temp:<dir>`, `--output`, `--output infojson:<name>`
  (written as `<name>.info.json`, like the real one), `--continue` / `--no-continue`,
  `--progress-template`.
- By URL path: `/watch` (any query) copies `tests/fixtures/import/download.webm` in 5 chunks with
  progress lines and writes the canned `info.json`; `/slow` waits 300 ms per chunk; `/no-total`
  prints `NA` totals; `/http403` fails with `HTTP Error 403: Forbidden`; `/offline` fails with an
  `ERROR:` line without a code; other paths: `ERROR: Unsupported URL`.
- With `--continue` it resumes from an existing `download.webm.part`.
- `$FAKE_YTDLP_LOG`: one JSON line per run, `{"argv": [...], "resume_from": N}`.

## `stub-separator` (bash)

The calliope-stems separator contract: `stub-separator <input.flac> <out_dir> <model>`. Copies
`tests/fixtures/import/stems/*.flac` into `<out_dir>` and prints `progress 0.25` ... `progress 1`.
Mode from `$STUB_SEPARATOR_MODE`, else the first line of a `stub-mode` file in the current
directory (or two levels up), else `ok`:

| mode | behaviour |
|---|---|
| `ok` | six stems, exit 0 |
| `fail` | message on stderr, exit 3 |
| `slow` | 2 s per progress step |
| `hang` | sleeps until killed |
| `bad-output` | writes `README.txt` instead of stems |
| `not-flac` | `vocals.flac` without the `fLaC` magic |
| `boundary` | vocals/drums/bass as `ok`; guitar 15.0 s audible, other 14.9 s, piano short loud bursts (9.5 s) |

`$STUB_SEPARATOR_LOG` gets one tab-separated argv line per run.

The real adapter `src/calliope-stems/separators/audio-separator.sh` is only syntax-checked
(`bash -n`); no test or agent ever runs it.
