#!/usr/bin/env bash
# All benchmark scenes, one after another: bash .optimize/bench/all.sh [OUTDIR] [SCENE...]
# Each scene marks segments with `perf NAME`; see scene.sh. Summary lines go to OUTDIR/*.perf.
B="/c/Users/liamw/Desktop/Sandbox Engine/.optimize/bench"
OUT="${1:-/c/Users/liamw/Desktop/Sandbox Engine/.optimize/scratch/run}"; shift
D="date 2026-06-21 12:00:00"
declare -A S
S[start]="perf startup; wait 300; perf end"
S[space]="$D; goto Earth 20000; wait 400; perf space_earth; wait 600; perf end"
S[galaxy]="$D; galactic 30000; wait 600; perf galaxy_view; wait 600; perf end"
S[fly]="$D; goto Earth 20000; wait 100; look Sagittarius A*; wait 60; perf fly_1e14; fly 1e14; wait 1200; perf fly_1e15; fly 1e15; wait 1200; perf end; fly 0"
S[earth]="$D; select Earth; surface 45 90 0.3 -10 0; wait 900; perf earth_still; wait 300; perf earth_look; spin 72; wait 1000; perf end; spin 0"
S[mars]="$D; select Mars; surface -14.6 175.5 0.3 -10 0; wait 900; perf mars_still; wait 300; perf mars_look; spin 72; wait 1000; perf end; spin 0"
S[nebula]="$D; select Skylias Stellar Nursery; go 150000000000000; wait 900; perf neb_still; wait 300; perf neb_look; spin 72; wait 1000; perf end; spin 0"
S[tarantula]="$D; select Tarantula Nebula; go 900000000000000; wait 900; perf tar_still; wait 300; perf tar_look; spin 72; wait 1000; perf end; spin 0"
R="goto Earth 20000; wait 200; rs RS 8514-1828-5-27938-309; wait 300; select RS 8514-1828-5-27938-309 2"
S[rsplanet]="$D; $R; surface 20 30 0.3 -10 0; wait 900; perf rs_still; wait 300; perf rs_look; spin 180; wait 1000; perf end; spin 0"
S[rsground]="$D; $R; surface -35 120 0.002 -5 0; wait 900; perf rsg_still; wait 300; perf rsg_look; spin 180; wait 1000; perf end; spin 0"
ORDER="${*:-start space galaxy fly earth mars rsplanet rsground nebula tarantula}"
for s in $ORDER; do bash "$B/scene.sh" "$s" "${S[$s]}" "$OUT"; done
