#!/usr/bin/env bash
# Reference screenshots of fixed scenes, time paused, for the "identical output" check:
# bash .optimize/bench/refs.sh OUTDIR [SCENE...]; compare two folders with compare.py.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"
OUT="${1:?outdir}"; shift; mkdir -p "$OUT"
D="date 2026-06-21 12:00:00; pause"
R="goto Earth 20000; wait 200; rs RS 8514-1828-5-27938-309; wait 300; select RS 8514-1828-5-27938-309 2"
declare -A S
S[space]="$D; goto Earth 20000; wait 600"
S[galaxy]="$D; galactic 30000; wait 900"
S[stars]="$D; goto Earth 20000; wait 100; look Sagittarius A*; wait 60; fly 1e15; wait 600; fly 0; wait 600"
S[earth]="$D; select Earth; surface 45 90 0.3 -10 0; wait 1200"
S[mars]="$D; select Mars; surface -14.6 175.5 0.3 -10 0; wait 1200"
S[rsplanet]="$D; $R; surface 20 30 0.3 -10 0; wait 1200"
S[rsground]="$D; $R; surface -35 120 0.002 -5 0; wait 1200"
S[nebula]="$D; select Skylias Stellar Nursery; go 150000000000000; wait 900"
S[tarantula]="$D; select Tarantula Nebula; go 900000000000000; wait 900"
S[saturn]="$D; goto Saturn 300000; wait 900"
ORDER="${*:-space galaxy stars earth mars rsplanet rsground nebula tarantula saturn}"
for s in $ORDER; do
  bash "$REPO/.audit/tools/runport.sh" "$OUT/$s.png" 5 "${S[$s]}" > /dev/null
  echo "$s $(grep -ciE "panic" "$OUT/$s.png.log") panics"
done
