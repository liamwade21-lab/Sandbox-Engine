#!/usr/bin/env bash
# What the impacts cost at the user's size: 3440x1440, Ultra clouds, levels 3 (spec step 8). bash .optimize/bench/hero_impact_cost.sh NAME
# On Kansas farmland, standing: none; five craters in view (`fx_decals`); a 40 m crater's dust at its thickest (the dust, in `hero_fx`, and
# `hero_fx_dust_tint`); a full thunderclap (rings and its ring of dust); the mod off with the craters still there (the aftermath); then
# none. For passes under 0.05 ms perf.rs's limit must be lowered for the run (it drops them from the log).
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s8; mkdir -p "$S"; N="$1"; rm -f "$S/$N.perf"
C="pause; timing frame; vclouds 2; levels 3; date 2026-06-21 18:00:00; select Earth; surface 38.5 -98.5 0.3 -6; agl 300; wait 1500; agl off; mods on viltrumite; wait 300; hero keys x 0.1; wait 800; hero dist 40; hero aim 0 -30; wait 300; perf none; wait 300; perf x"
C="$C; hero crater 8; wait 2; hero aim 72 -30; hero crater 14; wait 2; hero aim 72 -30; hero crater 20; wait 2; hero aim 72 -30; hero crater 5; wait 2; hero aim 72 -30; hero crater 30; wait 900; hero aim 0 -30; wait 3; perf craters5; wait 300; perf x"
C="$C; hero crater 40; wait 30; perf dust40; wait 120; perf x; wait 600; hero dist 22; hero aim 0 -12; wait 3; hero keys rmb 2.6; wait 192; perf clap; wait 90; perf x; wait 600"
C="$C; mods off; wait 60; perf aftermath; wait 300; perf x; surface 0 0 25 -30; wait 300; perf off; wait 300; perf end; wait 5"
env SE_PERF="$S/$N.perf" SE_NO_VSYNC=1 SE_HERO_GRAB=1 PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" --size 3440x1440 > /dev/null 2>&1
grep -E "ERROR|panicked" "$S/$N.png.log" | cut -c1-200 | head -3
python -I - "$S/$N.perf" <<'PY'
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t'); d={}
    for x in f[7:]:
        k,_,v=x.rpartition(' ')
        d[k]=v
    if f[0]=='x': continue
    keys=[k for k in d if ('hero_fx' in k or 'fx_decals' in k) and 'gpu' in k]
    print(f[0].ljust(10),f[3],{k.replace('/elapsed_gpu',''):d[k] for k in keys},'clouds',d.get('volumetric_clouds/elapsed_gpu'),'opaque',d.get('main_opaque_pass_3d/elapsed_gpu'),'cpu_main',d.get('cpu_main'),'peak',d.get('wgpu_peak'))
PY
