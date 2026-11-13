#!/usr/bin/env bash
# Jupiter's volumetric decks at Ultra 3440x1440, time paused, from 100 km over the cloud tops down to the
# deep deck (100 km under them), the day side: bash .optimize/bench/jupiter_decks.sh NAME [env VAR=val ...]
N="$1"; shift 1
D="/c/Users/liamw/Desktop/Sandbox Engine/.optimize/scratch/run"; mkdir -p "$D"
A="surface 10 170"
C="preset Ultra; date 2026-11-04 12:00:00; select Jupiter; wait 300; date 2026-11-04 12:00:00; pause; spin 0; $A 100 -20 0; wait 1500; perf km100; wait 300; $A 30 -5 0; wait 600; perf km30; wait 300; $A -20 0 0; wait 600; perf in_main; wait 300; $A -50 0 0; wait 600; perf gap; wait 300; $A -100 0 0; wait 600; perf deep; wait 300; perf end; wait 30"
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
    print(f[0], f[2], 'opaque',d.get('main_opaque_pass_3d'),'clouds',d.get('volumetric_clouds'), sub)
" "$D/$N.perf"
