#!/usr/bin/env bash
# Beat tracker adapter for the real model (Beat This! by CPJKU, checkpoint final0, CUDA with CPU fallback).
#
# Contract of the calliope-stems server (README, "The beat tracker contract"):
#   beat-this.sh <input.flac> <out_dir>
# It writes <out_dir>/beats.json ({"beats":[s..],"downbeats":[s..],"tracker":"..."}) and exits 0.
# stdout carries `progress <0..1>` lines; everything else goes to stderr.
#
# Environment:
#   BEATS_HOME            folder with .venv/bin/python and models/final0.ckpt  (default ~/edge-ai/beats)
#   BEAT_THIS_CHECKPOINT  checkpoint file                                      (default $BEATS_HOME/models/final0.ckpt)
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: beat-this.sh <input.flac> <out_dir>" >&2
  exit 64
fi
in=$1 out=$2

BEATS_HOME=${BEATS_HOME:-$HOME/edge-ai/beats}
PY=$BEATS_HOME/.venv/bin/python
CKPT=${BEAT_THIS_CHECKPOINT:-$BEATS_HOME/models/final0.ckpt}

[[ -f $in ]] || { echo "beat-this.sh: input not found: $in" >&2; exit 2; }
[[ -x $PY ]] || { echo "beat-this.sh: $PY not found (set BEATS_HOME)" >&2; exit 2; }
[[ -f $CKPT ]] || { echo "beat-this.sh: checkpoint not found: $CKPT (set BEATS_HOME or BEAT_THIS_CHECKPOINT)" >&2; exit 2; }
mkdir -p -- "$out"

exec "$PY" - "$in" "$out" "$CKPT" <<'PY'
import json
import os
import sys
from importlib.metadata import version

import numpy as np
import soundfile as sf
import torch
from beat_this.inference import Audio2Beats

src, out_dir, ckpt = sys.argv[1:4]

signal, sr = sf.read(src, dtype="float32", always_2d=True)
signal = signal.mean(axis=1)
print("progress 0.1", flush=True)


def run(device):
    return Audio2Beats(checkpoint_path=ckpt, device=device, dbn=False)(signal, sr)


device = "cuda" if torch.cuda.is_available() else "cpu"
try:
    beats, downbeats = run(device)
except torch.cuda.OutOfMemoryError:
    print("beat-this.sh: CUDA out of memory, falling back to the CPU", file=sys.stderr, flush=True)
    torch.cuda.empty_cache()
    device = "cpu"
    beats, downbeats = run(device)
print("progress 0.9", flush=True)

# Round to microseconds, keep strictly increasing non-negative beats, and only downbeats that are beats.
beat_list = []
for t in np.asarray(beats, dtype=float).tolist():
    t = round(t, 6)
    if t >= 0 and (not beat_list or t > beat_list[-1]):
        beat_list.append(t)
beat_set = set(beat_list)
down_list = []
for t in np.asarray(downbeats, dtype=float).tolist():
    t = round(t, 6)
    if t in beat_set and (not down_list or t > down_list[-1]):
        down_list.append(t)

doc = {
    "beats": beat_list,
    "downbeats": down_list,
    "tracker": f"beat_this {version('beat-this')} final0 {device}",
}
final = os.path.join(out_dir, "beats.json")
tmp = final + ".tmp"
with open(tmp, "w") as f:
    json.dump(doc, f)
os.replace(tmp, final)
print("progress 1", flush=True)
PY
