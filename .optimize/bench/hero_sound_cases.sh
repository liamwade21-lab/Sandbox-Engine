#!/usr/bin/env bash
# The sound cases the review of step 6 was about: bash .optimize/bench/hero_sound_cases.sh OUTDIR_NAME CASE...
# moon: X over the Moon, then standing and walking there. orbit: coasting at 400 km over Earth. fallair: X with a sideways start at
# 60 km over Earth, slowed through 1,200 m/s by the air alone. hyperx: X out of Hyper flight. hyperslow: Shift let go in Hyper.
# reon: turned off in Hyper and on again (no leaving-Hyper sound). jupiter, star: Ctrl held against a stop with nothing to stand on, then flying away from it. goto: Shift+G from orbit into Earth's air.
# Prints what fired, and the levels at each settled speed.
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
OUT="$1"; shift
S=".optimize/scratch/run/$OUT"; mkdir -p "$S"
start() { echo "timing frame; date 2026-06-21 10:00:00; $1; wait ${2:-1200}; mods on viltrumite; wait 120"; }
for CASE in "$@"; do
  W="$(cygpath -m "$REPO/$S/$CASE.csv")"
  P="$(cygpath -m "$REPO/$S/$CASE")"
  T="hero sound trace $W"
  case "$CASE" in
    moon) C="$(start 'select Moon; surface 0 30 3 0'); $T; wait 30; hero sound; hero keys x 0; wait 300; hero sound; shot ${P}_fall.png; wait 300; hero sound; hero keys off; wait 4200; hero sound; hero state; shot ${P}_stand.png; hero keys w 3; wait 120; hero sound; shot ${P}_walk.png; hero keys off; wait 60; hero keys space 0; wait 120; hero sound; shot ${P}_jump.png; wait 30" ;;
    orbit) C="$(start 'select Earth; surface 0 0 400 0'); $T; wait 30; hero sound; hero orbit 1; wait 60; hero sound; wait 240; hero sound; shot ${P}_coast.png; hero keys space 0; wait 60; hero sound; hero keys x 0; wait 240; hero sound; wait 30" ;;
    fallair) C="$(start 'select Earth; surface 0 0 60 0'); $T; wait 30; hero orbit 0.25; wait 3600; hero sound; shot ${P}_fall.png; wait 30" ;;
    hyperx) C="$(start 'select Earth; surface 0 0 40 0'); $T; wait 30; hero aim 0 20; hero keys w+shift 4; wait 240; hero sound; hero keys x 0; wait 600; hero sound; wait 30" ;;
    hyperslow) C="$(start 'select Earth; surface 0 0 40 0'); $T; wait 30; hero aim 0 20; hero keys w+shift 4; wait 240; hero sound; hero keys off; wait 240; hero sound; shot ${P}_hover.png; wait 30" ;;
    jupiter) C="$(start 'select Jupiter; surface 10 40 20 0' 900); $T; wait 30; hero aim 0 -89; hero keys ctrl+shift 12; wait 720; hero sound; hero state; hero keys ctrl 6; wait 360; hero sound; hero keys ctrl+w 2; wait 120; hero sound; hero state; shot ${P}_floor.png; hero keys off; wait 60; hero keys w 3; wait 180; hero sound; hero state; shot ${P}_away.png; hero keys off; wait 60; hero keys space 3; wait 180; hero sound; hero state; wait 30" ;;
    star) C="$(start 'select Sun; surface 10 40 20 0' 600); $T; wait 30; hero aim 0 -89; hero keys ctrl+shift 9; wait 540; hero sound; hero state; hero keys ctrl 4; wait 240; hero sound; hero state; shot ${P}_stop.png; hero keys off; wait 60; hero keys space 3; wait 180; hero sound; hero state; wait 30" ;;
    goto) C="$(start 'select Earth; surface 0 0 400 0'); $T; wait 30; hero sound; hero keys shift+g 0; wait 60; hero sound; wait 60; hero sound; wait 60; hero sound; wait 60; hero sound; wait 60; hero sound; wait 60; hero sound; wait 60; hero sound; wait 60; hero sound; shot ${P}_end.png; wait 300; hero sound; wait 30" ;;
    reon) C="$(start 'select Earth; surface 0 0 40 0'); $T; wait 30; hero aim 0 20; hero keys w+shift 4; wait 200; hero sound; mods off; wait 30; hero sound; mods on viltrumite; wait 60; hero sound; shot ${P}_on.png; wait 30" ;;
    *) echo "no case $CASE"; continue ;;
  esac
  echo "== $CASE"
  PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$CASE.png" 5 "$C; hero sound trace off; wait 5" > /dev/null 2>&1
  grep -E "panicked|ERROR" "$S/$CASE.png.log" | cut -c1-200 | head -3
  grep -E "fired, gain" "$S/$CASE.png.log" | sed -E 's/.*hero sound: //; s/ fired,//' | awk '{print $1, $3, $5}' | uniq -c | awk '{print "  " $1 "x", $2, "gain", $3, "pitch", $4}'
  grep -E "hero sound: on true" "$S/$CASE.png.log" | sed -E 's/.*state ([A-Za-z]+) .*air speed ([0-9.e+-]+) m\/s q ([0-9.e+-]+) Pa mach ([0-9.e+-]+).* p ([0-9.e+-]+) Pa.*fastFlight_loop level ([0-9.]+) pitch ([0-9.]+).*fasterFlight_loop level ([0-9.]+).*sentry_flightfx_loop level ([0-9.]+).*/  \1 air \2 m\/s q \3 mach \4 p \5 | fast \6 at \7, faster \8, hum \9/' | cut -c1-300
  python -I .optimize/bench/hero_sound.py "$S/$CASE.csv" | head -4 | tail -3 | cut -c1-200
done
