#!/usr/bin/env bash
# The GPU scatter's cost on Earth's Black Forest at Ultra 3440x1440 (Ultra clouds), time paused: perf from
# 3 km, 300 m and the ground (1.7 m). bash .optimize/bench/scatter_ultra.sh NAME "extra commands" [env VAR=val ...]
# Writes .optimize/scratch/run/NAME.perf; prints per segment the median frame, the main opaque pass (the ground
# and the scatter's draws), the scatter's compute and the clouds. Compare a run with SE_SCATTER_TEST=1 to one without.
N="$1"; X="$2"; shift 2
D="/c/Users/liamw/Desktop/Sandbox Engine/.optimize/scratch/run"; mkdir -p "$D"; rm -f "$D/$N.perf"
A="surface 47.95 8.05"
C="preset Ultra; vclouds 2; date 2026-11-04 11:00:00; pause; select Earth; $X; $A 3 -20 90; agl 3000; wait 1500; perf km3; wait 400; agl 300; wait 1200; perf m300; wait 400; $A 0.3 -5 90; agl 1.7; wait 1200; perf ground; wait 400; perf end; wait 30"
cd "/c/Users/liamw/Desktop/Sandbox Engine" || exit 1
env SE_PERF="$D/$N.perf" SE_NO_VSYNC=1 "$@" bash .audit/tools/runport.sh "$D/$N.png" 5 "$C" --size 3440x1440 > /dev/null 2>&1
grep -E "not found|panic|ERROR" "$D/$N.png.log" | cut -c12-160 | head -3
python -I -c "
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t'); d={}
    for x in f[7:]:
        k,_,v=x.rpartition(' ')
        if k.endswith('elapsed_gpu') and k.count('/')==1: d[k.split('/')[0]]=v
    print(f[0], 'median', f[2], 'opaque', d.get('main_opaque_pass_3d'), 'scatter', d.get('scatter'), 'clouds', d.get('volumetric_clouds'))
" "$D/$N.perf"
