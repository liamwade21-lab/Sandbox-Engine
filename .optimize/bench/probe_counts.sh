#!/usr/bin/env bash
# The buildings' probe (Viltrumite plan, step 9): the lists' counts and lag while he hovers, walks and flies low at 300 m/s,
# 3 km/s (30 m up) and 30 km/s (HIGH_M, default 150 m: at 30 m he hits the coarse ground ahead and crashes), the way the camera looks at first. A `hero probe stats` per segment (reset before each).
#   bash .optimize/bench/probe_counts.sh NAME "commands putting the camera ~25 m over a dense town" [run args, e.g. --size 3440x1440]
# Env: PRE (commands before the place, e.g. "preset Ultra"), and the run's env through `env` as usual (SE_NO_VSYNC=1 ...).
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s9; mkdir -p "$S"; N="$1"; P="$2"; shift 2
W="C:/Users/liamw/Desktop/SpaceEngine Rust Port Version 2/$S"
seg() { echo "hero probe stats reset; wait $1; hero probe stats; hero state"; }
C="timing frame; levels 3; ${PRE:-wait 1}; $P; wait 1500; agl off; mods on viltrumite; wait 200; hero probe trace $W/$N.csv; hero probe stats on; wait 60"
C="$C; $(seg 300); hero keys x 0.1; wait 400; hero keys w 12; wait 60; $(seg 480); hero keys shift+w 8; wait 150; $(seg 240)"
C="$C; hero keys space 0.05; wait 10; hero cruise 300 30; wait 120; $(seg 480); hero cruise 3000 30; wait 60; $(seg 240)"
C="$C; hero cruise 30000 ${HIGH_M:-150}; wait 20; $(seg 60); shot $W/${N}_30k.png; hero cruise off; hero keys z 0.05; wait 30; shot $W/${N}_end.png; wait 3"
PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" "$@" > /dev/null 2>&1
grep -E "panicked|ERROR|Validation" "$S/$N.png.log" | cut -c1-300 | head -5
grep -E "probe stats: [0-9]+ lists|hero: state|hero state" "$S/$N.png.log" | sed -E 's/^.*Z  INFO [a-z_:]+: //' | cut -c1-330
