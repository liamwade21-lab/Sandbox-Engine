#!/usr/bin/env bash
# The sea's splash from a dive (spec step 8): up 200 m, then down at speed; pictures 0.3 s and 1 s after, from behind and the side.
#   bash .optimize/bench/hero_splash.sh NAME "date ...; select Earth; surface LAT LON 0.01 -6" [extra run args]
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
N="$1"; P="$2"; shift 2
C="timing frame; $P; wait 1500; mods on viltrumite; wait 300; hero keys space 10; wait 640; hero aim 0 -75; wait 2; hero keys ctrl+shift 1.6; wait 75; hero dist 40; hero aim 0 -20; wait 3; shot @/${N}_b0.png; wait 1; hero aim 90 -10; wait 2; shot @/${N}_s0.png; wait 30; shot @/${N}_s1.png; hero aim -90 -20; wait 2; shot @/${N}_b1.png; hero impacts; hero state; wait 3"
bash .optimize/bench/hero_impact.sh "$N" "$C" "$@"
cd .optimize/scratch/run/s8 && python -I - "$N" <<'PY'
import sys
from PIL import Image
n = sys.argv[1]
s = Image.new('RGB', (1600, 900))
for i, k in enumerate(['b0', 's0', 'b1', 's1']):
    try:
        s.paste(Image.open(f'{n}_{k}.png').convert('RGB').resize((800, 450)), ((i % 2) * 800, (i // 2) * 450))
    except Exception:
        print('missing', k)
s.save(f'{n}_sheet.png')
PY
