#!/usr/bin/env bash
# Craters kept and freed (spec step 8, D3): a crater, then away 3 km and back (the ground's nodes change), 10 km and back (kept), 21 km
# away (freed at 20.5 km, logged), back (gone); then one more, the mod off (kept, drawn), and the camera 25 km away (freed).
#   bash .optimize/bench/hero_keep.sh NAME "date ...; select Earth; surface LAT LON 0.3 -6" [extra run args]
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
N="$1"; P="$2"; shift 2
go() { echo "hero keys space 0.05; wait 10; hero cruise $1 60; wait $2; hero cruise off; hero keys z 0.05; wait 30; hero impacts"; }
back() { echo "hero aim 180 0; wait 3; $(go $1 $2); hero aim 0 -89; wait 3"; }
C="timing frame; $P; agl 300; wait 1500; agl off; mods on viltrumite; wait 300; hero keys x 0.1; wait 800; hero crater 15; wait 600"
C="$C; hero dist 40; hero aim 0 -30; wait 3; shot @/${N}_0.png; hero impacts; hero aim 0 0; wait 3"
C="$C; $(go 300 600); $(back 300 600); hero dist 160; wait 600; shot @/${N}_3km.png; hero aim 0 0; wait 3"
C="$C; $(go 1000 600); $(back 1000 600); hero dist 160; wait 600; shot @/${N}_10km.png; hero aim 0 0; wait 3"
C="$C; $(go 1000 1300); $(back 1000 1300); hero dist 160; wait 600; shot @/${N}_21km.png"
C="$C; hero crater 12; wait 300; hero dist 60; hero aim 0 -60; wait 3; shot @/${N}_on.png; mods off; wait 120; shot @/${N}_off.png; hero impacts; surface 0 0 25 -30; wait 60; hero impacts; wait 3"
bash .optimize/bench/hero_impact.sh "$N" "$C" "$@"
grep -E "freed|aftermath" ".optimize/scratch/run/s8/$N.png.log" | sed -E 's/^.*(viltrumite: crater|hero impacts: [0-9])/\1/' | cut -c1-200
