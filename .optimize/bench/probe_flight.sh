#!/usr/bin/env bash
# The buildings' probe (Viltrumite plan, step 9): a near-far-near flight with the capsule anchored over a town, so the same
# buildings come in the near list, then the far one, then the near one again; on the way out the near and far lists in full
# for a moment (`hero probe both`). Logs the lists' lag and counts and the near-far shift (`hero probe stats`).
#   bash .optimize/bench/probe_flight.sh NAME "commands putting the camera ~25 m over the town" [SPEED_MS] [LEG_FRAMES] [env VAR=..]
# A line per list in .optimize/scratch/run/s9/NAME.csv (reach_m: the anchor from the camera); log NAME.png.log.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s9; mkdir -p "$S"; N="$1"; P="$2"; V="${3:-1000}"; L="${4:-1000}"; shift 2; shift 2 2>/dev/null
W="C:/Users/liamw/Desktop/SpaceEngine Rust Port Version 2/$S"
C="timing frame; levels 3; $P; wait 1500; agl off; mods on viltrumite; wait 200; hero probe trace $W/$N.csv; hero probe stats on; hero probe anchor ${ANCHOR_M:-300}; hero probe log 1; wait 100"
C="$C; hero dist 30; hero aim 0 -30; wait 3; shot $W/${N}_start.png; hero aim 0 30; wait 3"
C="$C; hero cruise $V 300; wait $((L / 4)); hero probe both on; hero probe log 1; wait 60; hero probe both off; wait $((L * 3 / 4)); hero cruise off; hero keys z 0.05; wait 30"
C="$C; hero aim 180 0; wait 3; hero cruise $V 300; wait $L; hero cruise off; hero keys z 0.05; wait 60; hero probe stats; hero probe log 1; wait 5"
env "$@" PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" > /dev/null 2>&1
grep -E "panicked|ERROR|Validation" "$S/$N.png.log" | cut -c1-300 | head -5
grep -E "probe (stats|shift|ids)|probe: list|hero probe" "$S/$N.png.log" | sed -E 's/^.*Z  INFO [a-z_:]+: //' | cut -c1-420
