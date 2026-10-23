#!/usr/bin/env bash
# Runs the GUI e2e tests (the #[ignore]d tests of the binaries below, built with --features
# e2e-hooks).
#
#   scripts/test-gui.sh            parallel: every test binary on its own private X display
#                                  (Xvfb if installed, else Xorg with the dummy driver) with its
#                                  own i3; tests inside a binary stay serial (--test-threads=1)
#   scripts/test-gui.sh --serial   one binary after the other on the display in $DISPLAY
#                                  (e.g. DISPLAY=:1 npm run test:gui:serial)
#
# Per-binary output goes to target/gui-logs/<binary>.log. Exit status is non-zero if any binary
# failed. Extra arguments after `--` are passed to every test binary (e.g. a test name filter).
# Never touches the display in $DISPLAY in parallel mode; private displays are numbered from 90.
set -uo pipefail

cd "$(dirname "$0")/.."
GUI_DIR=$PWD
REPO=$(cd ../.. && pwd)
LOGS=$REPO/target/gui-logs
BINARIES=(gui_smoke gui_e2e gui_library_e2e gui_import_e2e gui_export_e2e gui_editor_e2e gui_player_e2e gui_player_acceptance_e2e acceptance_stem_extraction_gui)

serial=0
if [[ ${1:-} == --serial ]]; then serial=1; shift; fi
[[ ${1:-} == -- ]] && shift
extra=("$@")

if ((serial)) && [[ -z ${DISPLAY:-} ]]; then
  echo "test-gui.sh --serial needs DISPLAY (e.g. DISPLAY=:1)" >&2
  exit 2
fi

echo "== build"
npm run build >/dev/null || exit 1
cargo build -p calliope-stems || exit 1
test_args=()
for b in "${BINARIES[@]}"; do test_args+=(--test "$b"); done
exes=$(cargo test --no-run --features e2e-hooks "${test_args[@]}" --message-format=json |
  python3 -I -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["test"]:
        print(m["target"]["name"], m["executable"])
') || exit 1
declare -A EXE
while read -r name path; do EXE[$name]=$path; done <<<"$exes"
for b in "${BINARIES[@]}"; do
  [[ -x ${EXE[$b]:-} ]] || { echo "no test binary for $b" >&2; exit 1; }
done

mkdir -p "$LOGS"
WORK=$(mktemp -d "$REPO/target/gui-logs/run.XXXXXX")
pids=()
cleanup() {
  trap - EXIT INT TERM
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
  for p in "${pids[@]}"; do wait "$p" 2>/dev/null; done
  rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# --- private displays ---------------------------------------------------------------------
cat >"$WORK/i3.conf" <<'EOF'
font pango:monospace 8
default_border none
default_floating_border none
EOF
cat >"$WORK/dummy.conf" <<'EOF'
Section "Device"
  Identifier "d"
  Driver "dummy"
  VideoRam 256000
EndSection
Section "Monitor"
  Identifier "m"
  HorizSync 5-1000
  VertRefresh 5-200
  Modeline "1920x1200" 193.25 1920 2056 2256 2592 1200 1203 1209 1245 -hsync +vsync
EndSection
Section "Screen"
  Identifier "s"
  Device "d"
  Monitor "m"
  DefaultDepth 24
  SubSection "Display"
    Depth 24
    Modes "1920x1200"
    Virtual 1920 1200
  EndSubSection
EndSection
Section "ServerLayout"
  Identifier "l"
  Screen "s"
  Option "AutoAddDevices" "false"
  Option "AutoEnableDevices" "false"
EndSection
EOF

next_display=90
# Starts X + i3 on a free display; sets DISPLAY_NUM. Pids are registered for cleanup.
start_display() {
  while [[ -e /tmp/.X11-unix/X$next_display || -e /tmp/.X$next_display-lock ]]; do next_display=$((next_display + 1)); done
  local n=$next_display
  next_display=$((n + 1))
  if command -v Xvfb >/dev/null; then
    Xvfb ":$n" -screen 0 1920x1200x24 -nolisten tcp >"$WORK/x$n.log" 2>&1 &
  elif [[ -x /usr/lib/Xorg ]]; then
    /usr/lib/Xorg ":$n" -config "$WORK/dummy.conf" -noreset -nolisten tcp -novtswitch -sharevts \
      -logfile "$WORK/xorg$n.log" >"$WORK/x$n.log" 2>&1 &
  else
    echo "no Xvfb (xorg-server-xvfb) and no /usr/lib/Xorg + xf86-video-dummy found" >&2
    return 1
  fi
  pids+=($!)
  local i
  for ((i = 0; i < 100; i++)); do
    DISPLAY=:$n xdotool getdisplaygeometry >/dev/null 2>&1 && break
    sleep 0.2
  done
  DISPLAY=:$n xdotool getdisplaygeometry >/dev/null 2>&1 || { echo "X server on :$n did not start" >&2; return 1; }
  env -u WAYLAND_DISPLAY DISPLAY=:$n i3 -c "$WORK/i3.conf" >"$WORK/i3-$n.log" 2>&1 &
  pids+=($!)
  for ((i = 0; i < 100; i++)); do
    DISPLAY=:$n i3-msg -t get_version >/dev/null 2>&1 && break
    sleep 0.2
  done
  DISPLAY_NUM=$n
}

# Runs one binary with the given display; the exit status is written to $WORK/<b>.status.
run_binary() {
  local b=$1 display=$2 t0 t1 rc
  t0=$(date +%s.%N)
  mkdir -p "$REPO/target/gui-e2e/_home-$b"
  (
    cd "$GUI_DIR"
    env -u WAYLAND_DISPLAY -u XAUTHORITY DISPLAY="$display" GDK_BACKEND=x11 HOME="$REPO/target/gui-e2e/_home-$b" \
      "${EXE[$b]}" --ignored --test-threads=1 "${extra[@]}"
  ) >"$LOGS/$b.log" 2>&1
  rc=$?
  t1=$(date +%s.%N)
  printf '%s %s\n' "$rc" "$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.0f", b-a}')" >"$WORK/$b.status"
}

start_all=$(date +%s)
echo "== run"
if ((serial)); then
  for b in "${BINARIES[@]}"; do
    echo "-- $b on $DISPLAY"
    run_binary "$b" "$DISPLAY" &
    pids+=($!)
    wait $!
  done
else
  declare -A WAITPID
  for b in "${BINARIES[@]}"; do
    start_display || exit 1
    echo "-- $b on :$DISPLAY_NUM"
    run_binary "$b" ":$DISPLAY_NUM" &
    WAITPID[$b]=$!
    pids+=($!)
  done
  for b in "${BINARIES[@]}"; do wait "${WAITPID[$b]}"; done
fi

# --- summary ------------------------------------------------------------------------------
echo "== summary"
failed=0
for b in "${BINARIES[@]}"; do
  read -r rc secs <"$WORK/$b.status" 2>/dev/null || { rc=1; secs=?; }
  res=$(grep -E '^test result:' "$LOGS/$b.log" | tail -1)
  if [[ $rc == 0 ]]; then verdict=ok; else verdict=FAILED; failed=1; fi
  printf '%-30s %-7s %5ss  %s\n' "$b" "$verdict" "$secs" "$res"
  if [[ $rc != 0 ]]; then grep -E '^test .* FAILED|panicked' "$LOGS/$b.log" | head -10 | sed 's/^/    /'; echo "    log: $LOGS/$b.log"; fi
done
echo "total wall time: $(($(date +%s) - start_all))s"
exit $failed
