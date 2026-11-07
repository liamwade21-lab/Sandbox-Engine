#!/usr/bin/env bash
# What fx_pass costs when he's hot, at the user's size: 3440x1440, Ultra clouds, High vegetation off nothing. bash .optimize/bench/hero_fx_cost.sh NAME
# Flying at the table's 3,500 K, 6,000 K and 10,000 K over the sea with the view behind him and from the side, with the effects, then
# the same spot cold: `hero_fx/elapsed_gpu` is the pass.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s7; mkdir -p "$S"; N="$1"; rm -f "$S/$N.perf"
C="pause; timing frame; vclouds 2; levels 3; date 2026-06-21 12:00:00; select Earth; surface 0 -10 0.6 -3 90; wait 900; mods on viltrumite; wait 300; hero hold off; perf cold; wait 300; perf x; hero cruise 300 500; wait 60; perf cone; hero cruise 400 500; wait 200; perf x; hero cruise 2600 500; wait 120; perf t3500; wait 200; perf x; hero cruise 4900 500; wait 120; perf t6000; wait 200; perf x; hero cruise 9700 500; wait 120; perf t10000; wait 200; perf x; hero aim 90 0; wait 20; perf t10000side; wait 200; perf x; hero cruise 0 500; hero cruise off; wait 400; perf cold2; wait 300; perf x; mods off; wait 120; perf off; wait 300; perf end; wait 5"
env SE_PERF="$S/$N.perf" SE_NO_VSYNC=1 PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" --size 3440x1440 > /dev/null 2>&1
grep -E "ERROR|panicked" "$S/$N.png.log" | cut -c1-200 | head -3
python -I - "$S/$N.perf" <<'PY'
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t'); d={}
    for x in f[7:]:
        k,_,v=x.rpartition(' ')
        d[k]=v
    if f[0]=='x': continue
    keys=[k for k in d if 'hero_fx' in k]
    print(f[0].ljust(11),'median',f[2],'fx',{k.split('/')[-1]:d[k] for k in keys if 'gpu' in k},'clouds',d.get('volumetric_clouds/elapsed_gpu'),'opaque',d.get('main_opaque_pass_3d/elapsed_gpu'),'cpu_main',d.get('cpu_main'),'cpu_render',d.get('cpu_render'),'peak',d.get('wgpu_peak'))
PY
