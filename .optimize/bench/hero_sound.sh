#!/usr/bin/env bash
# His sounds' levels by the air: bash .optimize/bench/hero_sound.sh NAME "select BODY; surface LAT LON KM 0"
# Hovers, flies at 20 m/s, about 300 m/s, 3 km/s and 30 km/s, stops, holds the right button, turns off. Then hero_sound.py
# checks the trace against F10's table and prints the levels; the log's `hero sound` lines have what the sinks really hold.
REPO="/c/Users/liamw/Desktop/Sandbox Engine"; cd "$REPO" || exit 1
S=.optimize/scratch/run/s6; mkdir -p "$S"; N="$1"; W="C:/Users/liamw/Desktop/Sandbox Engine/$S/$N.csv"
C="timing frame; date 2026-06-21 10:00:00; $2; wait 1200; mods on viltrumite; wait 120; hero sound trace $W; wait 30; hero sound; hero keys w 100; wait 180; hero sound; hero keys w+shift 100; wait 60; hero keys w 100; wait 60; hero sound; hero keys w+shift 100; wait 120; hero keys w 100; wait 60; hero sound; hero keys w+shift 100; wait 180; hero keys w 100; wait 120; hero sound; hero keys off; wait 240; hero sound; hero keys rmb 2; wait 60; hero sound; wait 90; hero sound; mods off; wait 5; hero sound; wait 30; hero sound; hero sound trace off; wait 5"
PORT_BIN="${PORT_BIN:-target/agent/spaceengine-rs.exe}" bash .audit/tools/runport.sh "$S/$N.png" 5 "$C" > /dev/null 2>&1
grep -E "panic|ERROR" "$S/$N.png.log" | cut -c1-200 | head -3
grep -E "sound: on (true|false)" "$S/$N.png.log" | sed -E 's/.*hero sound: //' | cut -c1-140
python -I .optimize/bench/hero_sound.py "$S/$N.csv"
