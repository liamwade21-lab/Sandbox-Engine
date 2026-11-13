#!/usr/bin/env bash
# His GPU and CPU cost at the user's size: forest, Ultra clouds. bash cost.sh NAME
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
S=.optimize/scratch/run; mkdir -p "$S"; N="$1"; rm -f "$S/$N.perf"
C="pause; vclouds 2; levels 3; date 2026-06-21 10:00:00; select Earth; surface 47.93 8.02 0.3 -6 30; agl 1.7; wait 1800; perf off1; wait 400; perf x; mods on viltrumite; wait 300; perf at6; wait 400; perf x; mods off; wait 60; perf off2; wait 400; perf end; wait 30"
env SE_PERF="$S/$N.perf" SE_NO_VSYNC=1 PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" --size 3440x1440 > /dev/null 2>&1
grep -E "ERROR|panic" "$S/$N.png.log" | cut -c1-200 | head -3
python -I - "$S/$N.perf" <<'PY'
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t'); d={}
    for x in f[7:]:
        k,_,v=x.rpartition(' ')
        d[k]=v
    print(f[0].ljust(6),'median',f[2],'opaque',d.get('main_opaque_pass_3d/elapsed_gpu'),'cpu_main',d.get('cpu_main'),'cpu_render',d.get('cpu_render'),'cpu_graph',d.get('cpu_graph'))
PY
