#!/usr/bin/env bash
# Generates the import test fixtures (invented content only) with ffmpeg lavfi sources.
# Idempotent: re-running keeps existing files (FORCE=1 regenerates). Output is kept small.
# Usage: bash tests/fixtures/import/make-fixtures.sh
set -euo pipefail

command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"
mkdir -p stems

# Existing files are kept (Ogg/WebM muxing is not deterministic, so rewriting would only churn
# the committed binaries). FORCE=1 regenerates everything.
ff() {
  local out="${!#}"
  if [ -e "$out" ] && [ "${FORCE:-0}" != 1 ]; then return 0; fi
  ffmpeg -nostdin -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact -flags:v +bitexact "$@"
}

# 5 s stereo-ish sine sources
sine5="sine=frequency=440:duration=5:sample_rate=22050"

ff -f lavfi -i "$sine5" -ac 1 -c:a libmp3lame -b:a 32k -write_xing 0 -id3v2_version 3 \
  -metadata title="Glass Harbour" -metadata artist="The Example Band" \
  -metadata album="Test Pressings" -metadata date="2021-03-04" \
  -metadata composer="Ann Example; Bo Sample" \
  -metadata copyright="(c) 2021 The Example Band" tagged.mp3

ff -f lavfi -i "sine=frequency=330:duration=5:sample_rate=8000" -ac 1 -c:a flac untagged.flac

ff -f lavfi -i "sine=frequency=550:duration=5:sample_rate=22050" -ac 1 -c:a libvorbis -q:a -1 \
  -metadata title="Glass Harbour" -metadata artist="The Example Band" \
  -metadata album="Test Pressings" -metadata date="2021-03-04" \
  -metadata composer="Ann Example; Bo Sample" \
  -metadata copyright="(c) 2021 The Example Band" tagged.ogg

ff -f lavfi -i "testsrc=size=160x120:rate=10:duration=2" \
   -f lavfi -i "sine=frequency=660:duration=2:sample_rate=22050" \
   -c:v libx264 -preset ultrafast -crf 40 -pix_fmt yuv420p -c:a aac -b:a 24k -ac 1 \
   -metadata title="Clip Title" -shortest with-audio.mp4

ff -f lavfi -i "testsrc=size=160x120:rate=10:duration=2" \
   -c:v libx264 -preset ultrafast -crf 40 -pix_fmt yuv420p -an no-audio.mp4

printf 'This is not audio. It is a plain text file with an mp3 name.\n' > not-audio.mp3

# input for the fake yt-dlp (Opus in WebM)
ff -f lavfi -i "sine=frequency=262:duration=5:sample_rate=48000" -ac 1 -c:a libopus -b:a 16k download.webm

cat > info.json <<'JSON'
{
  "id": "abc123",
  "title": "The Example Band - Night Drive",
  "webpage_url": "https://media.example/watch?v=abc123",
  "upload_date": "20200115",
  "duration": 5,
  "extractor": "generic"
}
JSON

# six 1 s mono 8 kHz stems, different tones
i=0
for name in vocals drums bass guitar piano other; do
  f=$((220 + 110 * i)); i=$((i + 1))
  ff -f lavfi -i "sine=frequency=$f:duration=1:sample_rate=8000" -ac 1 -c:a flac "stems/$name.flac"
done

# 16 min of silence, mono 8 kHz (FLAC compresses silence to a few KB)
ff -f lavfi -i "anullsrc=r=8000:cl=mono" -t 960 -c:a flac long.flac

# an Ogg file with a .flac name
[ -e not-flac.flac ] && [ "${FORCE:-0}" != 1 ] || cp tagged.ogg not-flac.flac

echo "fixtures written to $here"
