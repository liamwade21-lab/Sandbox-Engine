#!/usr/bin/env bash
# bash .optimize/bench/session_scatter.sh NAME ROUNDS [env...]: the GPU scatter over a session on Earth. Each round
# visits two new spots 50 m above the ground (new cells and height pages), then measures the same view (300 m over
# the Black Forest, paused, fixed date). Prints per round the opaque GPU time, the scatter's compute and the counts
# (GPU objects must stay flat). NAME.perf in .optimize/scratch/run. Set PORT_BIN to test another exe.
N="$1"; R="$2"; shift 2
cd "/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2" || exit 1
D=.optimize/scratch/run; mkdir -p $D; rm -f $D/$N.perf
C="preset Ultra; vclouds 2; select Earth; wait 300"
for i in $(seq 1 $R); do
  C="$C; resume; surface $((40 + i)) $((i * 7 - 60)) 0.3 -10 0; agl 50; wait 400; surface $((45 - i)) $((i * 11)) 0.3 -10 0; agl 50; wait 400"
  C="$C; date 2026-11-04 11:00:00; pause; surface 47.95 8.05 0.3 -25 90; agl 300; wait 900; perf s$i; wait 600; perf end"
done
C="$C; wait 30"
NO_COLOR=1 RUST_LOG_STYLE=never SE_NO_MUSIC=1 SE_NO_VSYNC=1 SE_PERF="$D/$N.perf" env "$@" timeout 3000 ${PORT_BIN:-target/release/spaceengine-rs.exe} --run "$C" --screenshot $D/$N.png --frames 5 --size 3440x1440 > $D/$N.log 2>&1
echo "exit $?"; grep -E "not found|panic|ERROR" $D/$N.log | cut -c1-160 | sort | uniq -c | head -5
python -I - "$D/$N.perf" <<'PY'
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t')
    keep=[x for x in f if x.split(' ')[0] in ('meshes','all_entities','wgpu','render')]
    g={x.rpartition(' ')[0].split('/')[0]:x.rpartition(' ')[2] for x in f[7:] if 'elapsed_gpu' in x and x.count('/')==1}
    print(f[0], f[2], 'opaque', g.get('main_opaque_pass_3d'), 'scatter', g.get('scatter'), '|', ' | '.join(keep))
PY
