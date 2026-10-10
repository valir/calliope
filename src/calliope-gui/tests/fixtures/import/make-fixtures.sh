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

# six 16 s mono 8 kHz stems, different tones (16 s audible each: kept by the 15 s empty-stem rule)
i=0
for name in vocals drums bass guitar piano other; do
  f=$((220 + 110 * i)); i=$((i + 1))
  ff -f lavfi -i "sine=frequency=$f:duration=16:sample_rate=8000" -ac 1 -c:a flac "stems/$name.flac"
done

# quiet stems for the empty-stem check: digital silence, about -60 dBFS, about -45 dBFS
mkdir -p stems-quiet
ff -f lavfi -i "anullsrc=r=8000:cl=mono" -t 1 -sample_fmt s16 -c:a flac stems-quiet/silent.flac
ff -f lavfi -i "aevalsrc=0.001*sin(2*PI*440*t):s=8000:d=1" -ac 1 -sample_fmt s16 -c:a flac stems-quiet/minus60.flac
ff -f lavfi -i "aevalsrc=0.005623*sin(2*PI*440*t):s=8000:d=1" -ac 1 -sample_fmt s16 -c:a flac stems-quiet/minus45.flac

# stems for the audible-time rule (100 ms windows above -40 dBFS, 15 s minimum); the tone
# is gated on whole 100 ms windows so rounding by one sample cannot change a count
mkdir -p stems-activity
gen() { ff -f lavfi -i "aevalsrc='$1':s=8000:d=$2" -ac 1 -sample_fmt s16 -c:a flac "$3"; }
# 9 s burst at -17 dBFS + five one-sample clicks at -8 dBFS: 9.5 s audible, empty
gen "0.14*sin(2*PI*700*t)*between(floor(t*10),50,139)+0.4*(eq(floor(t*8000),160400)+eq(floor(t*8000),168400)+eq(floor(t*8000),176400)+eq(floor(t*8000),184400)+eq(floor(t*8000),192400))" 30 stems-activity/bursts.flac
# eight 2 s phrases with 2 s pauses: 16.0 s audible in pieces, kept
gen "0.1*sin(2*PI*500*t)*lt(mod(floor(t*10),40),20)" 32 stems-activity/phrases.flac
# boundary: 5.0 + 5.0 + 4.9 s = 14.9 s (empty) and 5.0 + 5.0 + 5.0 s = 15.0 s (kept)
gen "0.1*sin(2*PI*500*t)*(between(floor(t*10),10,59)+between(floor(t*10),70,119)+between(floor(t*10),130,178))" 20 stems-activity/audible-14900ms.flac
gen "0.1*sin(2*PI*500*t)*(between(floor(t*10),10,59)+between(floor(t*10),70,119)+between(floor(t*10),130,179))" 20 stems-activity/audible-15000ms.flac

# 16 min of silence, mono 8 kHz (FLAC compresses silence to a few KB)
ff -f lavfi -i "anullsrc=r=8000:cl=mono" -t 960 -c:a flac long.flac

# an Ogg file with a .flac name
[ -e not-flac.flac ] && [ "${FORCE:-0}" != 1 ] || cp tagged.ogg not-flac.flac

echo "fixtures written to $here"
