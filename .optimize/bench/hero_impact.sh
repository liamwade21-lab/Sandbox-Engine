#!/usr/bin/env bash
# Impact scenes (spec step 8): bash .optimize/bench/hero_impact.sh NAME "dev; commands" [extra run args]
# Pictures and logs in .optimize/scratch/run/s8. In the commands, @ stands for that folder as a Windows path: `shot @/a.png`.
# Runs the agent build unless PORT_BIN says otherwise; prints the panics, the impacts' log lines and how many pictures were saved.
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s8; mkdir -p "$S"; N="$1"; W="C:/Users/liamw/Desktop/Sandbox Engine/$S"
C="${2//@/$W}"; shift 2
PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" "$@" > /dev/null 2>&1
L="$S/$N.png.log"
grep -E "panicked|ERROR|wgpu error|Validation" "$L" | cut -c1-300 | head -5
grep -E "hero impacts:|viltrumite: crater|viltrumite: impacts|hero: on" "$L" | sed -E 's/^.*(hero impacts:|viltrumite: crater|viltrumite: impacts|hero: on)/\1/' | cut -c1-400
grep -c "Screenshot saved" "$L" | sed 's/^/pictures saved: /'
