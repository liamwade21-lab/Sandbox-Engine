#!/usr/bin/env bash
# bash .optimize/bench/session_rs.sh NAME ROUNDS [env...]: on the user's planet RS 8514-912-7-1071286-73 2 only: each round two new spots at
# 3 km, then the same view from 2000 km measured (paused, fixed date). NAME.perf.
N="$1"; R="$2"; shift 2
cd "/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2" || exit 1
D=.optimize/scratch/run; rm -f $D/$N.perf
C="preset Ultra; wait 600; rs RS 8514-912-7-1071286-73; wait 600; select RS 8514-912-7-1071286-73 2; go 10000; wait 300"
for i in $(seq 1 $R); do
  C="$C; resume; surface $((i*9)) $((i*23)) 3 -10 0; wait 500; surface $((i*13-40)) $((i*37-90)) 3 -10 0; wait 500"
  C="$C; date 2026-11-03 21:45:22; pause; surface 10 20 2000 -60 0; wait 900; perf rs$i; wait 600; perf end"
done
C="$C; wait 30"
NO_COLOR=1 RUST_LOG_STYLE=never SE_NO_MUSIC=1 SE_NO_VSYNC=1 SE_PERF="$D/$N.perf" env "$@" timeout 2400 target/release/spaceengine-rs.exe --run "$C" --screenshot $D/$N.png --frames 5 --size 3440x1440 > $D/$N.log 2>&1
echo "exit $?"; grep -E "not found|panic" $D/$N.log | cut -c1-160 | sort | uniq -c | head -5
python -I - "$D/$N.perf" <<'PY'
import sys
for l in open(sys.argv[1]):
    f=l.rstrip().split('\t')
    keep=[x for x in f if x.split(' ')[0] in ('meshes','all_entities','wgpu','render')]
    g={x.rpartition(' ')[0].split('/')[0]:x.rpartition(' ')[2] for x in f[7:] if 'elapsed_gpu' in x and x.count('/')==1}
    print(f[0], f[2], 'opaque', g.get('main_opaque_pass_3d'), '|', ' | '.join(keep))
PY
