#!/usr/bin/env bash
# One benchmark scene: bash .optimize/bench/scene.sh NAME "dev; commands" [OUTDIR]
# Runs the release port uncapped (SE_NO_VSYNC) with the frame-time logger (SE_PERF, perf.rs).
# The commands mark segments with `perf SEG` ... `perf end`. Writes OUTDIR/NAME.perf (one line
# per segment), OUTDIR/NAME.png (final frame), OUTDIR/NAME.mem (peak working set, MB).
REPO="/c/Users/liamw/Desktop/Space Engine Recomp"
NAME="$1"; CMDS="$2"; OUT="${3:-$REPO/.optimize/scratch}"
mkdir -p "$OUT"; rm -f "$OUT/$NAME.perf"
( peak=0; while sleep 2; do
    v=$(powershell.exe -NoProfile -Command "(Get-Process spaceengine-rs -ErrorAction SilentlyContinue | Select-Object -First 1).PeakWorkingSet64" 2>/dev/null | tr -d '\r')
    [ -n "$v" ] && [ "$v" -gt "$peak" ] 2>/dev/null && peak=$v && echo $((peak / 1048576)) > "$OUT/$NAME.mem"
  done ) &
SAMPLER=$!
SE_PERF="$OUT/$NAME.perf" SE_NO_VSYNC=1 bash "$REPO/.audit/tools/runport.sh" "$OUT/$NAME.png" 5 "$CMDS" > /dev/null
kill $SAMPLER 2>/dev/null
echo "== $NAME (peak $(cat "$OUT/$NAME.mem" 2>/dev/null) MB)"
cut -f1-7 "$OUT/$NAME.perf" 2>/dev/null
grep -iE "panic|error" "$OUT/$NAME.png.log" | grep -v wgpu_hal | head -3
