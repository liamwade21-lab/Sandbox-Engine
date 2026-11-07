#!/usr/bin/env bash
# A hard landing's crater from a dive, from behind and the side, with its dust and once it has cleared (spec step 8):
#   bash .optimize/bench/hero_crater.sh NAME "date ...; select BODY; surface LAT LON 1.5 -10" ["extra commands at the end"] [extra run args]
# Pictures in .optimize/scratch/run/s8: NAME_dust_b/s.png (2.5 s after the dive starts, about when he lands), NAME_b/s.png (cleared),
# and a sheet NAME_sheet.png.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
N="$1"; P="$2"; X="${3:-}"; shift 3 2>/dev/null || shift $#
C="timing frame; $P; agl 1500; wait 1500; agl off; mods on viltrumite; wait 300; hero aim 0 -70; wait 2; hero keys ctrl+shift 2.5; wait 150; hero dist 30; hero aim 0 -30; wait 3; shot @/${N}_dust_b.png; wait 1; hero aim 90 -18; wait 2; shot @/${N}_dust_s.png; hero impacts; wait 600; hero aim -90 -30; wait 3; shot @/${N}_b.png; wait 1; hero aim 90 -18; wait 2; shot @/${N}_s.png; $X; wait 3"
bash .optimize/bench/hero_impact.sh "$N" "$C" "$@"
cd .optimize/scratch/run/s8 && python -I - "$N" <<'PY'
import sys
from PIL import Image
n = sys.argv[1]
W, H = 800, 450
sheet = Image.new('RGB', (W * 2, H * 2))
for i, k in enumerate(['dust_b', 'dust_s', 'b', 's']):
    try:
        sheet.paste(Image.open(f'{n}_{k}.png').convert('RGB').resize((W, H)), ((i % 2) * W, (i // 2) * H))
    except Exception:
        print('missing', k)
sheet.save(f'{n}_sheet.png')
PY
