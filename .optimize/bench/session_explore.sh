#!/usr/bin/env bash
# bash .optimize/bench/session_explore.sh NAME ROUNDS [env...]: a session exploring as the user does (2026-11-05:
# 80 fps after a restart, 40s after exploring). Each round measures the same views (paused, fixed dates) of the
# user's planet RS 8514-495-4-971-146 2 from 30 km (p) and of New York from 300 m (e), then visits a forest, a
# generated life world's ground, a level 2 town, the level 4 system's stations, and four other star systems.
# `agl off` before every measured view: the hold above the ground stays on until turned off (it once brought the
# camera back 50 m over the planet: twice the ground, mistaken for a slowdown). Prints per measured view the frame
# median, the CPU time of the main schedules / the render thread's preparation (unaffected by a frame cap from
# outside), the GPU time and the counts (GPU objects must stay flat); video memory every 15 s in NAME.vram.
# NAME.perf / .log in .optimize/scratch/run. Set PORT_BIN to test another exe.
N="$1"; R="$2"; shift 2
cd "/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2" || exit 1
D=.optimize/scratch/run; mkdir -p $D; rm -f $D/$N.perf $D/$N.vram
U="RS 8514-495-4-971-146"; L="RS 8514-928-8-5906585-158"; W="WISEPA J090649.36+473538.6"; S="SDSS J212413.88+010000.2"
P="agl off; rs $U; wait 400; select $U 2; date 2026-11-03 22:10:00; pause; surface 10 20 30 -20 0; wait 1500"
C="preset Ultra; wait 300"
for i in $(seq 1 $R); do
  C="$C; $P; perf p$i; wait 600; perf v_nyc$i; resume"
  C="$C; agl off; select Earth; date 2026-11-04 15:00:00; pause; surface 40.75 -73.98 0.3 -10 40; agl 300; wait 1200; perf e$i; wait 600; perf v_forest$i; resume"
  C="$C; surface $((34 + i)) $((i * 3 - 95)) 0.2 -10 0; agl 60; wait 400"
  C="$C; perf v_life$i; agl off; rs $L; wait 400; select $L 3; surface $((i * 13)) $((i * 29)) 0.3 -10 0; agl 50; wait 500"
  C="$C; perf v_town$i; agl off; select $W; wait 200; go 300000000; wait 400; select $W 1; town city 3; wait 500"
  C="$C; perf v_space$i; select $S; wait 200; go 300000000; wait 400; select $S 1; surface 20 0 50000 -90; wait 300; station $((i % 3)) 5; wait 300"
  C="$C; perf v_stars$i; rs RS 8514-916-2-2-17; wait 250; rs RS 8514-916-4-132-92; wait 250; rs RS 8514-916-5-1030-108; wait 250; rs RS 8514-495-3-123-106; wait 250; perf v_back$i"
done
C="$C; $P; perf p_end; wait 600; perf end; wait 30"
powershell -NoProfile -Command "\$f='$D/$N.vram'; Start-Sleep 20; while (\$p = Get-Process spaceengine-rs -ErrorAction SilentlyContinue) { \$c = (Get-Counter ('\GPU Process Memory(pid_' + \$p.Id + '*)\Dedicated Usage') -ErrorAction SilentlyContinue).CounterSamples | Measure-Object CookedValue -Sum; ('{0} {1:N0}' -f (Get-Date -Format HH:mm:ss), (\$c.Sum/1MB)) | Out-File -Append -Encoding ascii \$f; Start-Sleep 15 }" &
NO_COLOR=1 RUST_LOG_STYLE=never SE_NO_MUSIC=1 SE_NO_VSYNC=1 SE_PERF="$D/$N.perf" env "$@" timeout 3000 ${PORT_BIN:-target/release/spaceengine-rs.exe} --run "$C" --screenshot $D/$N.png --frames 5 --size 3440x1440 > $D/$N.log 2>&1
echo "exit $?"; wait
grep -E "not found|panic|ERROR" $D/$N.log | cut -c1-160 | sort | uniq -c | head -8
python -I .optimize/bench/session_table.py "$D/$N.perf"
