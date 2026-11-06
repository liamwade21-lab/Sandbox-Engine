#!/usr/bin/env bash
# ENV_A="X=0" ENV_B="Y=1" bash .optimize/bench/ab_nodes.sh NAME: the user's planet RS 8514-495-4-971-146 2 from 30 km
# on a first visit, alternating two sets of env switches (A, B, A, B). GPU times from separate moments are not
# comparable (a capped or unfocused window clocks the card down): compare the rounds next to each other, and only
# rounds whose frame median is not stuck at 16.6 ms. NAME*.perf / .log in .optimize/scratch/run.
N="$1"; shift
cd "/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2" || exit 1
D=.optimize/scratch/run; U="RS 8514-495-4-971-146"
C="preset Ultra; wait 300; rs $U; wait 400; select $U 2; date 2026-11-03 22:10:00; pause; surface 10 20 30 -20 0; wait 1500; perf q; wait 600; perf end; wait 30"
for r in a1 b1 a2 b2; do
  E="$ENV_A"; case $r in b*) E="$ENV_B";; esac
  rm -f "${D:?}/${N:?}$r.perf"
  env $E NO_COLOR=1 SE_NO_MUSIC=1 SE_NO_VSYNC=1 SE_PERF="$D/$N$r.perf" timeout 300 ${PORT_BIN:-target/release/spaceengine-rs.exe} --run "$C" --screenshot "$D/$N$r.png" --frames 5 --size 3440x1440 > "$D/$N$r.log" 2>&1
  echo "$r [$E] $(python -I .optimize/bench/session_table.py "$D/$N$r.perf" | cut -c1-120)"
done
