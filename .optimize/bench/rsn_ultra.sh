#!/usr/bin/env bash
# The user's RSN planet (their 2026-11-02 video) as they play it, Ultra at 3440x1440, uncapped:
# bash .optimize/bench/rsn_ultra.sh NAME [PRESET] [HEADING] [END_PITCH] [PLANET]; writes .optimize/scratch/run/NAME.perf
# PLANET: the planet number in that system (default 4, the user's; 2 is the Earth-like one with full cloud cover)
G="RG 0-7-460129-1254"; N="RN 0-7-460129-1254-197"; P="RSN 0-7-460129-1254-197-12"; S="surface 25.4618 -48.1856 0.002"
PRE="${2:-Ultra}"
C="preset $PRE; date 2026-10-30 19:35:26; select $G; wait 300; select $G; go 30000000000000000; wait 600; spin 0; goto $N; wait 300; spin 0; goto $N; wait 1500; spin 0; goto $P; wait 900; spin 0; goto $P; wait 300; select $P ${5:-4}; date 2026-10-30 19:35:26; pause; $S -5 0; agl 1.5; wait 900; perf horizon; wait 300; $S 35 0; wait 60; perf sky; wait 300; $S -60 0; wait 60; perf ground; wait 300; perf look; spin 72; wait 600; perf end; spin 0; $S ${4:--5} ${3:-0}; wait 60"
D="/c/Users/liamw/Desktop/Sandbox Engine/.optimize/scratch/run"; mkdir -p "$D"
SE_PERF="$D/$1.perf" SE_NO_VSYNC=1 bash "/c/Users/liamw/Desktop/Sandbox Engine/.audit/tools/runport.sh" "$D/$1.png" 5 "$C" --size 3440x1440 > /dev/null
grep -E "not found|panic" "$D/$1.png.log" | cut -c12-160 | head -3
cut -f1-7 "$D/$1.perf" | tail -4
