#!/usr/bin/env bash
# Generates the Player test repository tests/fixtures/library-player/ (invented content only).
# Idempotent: existing audio files are kept (FORCE=1 regenerates); track.json and player.json
# files are rewritten deterministically. Needs ffmpeg.
# Usage: bash tests/fixtures/player/make-fixtures.sh
set -euo pipefail

command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$here/../library-player"
tracks="$repo/tracks"
mkdir -p "$tracks"

ff() {
  local out="${!#}"
  if [ -e "$out" ] && [ "${FORCE:-0}" != 1 ]; then return 0; fi
  ffmpeg -nostdin -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact -flags:v +bitexact "$@"
}

# sine PATH FREQ DURATION CHANNELS : 22050 Hz s16 FLAC sine
sine() {
  ff -f lavfi -i "sine=frequency=$2:sample_rate=22050:duration=$3" -ac "$4" -sample_fmt s16 -c:a flac "$1"
}

# envelope PATH FREQ : 20 s stereo sine (full scale: the sine filter peaks at 1/8, hence the 8), amplitude 0.2 / 0.9 / 0.5 / 0.9 per 5 s
envelope() {
  ff -f lavfi -i "sine=frequency=$2:sample_rate=22050:duration=20,volume='8*if(lt(t,5),0.2,if(lt(t,10),0.9,if(lt(t,15),0.5,0.9)))':eval=frame,pan=stereo|c0=c0|c1=c0" \
    -sample_fmt s16 -c:a flac "$1"
}

printf '{"schema_version": 1}\n' > "$repo/calliope-repository.json"

# track ID TYPE TITLE AUDIO(json) ORIGINAL(json) STEMS(json) BACKINGS(json)
track() {
  local id="$1" type="$2" title="$3" audio="$4" original="$5" stems="$6" backings="$7"
  cat > "$tracks/$id/track.json" <<JSON
{
  "schema_version": 2,
  "id": "$id",
  "type": "$type",
  "band": "The Example Band",
  "album": "Player Tests",
  "title": "$title",
  "composers": [],
  "year": null,
  "source_url": null,
  "copyright": null,
  "audio": $audio,
  "original": $original,
  "stems": $stems,
  "stem_model": $STEM_MODEL,
  "tablatures": [],
  "backings": $backings,
  "imported": "2026-10-07T10:00:00Z",
  "modified": "2026-10-07T10:00:00Z"
}
JSON
}

p=0199c0a0-0000-7000-8000-000000000
H='"htdemucs_6s"'
STEMS_VG='[
    {"name": "vocals", "file": "stems/vocals.flac"},
    {"name": "guitar", "file": "stems/guitar.flac"}
  ]'
BACKING='[{"id": "backing", "name": "Backing", "file": "backings/backing.flac", "created": "2026-10-07T10:00:00Z", "modified": "2026-10-07T10:00:00Z", "sample_rate": 22050, "bits": 16, "mix": {"stems": [{"name": "vocals", "gain_db": -6.0, "unmuted": true}, {"name": "guitar", "gain_db": 0.0, "unmuted": false}]}}]'

# practice_audio ID : stems vocals+guitar, enveloped backing (used by 301 and 304)
practice_audio() {
  local d="$tracks/$1"
  mkdir -p "$d/stems" "$d/backings"
  sine "$d/stems/vocals.flac" 440 20 1
  sine "$d/stems/guitar.flac" 660 20 1
  envelope "$d/backings/backing.flac" 330
}

# 301 Practice Song
id=${p}301; practice_audio $id
sine "$tracks/$id/original.flac" 550 20 2
STEM_MODEL="$H"
track $id stem "Practice Song" null '"original.flac"' "$STEMS_VG" "$BACKING"

# 302 Stems Only
id=${p}302; mkdir -p "$tracks/$id/stems"
STEM_MODEL="$H"
sine "$tracks/$id/stems/bass.flac" 110 20 1
sine "$tracks/$id/stems/drums.flac" 220 20 1
track $id stem "Stems Only" null null '[
    {"name": "bass", "file": "stems/bass.flac"},
    {"name": "drums", "file": "stems/drums.flac"}
  ]' '[]'

# 303 MP3 Backing
id=${p}303; mkdir -p "$tracks/$id"
ff -f lavfi -i "sine=frequency=330:sample_rate=44100:duration=20" -ac 2 -c:a libmp3lame -b:a 128k "$tracks/$id/backing.mp3"
STEM_MODEL=null
track $id backing "MP3 Backing" '"backing.mp3"' null '[]' '[]'

# 304 With Tempo
id=${p}304; practice_audio $id
STEM_MODEL="$H"
track $id stem "With Tempo" null null "$STEMS_VG" "$BACKING"
cat > "$tracks/$id/player.json" <<'JSON'
{"schema_version": 1, "source": "stem:guitar", "position_ms": 4000, "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": 500.0, "bpm": 120.0}, {"start_ms": 10500.0, "bpm": 90.0}]}, "speed": 1.0, "metronome": {"on": false, "count_in": false}, "loops": [{"id": "loop-1", "start": {"tick": 64}, "end": {"tick": 192}, "repeat": true, "count_in": false}, {"id": "loop-2", "start": {"tick": 384}, "end": {"tick": 448}, "repeat": false, "count_in": true}, {"id": "loop-3", "start": {"tick": 512}, "end": null, "repeat": true, "count_in": false}], "current_loop": "loop-1"}
JSON

# 305 Missing Backing (backings/backing.flac deliberately absent)
id=${p}305; mkdir -p "$tracks/$id/stems"
sine "$tracks/$id/stems/vocals.flac" 440 20 1
STEM_MODEL="$H"
track $id stem "Missing Backing" null null '[
    {"name": "vocals", "file": "stems/vocals.flac"}
  ]' "$BACKING"

# 306 Missing Audio (gone.flac deliberately absent)
id=${p}306; mkdir -p "$tracks/$id"
STEM_MODEL=null
track $id backing "Missing Audio" '"gone.flac"' null '[]' '[]'

echo "fixtures written to $repo"
