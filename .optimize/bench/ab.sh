#!/usr/bin/env bash
# Scene-by-scene A/B, alternating which build goes first each round: bash ab.sh OUTDIR ROUNDS SCENE...
# AB_BASE / AB_NEW pick the two exes (default: the morning baseline vs target/release).
R="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; B="$R/.optimize/bench"; OUT="$1"; N="$2"; shift 2
BASE="${AB_BASE:-$R/.optimize/scratch/baseexe/spaceengine-rs.exe}"; NEW="${AB_NEW:-$R/target/release/spaceengine-rs.exe}"
for r in $(seq 1 "$N"); do
  for s in "$@"; do
    if [ $((r % 2)) -eq 1 ]; then order="base new"; else order="new base"; fi
    for w in $order; do
      exe=$BASE; [ "$w" = new ] && exe=$NEW
      PORT_BIN="$exe" bash "$B/all.sh" "$OUT/$w$r" "$s" > /dev/null 2>&1
    done
  done
done
echo done
