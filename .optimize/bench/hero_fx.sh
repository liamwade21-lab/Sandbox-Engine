#!/usr/bin/env bash
# Heat and boom scenes (spec step 7): bash .optimize/bench/hero_fx.sh NAME "dev; commands" [extra run args]
# Pictures and logs in .optimize/scratch/run/s7. In the commands, @ stands for that folder as a Windows path: `shot @/a.png`,
# `hero boom @/a.csv`. Runs the agent build unless PORT_BIN says otherwise; prints the panics, `hero air` lines and pictures saved.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s7; mkdir -p "$S"; N="$1"; W="C:/Users/liamw/Desktop/SpaceEngine Rust Port Version 2/$S"
C="${2//@/$W}"; shift 2
PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" "$@" > /dev/null 2>&1
L="$S/$N.png.log"
grep -E "panicked|ERROR|wgpu error|Validation" "$L" | cut -c1-300 | head -5
grep -E "hero air:" "$L" | sed -E 's/.*hero air: //' | cut -c1-700
grep -c "Screenshot saved" "$L" | sed 's/^/pictures saved: /'
