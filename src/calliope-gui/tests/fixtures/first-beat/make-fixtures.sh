#!/usr/bin/env bash
# Generates the first-beat test fixture tests/fixtures/first-beat/noisy-intro.flac (invented content).
# Idempotent: an existing file is kept (FORCE=1 regenerates). Needs ffmpeg.
# Usage: bash tests/fixtures/first-beat/make-fixtures.sh
set -euo pipefail

command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ff() {
  local out="${!#}"
  if [ -e "$out" ] && [ "${FORCE:-0}" != 1 ]; then return 0; fi
  ffmpeg -nostdin -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact -flags:v +bitexact "$@"
}

# Generates noisy-intro.flac
ff -f lavfi -i "anoisesrc=color=white:amplitude=0.005:seed=1:sample_rate=44100" -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=4" \
  -filter_complex "[0:a]atrim=duration=1.2,asetpts=N/SR/TB,aformat=channel_layouts=stereo[n];[1:a]volume=4,aformat=channel_layouts=stereo[t];[n][t]concat=n=2:v=0:a=1" \
  -ar 44100 -sample_fmt s16 -c:a flac "$here/noisy-intro.flac"
