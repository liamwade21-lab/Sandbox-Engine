#!/usr/bin/env bash
# Final A/B: the baseline exe (with its own shaders, .optimize/scratch/baseexe) against the merged
# release build, scene sets interleaved twice, then reference shots base / new / base again.
R="/c/Users/liamw/Desktop/Sandbox Engine"; B="$R/.optimize/bench"; S="$R/.optimize/scratch/final"
BASE="$R/.optimize/scratch/baseexe/spaceengine-rs.exe"; NEW="$R/target/release/spaceengine-rs.exe"
for r in 1 2; do
  PORT_BIN="$BASE" bash "$B/all.sh" "$S/base$r" > "$S/base$r.txt" 2>&1
  PORT_BIN="$NEW" bash "$B/all.sh" "$S/new$r" > "$S/new$r.txt" 2>&1
done
PORT_BIN="$BASE" bash "$B/refs.sh" "$S/refs_base1" > /dev/null 2>&1
PORT_BIN="$NEW" bash "$B/refs.sh" "$S/refs_new" > /dev/null 2>&1
PORT_BIN="$BASE" bash "$B/refs.sh" "$S/refs_base2" > /dev/null 2>&1
echo done
