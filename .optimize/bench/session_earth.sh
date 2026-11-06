#!/usr/bin/env bash
# bash .optimize/bench/session_earth.sh NAME ROUNDS [env...]: the same view of New York from 300 m (paused, fixed
# date) measured after each trip away to another star system and back. The main schedules' CPU time grew by about
# 1 ms a visit there (2026-11-05, session_explore.sh); compare with env switches (SE_SCATTER=0, SE_ORBITAL=0) to
# find what piles up; TRIP="commands" replaces the trip away. NAME.perf / .log in .optimize/scratch/run. Set PORT_BIN to test another exe.
N="$1"; R="$2"; shift 2
cd "/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2" || exit 1
D=.optimize/scratch/run; mkdir -p $D; rm -f $D/$N.perf
C="preset Ultra; wait 300"
for i in $(seq 1 $R); do
  C="$C; agl off; select Earth; date 2026-11-04 15:00:00; pause; surface 40.75 -73.98 0.3 -10 40; agl 300; wait 1200; perf e$i; wait 600; perf trip$i; resume"
  C="$C; agl off; ${TRIP:-rs RS 8514-916-2-2-17; wait 300}"
done
C="$C; perf end; wait 30"
NO_COLOR=1 RUST_LOG_STYLE=never SE_NO_MUSIC=1 SE_NO_VSYNC=1 SE_PERF="$D/$N.perf" env "$@" timeout 1800 ${PORT_BIN:-target/release/spaceengine-rs.exe} --run "$C" --screenshot $D/$N.png --frames 5 --size 3440x1440 > $D/$N.log 2>&1
echo "exit $?"
grep -E "not found|panic|ERROR" $D/$N.log | cut -c1-160 | sort | uniq -c | head -8
python -I .optimize/bench/session_table.py "$D/$N.perf" | cut -c1-150
