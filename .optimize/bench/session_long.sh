#!/usr/bin/env bash
# bash .optimize/bench/session_long.sh NAME ROUNDS [env...]: a session that keeps exploring (Mars, Jupiter, the Moon,
# two procedural planets near the ground), with Earth's view and the user's planet RS 8514-912-7-1071286-73 2 from
# 2000 km measured each round (paused, fixed dates) and the video memory sampled every 15 s (NAME.vram).
N="$1"; R="$2"; shift 2
cd "/c/Users/liamw/Desktop/Sandbox Engine" || exit 1
D=.optimize/scratch/run; rm -f $D/$N.perf $D/$N.vram
C="preset Ultra; wait 300"
for i in $(seq 1 $R); do
  C="$C; select Earth; date 2026-08-10 12:00:00; pause; surface 20 -40 800 -60 0; wait 900; perf earth$i; wait 600; perf trip$i; resume"
  C="$C; select Mars; surface $((i*7)) $((i*31)) 2 -10 0; wait 500; surface $((i*7)) $((i*31+5)) 300 -60 0; wait 300"
  C="$C; select Jupiter; go 200000; wait 300; select Moon; surface $((i*11)) $((i*17)) 3 -10 0; wait 500"
  C="$C; rs RS 8514-912-7-1071286-73; wait 400; select RS 8514-912-7-1071286-73 2; surface $((i*9)) $((i*23)) 3 -10 0; wait 600"
  C="$C; date 2026-11-03 21:45:22; pause; surface 10 20 2000 -60 0; wait 900; perf rs$i; wait 600; perf trip${i}b; resume"
  C="$C; rs RS 8514-916-6-8246-147; wait 400; select RS 8514-916-6-8246-147 5.1; surface $((i*5)) $((i*41)) 5 -10 0; wait 500"
  C="$C; select Sun; wait 60"
done
C="$C; perf end; wait 30"
powershell -NoProfile -Command "\$f='$D/$N.vram'; Start-Sleep 20; while (\$p = Get-Process spaceengine-rs -ErrorAction SilentlyContinue) { \$c = (Get-Counter ('\GPU Process Memory(pid_' + \$p.Id + '*)\Dedicated Usage') -ErrorAction SilentlyContinue).CounterSamples | Measure-Object CookedValue -Sum; ('{0} {1:N0}' -f (Get-Date -Format HH:mm:ss), (\$c.Sum/1MB)) | Out-File -Append -Encoding ascii \$f; Start-Sleep 15 }" &
NO_COLOR=1 RUST_LOG_STYLE=never SE_NO_MUSIC=1 SE_NO_VSYNC=1 SE_PERF="$D/$N.perf" env "$@" timeout 2400 target/release/spaceengine-rs.exe --run "$C" --screenshot $D/$N.png --frames 5 --size 3440x1440 > $D/$N.log 2>&1
echo "exit $?"; wait
grep -E "not found|panic" $D/$N.log | cut -c1-160 | sort | uniq -c | head -5
