#!/usr/bin/env bash
# Bit-for-bit comparison of two builds or shader sets (the Viltrumite plan's step 3 proved its shader move with it):
#   bash pixel_scenes.sh OUTDIR SCENE...   (PORT_BIN picks the exe; SE_MODS=0 hides the Mods button)
# Make each side a folder with the exe and assets/shaders from freeze_clocks.py (every clock in the shaders stopped),
# run it, and compare with pixel_cmp.py A B. `pause` comes first, then `timing frame`, then the date: with those
# the forest and a street come out bit-identical between two runs of one exe, apart from a few HUD digits. Measure the
# noise first: run one side twice. Earth from afar, Saturn and the space view keep small differences of their own.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"
OUT="${1:?outdir}"; shift; mkdir -p "$OUT"
T="pause; timing frame"
D="$T; date 2026-06-21 12:00:00"
declare -A S
R="goto Earth 20000; wait 200; rs RS 8514-1828-5-27938-309; wait 300; select RS 8514-1828-5-27938-309 2"
S[space]="$D; goto Earth 20000; wait 600"
S[rsplanet]="$D; $R; surface 20 30 0.3 -10 0; wait 2400"
S[rsground]="$D; $R; surface -35 120 0.002 -5 0; wait 2400"
S[stars]="$D; goto Earth 20000; wait 100; look Sagittarius A*; wait 60; fly 1e15; wait 600; fly 0; wait 600"
S[forest]="$D; select Earth; surface 47.95 8.05 0.3 -5 90; agl 1.7; wait 2400"
S[forest300]="$D; select Earth; surface 47.95 8.05 0.3 -15 90; agl 300; wait 2400"
S[town_day]="$T; date 2026-06-21 16:00:00; select Earth; surface 40.7498 -73.9796 0.3 -4 40; agl 4; wait 2400"
S[town_night]="$T; date 2026-06-22 03:00:00; select Earth; surface 40.7498 -73.9796 0.3 -4 40; agl 4; wait 2400"
S[earth]="$D; select Earth; surface 45 90 0.3 -10 0; wait 2400"
S[mars]="$D; select Mars; surface -14.6 175.5 0.3 -10 0; wait 2400"
S[saturn]="$D; goto Saturn 300000; wait 900"
S[jupiter]="$D; select Jupiter; surface 10 180 100 -5 0; wait 1500"
S[jupiter_deep]="$D; select Jupiter; surface 10 180 -40 -5 0; wait 1500"
for s in "$@"; do
  bash "$REPO/.audit/tools/runport.sh" "$OUT/$s.png" 5 "${S[$s]}" > /dev/null
  echo "$s panics $(grep -ciE 'panic' "$OUT/$s.png.log") saved $(grep -c 'Screenshot saved' "$OUT/$s.png.log")"
done
