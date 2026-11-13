#!/usr/bin/env bash
# The moon of the user's 2026-11-04 video, RS 8514-916-6-8246-147 5.1 (hot arid terra, cumulus at 12-21 km
# and two veils at 90-137 km), at Ultra 3440x1440, time paused, perf from 4,782 km down to 7 km (the clouds'
# parts too): bash .optimize/bench/moon5_ultra.sh NAME "extra commands" [env VAR=val ...]; writes
# .optimize/scratch/run/NAME.perf
N="$1"; X="$2"; shift 2
D="/c/Users/liamw/Desktop/Sandbox Engine/.optimize/scratch/run"; mkdir -p "$D"
A="surface -6.6 66.8"
C="preset Ultra; date 2026-11-03 21:34:07; wait 600; rs RS 8514-916-6-8246-147; wait 600; select RS 8514-916-6-8246-147 5.1; go 10000; wait 600; date 2026-11-03 21:34:07; pause; spin 0; $X; $A 4782 -90; wait 1500; perf orbit; wait 300; $A 346 -60 0; wait 300; perf km346; wait 300; $A 72 -45 0; wait 300; perf km72; wait 300; $A 30 -30 0; wait 300; perf km30; wait 300; $A 13 -10 0; wait 300; perf km13; wait 300; $A 7 0 0; wait 300; perf km7; wait 300; perf end; wait 30"
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
