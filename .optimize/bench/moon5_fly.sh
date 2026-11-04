#!/usr/bin/env bash
# The moon of the user's 2026-11-04 video, RS 8514-916-6-8246-147 5.1, as they flew it: Ultra 3440x1440,
# time running, from 8,000 km (the nebula behind it, then the moon), then flying at 56 km/s 72 and 22 km
# up and 10 km/s 5 km up: bash .optimize/bench/moon5_fly.sh NAME "extra commands" [env VAR=val ...]
N="$1"; X="$2"; shift 2
D="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2/.optimize/scratch/run"; mkdir -p "$D"
A="surface -6.6 66.8"
C="preset Ultra; date 2026-11-03 21:34:07; wait 600; rs RS 8514-916-6-8246-147; wait 600; select RS 8514-916-6-8246-147 5.1; go 10000; wait 600; date 2026-11-03 21:34:07; spin 0; $X; $A 8000 80 0; wait 1500; perf sky8000; wait 300; $A 8000 -90 0; wait 300; perf moon8000; wait 300; $A 72 -20 0; wait 300; agl 62000; fly 56; wait 60; perf fly72; wait 300; fly 0; $A 22 -20 0; wait 300; agl 12000; fly 56; wait 60; perf fly22; wait 300; fly 0; $A 5 -10 0; wait 300; agl 1500; fly 10; wait 60; perf fly5; wait 300; perf end; fly 0; wait 30"
cd "/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2" || exit 1
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
    print(f[0:7], 'opaque',d.get('main_opaque_pass_3d'),'transp',d.get('main_transparent_pass_3d'),'clouds',d.get('volumetric_clouds'), sub)
" "$D/$N.perf"
