#!/usr/bin/env bash
# The user's planet of their 2026-11-03 videos, RS 8514-3620-1-7-69 1 (two full cloud decks), at Ultra 3440x1440,
# time paused, perf from 3,400 km, 114 km, 3 km and the ground (the clouds' parts too):
# bash .optimize/bench/rs8514_ultra.sh NAME "extra commands" [env VAR=val ...]; writes .optimize/scratch/run/NAME.perf
N="$1"; X="$2"; shift 2
D="/c/Users/liamw/Desktop/Sandbox Engine/.optimize/scratch/run"; mkdir -p "$D"; rm -f "$D/$N.perf"
A="surface 47.65 125.78"
C="preset Ultra; date 2026-11-03 20:44:21; wait 600; rs RS 8514-3620-1-7-69; wait 600; select RS 8514-3620-1-7-69 1; go 30000; wait 600; date 2026-11-03 20:44:21; pause; spin 0; $X; surface 10.99 22.10 3409 -90; wait 1500; perf orbit; wait 300; $A 114 -20 90; wait 300; perf km114; wait 300; $A 3 -5 90; wait 600; perf km3; wait 300; $A 0.002 -5 90; agl 1.5; wait 600; perf ground; wait 300; perf end; wait 30"
cd "/c/Users/liamw/Desktop/Sandbox Engine" || exit 1
env SE_PERF="$D/$N.perf" SE_NO_VSYNC=1 "$@" bash .audit/tools/runport.sh "$D/$N.png" 5 "$C" --size 3440x1440 > /dev/null 2>&1
grep -E "not found|panic" "$D/$N.png.log" | cut -c12-160 | head -3
python -c "
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t'); d={}
    for x in f[7:]:
        k,_,v=x.rpartition(' ')
        if k.endswith('elapsed_gpu') and '/' in k and k.count('/')==1: d[k.split('/')[0]]=v
    sub=' '.join(x.split('/')[1]+'='+x.rpartition(' ')[2] for x in f[7:] if x.startswith('volumetric_clouds/') and x.count('/')==2 and 'elapsed_gpu' in x)
    print(f[0], f[2], 'opaque',d.get('main_opaque_pass_3d'),'transp',d.get('main_transparent_pass_3d'),'clouds',d.get('volumetric_clouds'), sub)
" "$D/$N.perf"
