#!/usr/bin/env bash
# Generates the Export test repository tests/fixtures/library-export/ (invented content only).
# Idempotent: existing audio files are kept (FORCE=1 regenerates); track.json and player.json
# files are rewritten deterministically. Needs ffmpeg (with libmp3lame).
# Usage: bash tests/fixtures/export/make-fixtures.sh
set -euo pipefail

command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../library-export"
tracks="$repo/tracks"
mkdir -p "$tracks"

ff() {
  local out="${!#}"
  if [ -e "$out" ] && [ "${FORCE:-0}" != 1 ]; then return 0; fi
  ffmpeg -nostdin -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact -flags:v +bitexact "$@"
}

# sine PATH FREQ DURATION CHANNELS RATE : s16 FLAC sine
sine() {
  ff -f lavfi -i "sine=frequency=$2:sample_rate=$5:duration=$3" -ac "$4" -sample_fmt s16 -c:a flac "$1"
}

printf '{"schema_version": 1}\n' > "$repo/calliope-repository.json"

# track ID TYPE BAND ALBUM TITLE AUDIO(json) STEMS(json) BACKINGS(json) EXTRA(json fragment, may be empty)
track() {
  local id="$1" type="$2" band="$3" album="$4" title="$5" audio="$6" stems="$7" backings="$8" extra="${9:-}"
  cat > "$tracks/$id/track.json" <<JSON
{
  "schema_version": 2,
  "id": "$id",
  "type": "$type",
  "band": "$band",
  "album": "$album",
  "title": "$title",$extra
  "composers": [],
  "year": ${YEAR:-null},
  "source_url": null,
  "copyright": null,
  "audio": $audio,
  "original": null,
  "stems": $stems,
  "stem_model": $STEM_MODEL,
  "tablatures": [],
  "backings": $backings,
  "imported": "2026-10-23T10:00:00Z",
  "modified": "2026-10-23T10:00:00Z"
}
JSON
}

# tempo ID START_MS BPM : the track's player.json
tempo() {
  printf '{"schema_version": 6, "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": %s, "bpm": %s}]}}\n' "$2" "$3" > "$tracks/$1/player.json"
}

p=0199c0a0-0000-7000-8000-000000000
B="The Example Band"
A="Export Tests"
H='"htdemucs_6s"'
GUITAR='[{"name": "guitar", "file": "stems/guitar.flac"}]'
BACKING='[{"id": "backing", "name": "Backing", "file": "backings/backing.flac", "created": "2026-10-23T10:00:00Z", "modified": "2026-10-23T10:00:00Z", "sample_rate": 44100, "bits": 16, "mix": {"stems": [{"name": "guitar", "gain_db": 0.0, "unmuted": true}]}}]'
YEAR=null

# 401 Stem Mix: 330 Hz left / 550 Hz right
id=${p}401; mkdir -p "$tracks/$id/stems" "$tracks/$id/backings"
sine "$tracks/$id/stems/guitar.flac" 660 4 1 44100
ff -f lavfi -i "sine=frequency=330:sample_rate=44100:duration=4" -f lavfi -i "sine=frequency=550:sample_rate=44100:duration=4" \
  -filter_complex "[0:a][1:a]amerge=inputs=2" -sample_fmt s16 -c:a flac "$tracks/$id/backings/backing.flac"
STEM_MODEL="$H" YEAR=2021
track $id stem "$B" "$A" "Stem Mix" null "$GUITAR" "$BACKING" '
  "position": 3,
  "album_position": 1,'
YEAR=null
tempo $id 0 120

# 402 No Mix
id=${p}402; mkdir -p "$tracks/$id/stems"
sine "$tracks/$id/stems/guitar.flac" 660 4 1 44100
STEM_MODEL="$H"
track $id stem "$B" "$A" "No Mix" null "$GUITAR" '[]'
tempo $id 0 120

# 403 Plain MP3
id=${p}403; mkdir -p "$tracks/$id"
ff -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=4" -ac 2 -c:a libmp3lame -q:a 4 "$tracks/$id/backing.mp3"
STEM_MODEL=null
track $id backing "$B" "$A" "Plain MP3" '"backing.mp3"' '[]' '[]'
tempo $id 0 120

# 404 Mono Backing
id=${p}404; mkdir -p "$tracks/$id"
sine "$tracks/$id/backing.flac" 660 4 1 44100
STEM_MODEL=null
track $id backing "$B" "$A" "Mono Backing" '"backing.flac"' '[]' '[]'
tempo $id 0 120

# 405 High Rate
id=${p}405; mkdir -p "$tracks/$id"
sine "$tracks/$id/backing.flac" 1000 4 2 48000
STEM_MODEL=null
track $id backing "$B" "$A" "High Rate" '"backing.flac"' '[]' '[]'
tempo $id 0 100

# 406 With Tempo
id=${p}406; mkdir -p "$tracks/$id/stems" "$tracks/$id/backings"
sine "$tracks/$id/stems/guitar.flac" 660 4 1 44100
sine "$tracks/$id/backings/backing.flac" 440 4 2 44100
STEM_MODEL="$H"
track $id stem "$B" "$A" "With Tempo" null "$GUITAR" "$BACKING"
tempo $id 500 120

# 407 Long Intro: 3 s of digital silence, then 3 s of 440 Hz
id=${p}407; mkdir -p "$tracks/$id"
ff -f lavfi -i "anullsrc=r=44100:cl=stereo" -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=3" \
  -filter_complex "[0:a]atrim=duration=3,asetpts=N/SR/TB[s];[1:a]aformat=channel_layouts=stereo[t];[s][t]concat=n=2:v=0:a=1" \
  -ar 44100 -sample_fmt s16 -c:a flac "$tracks/$id/intro.flac"
STEM_MODEL=null
track $id backing "$B" "$A" "Long Intro" '"intro.flac"' '[]' '[]'
tempo $id 3000 100

# 408 Missing Mix (backings/backing.flac deliberately absent)
id=${p}408; mkdir -p "$tracks/$id/stems"
sine "$tracks/$id/stems/guitar.flac" 660 4 1 44100
STEM_MODEL="$H"
track $id stem "$B" "$A" "Missing Mix" null "$GUITAR" "$BACKING"
tempo $id 0 120

# 409 Live: A/B? (band AC/DC*, no album, position 5)
id=${p}409; mkdir -p "$tracks/$id"
sine "$tracks/$id/backing.flac" 440 1 2 44100
STEM_MODEL=null
track $id backing 'AC/DC*' '' 'Live: A/B?' '"backing.flac"' '[]' '[]' '
  "position": 5,'
tempo $id 0 120

# 410 No Tempo (no player.json)
id=${p}410; mkdir -p "$tracks/$id/stems" "$tracks/$id/backings"
sine "$tracks/$id/stems/guitar.flac" 660 4 1 44100
sine "$tracks/$id/backings/backing.flac" 440 4 2 44100
rm -f "$tracks/$id/player.json"
STEM_MODEL="$H"
track $id stem "$B" "$A" "No Tempo" null "$GUITAR" "$BACKING"

echo "fixtures written to $repo"
