#!/usr/bin/env bash
# Generates the editor test repository tests/fixtures/library-editor/ (invented content only).
# Idempotent: existing audio files are kept (FORCE=1 regenerates); track.json files are
# rewritten deterministically. Needs ffmpeg and the import fixtures (import/make-fixtures.sh).
# Usage: bash tests/fixtures/editor/make-fixtures.sh
set -euo pipefail

command -v ffmpeg >/dev/null || { echo "ffmpeg is required" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
imp="$here/../import"
repo="$here/../library-editor"
tracks="$repo/tracks"
mkdir -p "$tracks"

ff() {
  local out="${!#}"
  if [ -e "$out" ] && [ "${FORCE:-0}" != 1 ]; then return 0; fi
  ffmpeg -nostdin -hide_banner -loglevel error -y -fflags +bitexact -flags:a +bitexact -flags:v +bitexact "$@"
}

# stem NAME_PATH FREQ DURATION CHANNELS : 22050 Hz s16 FLAC sine
sine() {
  ff -f lavfi -i "sine=frequency=$2:sample_rate=22050:duration=$3" -ac "$4" -sample_fmt s16 -c:a flac "$1"
}

printf '{"schema_version": 1}\n' > "$repo/calliope-repository.json"

# track ID TYPE TITLE AUDIO(json) STEM_MODEL(json) STEMS(json lines) [EXTRA(json fragment)]
track() {
  local id="$1" type="$2" title="$3" audio="$4" model="$5" stems="$6" extra="${7:-}"
  cat > "$tracks/$id/track.json" <<JSON
{
  "schema_version": 2,
  "id": "$id",
  "type": "$type",
  "band": "The Example Band",
  "album": "Editor Tests",
  "title": "$title",
  "composers": [],
  "year": null,
  "source_url": null,
  "copyright": null,
  "audio": $audio,
  "original": null,
  "stems": $stems,
  "stem_model": $model,
  "tablatures": [],${extra:+
  $extra,}
  "imported": "2026-10-07T10:00:00Z",
  "modified": "2026-10-07T10:00:00Z"
}
JSON
}

p=0199c0a0-0000-7000-8000-000000000
H='"htdemucs_6s"'

# 201 Four Lanes
id=${p}201; mkdir -p "$tracks/$id/stems"
sine "$tracks/$id/stems/vocals.flac" 440 12 1
sine "$tracks/$id/stems/drums.flac" 220 12 1
sine "$tracks/$id/stems/bass.flac" 110 12 2
sine "$tracks/$id/stems/guitar.flac" 660 10 1
track $id stem "Four Lanes" null "$H" '[
    {"name": "vocals", "file": "stems/vocals.flac"},
    {"name": "drums", "file": "stems/drums.flac"},
    {"name": "bass", "file": "stems/bass.flac"},
    {"name": "guitar", "file": "stems/guitar.flac"}
  ]'

# 202 Mixed Before
id=${p}202; mkdir -p "$tracks/$id/stems" "$tracks/$id/backings"
sine "$tracks/$id/stems/vocals.flac" 440 6 1
sine "$tracks/$id/stems/guitar.flac" 660 6 1
sine "$tracks/$id/backings/backing.flac" 330 6 2
track $id stem "Mixed Before" null "$H" '[
    {"name": "vocals", "file": "stems/vocals.flac"},
    {"name": "guitar", "file": "stems/guitar.flac"}
  ]' '"backings": [{"id": "backing", "name": "Backing", "file": "backings/backing.flac", "created": "2026-10-07T10:00:00Z", "modified": "2026-10-07T10:00:00Z", "sample_rate": 22050, "bits": 16, "mix": {"stems": [{"name": "vocals", "gain_db": -6.0, "unmuted": true}, {"name": "guitar", "gain_db": 0.0, "unmuted": false}]}}]'

# 203 Plain Backing
id=${p}203; mkdir -p "$tracks/$id"
[ -e "$tracks/$id/backing.mp3" ] && [ "${FORCE:-0}" != 1 ] || cp "$imp/tagged.mp3" "$tracks/$id/backing.mp3"
track $id backing "Plain Backing" '"backing.mp3"' null '[]'

# 204 Six Lanes
id=${p}204; mkdir -p "$tracks/$id/stems"
for n in vocals drums bass guitar piano other; do
  [ -e "$tracks/$id/stems/$n.flac" ] && [ "${FORCE:-0}" != 1 ] || cp "$imp/stems/$n.flac" "$tracks/$id/stems/$n.flac"
done
track $id stem "Six Lanes" null "$H" '[
    {"name": "vocals", "file": "stems/vocals.flac"},
    {"name": "drums", "file": "stems/drums.flac"},
    {"name": "bass", "file": "stems/bass.flac"},
    {"name": "guitar", "file": "stems/guitar.flac"},
    {"name": "piano", "file": "stems/piano.flac"},
    {"name": "other", "file": "stems/other.flac"}
  ]'

# 205 Missing Stem (drums.flac deliberately absent)
id=${p}205; mkdir -p "$tracks/$id/stems"
[ -e "$tracks/$id/stems/vocals.flac" ] && [ "${FORCE:-0}" != 1 ] || cp "$imp/stems/vocals.flac" "$tracks/$id/stems/vocals.flac"
track $id stem "Missing Stem" null "$H" '[
    {"name": "vocals", "file": "stems/vocals.flac"},
    {"name": "drums", "file": "stems/drums.flac"}
  ]'

echo "fixtures written to $repo"
