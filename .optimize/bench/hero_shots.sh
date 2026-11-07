#!/usr/bin/env bash
# His one-shots, each trigger in turn: bash .optimize/bench/hero_shots.sh NAME [FPS]
# On the Sahara: X and a fall, a jump, a sprint's jump, Space held, a dive, Shift in Hover held to Hyper, the right button, then
# keys mashed; then the sea. FPS is what `timing frame` steps by (each `wait` scales with it, `hero keys` goes by real seconds), so
# 20, 60 and 240 must fire the same list. Prints what fired, and the most one-shots playing at once.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s6; mkdir -p "$S"; N="$1"; FPS="${2:-60}"
F="$(cygpath -m "$REPO/$S/$N.csv")"
w() { echo "wait $(( $1 * FPS / 60 ))"; }
HOVER="hero keys space 0; $(w 12); hero keys space 0; $(w 30)"
C="timing frame $FPS; date 2026-06-21 10:00:00; select Earth; surface 23 10 1 0; agl 60; wait 1500; mods on viltrumite; $(w 200); hero sound trace $F"
C="$C; hero keys x 0; $(w 330); hero sound"                                  # X, falling 60 m
C="$C; hero keys space 0; $(w 420); hero sound"                              # A jump
C="$C; hero keys w+shift 4; $(w 150); hero keys w+shift+space 0; $(w 420); hero sound" # A sprint's jump
C="$C; hero keys space 3; $(w 420); hero sound"                              # Space held: one jump
C="$C; $HOVER; hero keys space 2; $(w 180); hero aim 0 -85; hero keys w+shift 1.5; $(w 180); hero sound" # Take off, a dive
C="$C; $HOVER; hero aim 0 40; hero keys shift 3.5; $(w 330); hero sound"     # Shift in Hover, held to Hyper, let go
C="$C; hero keys rmb 2; $(w 150); hero sound"                                # The right button held
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do C="$C; hero keys shift 0.05; $(w 12)"; done   # Shift mashed
for i in 1 2 3 4 5 6 7 8 9 10; do C="$C; hero keys w 0.05; $(w 40)"; done        # W mashed
C="$C; hero sound; hero keys off; $(w 60); hero sound trace off; mods off; $(w 30); hero sound; $(w 5)"
PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" > /dev/null 2>&1
grep -E "panicked|ERROR" "$S/$N.png.log" | cut -c1-200 | head -3
grep -E "fired, gain" "$S/$N.png.log" | sed -E 's/.*hero sound: //; s/ fired,//' | awk '{print $1, $3, $5}' | uniq -c | awk '{print "  " $1 "x", $2, "gain", $3, "pitch", $4}'
grep -E "hero sound: on false" "$S/$N.png.log" | tail -1 | sed -E 's/.*(entities .*)/  off: \1/' | cut -c1-500
python -I - "$S/$N.csv" <<'PY'
import csv, sys
rows = list(csv.DictReader(open(sys.argv[1])))
print('  most one-shots at once', max(int(r['shots']) for r in rows), ', loops', max(int(r['loops']) for r in rows), ', frames', len(rows))
last = None
out = []
for r in rows:
    if r['state'] != last:
        last = r['state']
        out.append(last)
print('  states: ' + ' '.join(out))
PY
