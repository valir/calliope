#!/usr/bin/env bash
# Separator adapter for the real model (Demucs htdemucs_6s via audio-separator, on the GPU).
#
# Contract of the calliope-stems server (plan section 2.8):
#   audio-separator.sh <input.flac> <out_dir> <model>
# It must write <out_dir>/<stem>.flac (vocals, drums, bass, guitar, piano, other) and exit 0.
# stdout may carry `progress <0..1>` lines (this adapter prints none); everything else is logged.
#
# Environment:
#   STEMS_HOME  folder with .venv/bin/audio-separator and models/  (default ~/edge-ai/stems)
#   OLLAMA      Ollama base URL, to free VRAM first          (default http://127.0.0.1:11434)
#
# Based on the separation part of ~/edge-ai/stems/backing (without the mixing).
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: audio-separator.sh <input.flac> <out_dir> <model>" >&2
  exit 64
fi
in=$1 out=$2 model=$3

STEMS_HOME=${STEMS_HOME:-$HOME/edge-ai/stems}
OLLAMA=${OLLAMA:-http://127.0.0.1:11434}
NEED_VRAM_MIB=2500 # Demucs peaks ~1.6 GiB; unload Ollama models only if less is free

# The model name ends up in a file name: allow only plain names.
if [[ ! $model =~ ^[A-Za-z0-9_.-]+$ || $model == .* ]]; then
  echo "audio-separator.sh: invalid model name" >&2
  exit 64
fi
[[ -f $in ]] || { echo "audio-separator.sh: input not found: $in" >&2; exit 2; }
[[ -x $STEMS_HOME/.venv/bin/audio-separator ]] || {
  echo "audio-separator.sh: $STEMS_HOME/.venv/bin/audio-separator not found (set STEMS_HOME)" >&2
  exit 2
}
mkdir -p -- "$out"

free_vram() { nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | head -1; }

make_room_on_gpu() {
  local free
  free=$(free_vram 2>/dev/null) || return 0 # no nvidia-smi: nothing to free
  [[ $free =~ ^[0-9]+$ ]] || return 0
  (( free >= NEED_VRAM_MIB )) && return 0
  echo "only ${free} MiB VRAM free; asking Ollama to unload its models"
  curl -fs "$OLLAMA/api/ps" |
    python3 -c 'import json,sys; [print(m["name"]) for m in json.load(sys.stdin)["models"]]' |
    while read -r m; do
      curl -fs "$OLLAMA/api/generate" -d "{\"model\":\"$m\",\"keep_alive\":0}" >/dev/null && echo "unloaded $m"
    done || true
  sleep 2
  echo "$(free_vram) MiB VRAM free"
}

make_room_on_gpu

"$STEMS_HOME/.venv/bin/audio-separator" "$in" \
  -m "$model.yaml" \
  --model_file_dir "$STEMS_HOME/models" \
  --output_dir "$out" \
  --output_format FLAC \
  --custom_output_names '{"Vocals":"vocals","Drums":"drums","Bass":"bass","Guitar":"guitar","Piano":"piano","Other":"other"}' \
  --log_level warning
