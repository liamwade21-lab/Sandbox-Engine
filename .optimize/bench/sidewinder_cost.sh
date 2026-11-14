#!/usr/bin/env bash
# The Sidewinder mod's cost at the user's size and settings (step 7.2): on foot before any spawn, the ship parked in view,
# seated on the ground, flying, in supercruise. Real frame times (no `timing frame`), VSync off. bash sidewinder_cost.sh NAME
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
S=.optimize/scratch/run; mkdir -p "$S"; N="${1:-swcost}"; rm -f "$S/$N.perf"
C="vclouds 2; levels 3; date 2026-11-12 01:00:00; wait 1800; perf foot; wait 600; perf x; sw spawn; wait 300; perf parked; wait 600; perf x; sw board; wait 200; perf seated; wait 600; perf x; sw gear; sw thrust 0 1 0 3; wait 400; perf flying; wait 600; perf x; sw stick 1 0 0 4; wait 400; sw jump; wait 1800; perf cruise; wait 600; perf end; wait 30"
env SE_SANDBOX=1 SE_PERF="$S/$N.perf" SE_NO_VSYNC=1 PORT_BIN="${PORT_BIN:-target/release/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" --size 3440x1440 > /dev/null 2>&1
grep -E "ERROR|panic" "$S/$N.png.log" | cut -c1-200 | head -3
grep -E "sidewinder: (supercruise|liftoff)" "$S/$N.png.log" | sed -E 's/^.*sidewinder: //' | head -3
python -I - "$S/$N.perf" <<'PY'
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t'); d={}
    for x in f[7:]:
        k,_,v=x.rpartition(' ')
        d[k]=v
    if f[0] in ('x',): continue
    print(f[0].ljust(8),'median',f[2],'opaque',d.get('main_opaque_pass_3d/elapsed_gpu'),'cpu_main',d.get('cpu_main'),'cpu_render',d.get('cpu_render'),'cpu_graph',d.get('cpu_graph'))
PY
