#!/usr/bin/env bash
# Every action of step 8 at one place, each from behind and the side (spec step 8):
#   bash .optimize/bench/hero_actions.sh NAME "date ...; select Earth; surface LAT LON 0.3 -6" [extra run args]
# A fall from 300 m (a hard landing at the air's terminal speed), a punch into the ground, a ground punch, three thunderclaps on the
# ground (charges 0.1, 0.5, 1), then up 120 m: a punch in the air and the three claps again. Pictures NAME_*.png in
# .optimize/scratch/run/s8 and two sheets, NAME_ground.png and NAME_air.png.
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
N="$1"; P="$2"; shift 2
pair() { echo "shot @/${N}_$1_b.png; wait 1; hero aim 90 -12; wait 2; shot @/${N}_$1_s.png; wait 1; hero aim -90 $2; wait 2"; }
# A clap held SECS: the clap comes 0.63 s after letting go; the pictures 0.35 s after it.
clap() { echo "hero keys rmb $2; wait $(python -I -c "print(int(($2 + 0.63 + 0.35) * 60))"); $(pair "$1" "$3"); wait 300"; }
C="timing frame; $P; agl 300; wait 1500; agl off; mods on viltrumite; wait 300; hero keys x 0.1; wait 800; hero impacts"
C="$C; hero dist 12; hero aim 0 -40; wait 3; hero keys lmb 0.05; wait 36; $(pair punch -40); wait 200"
C="$C; hero aim 0 -60; wait 3; hero keys lmb 0.05; wait 22; $(pair smash -60); wait 400"
C="$C; hero dist 22; hero aim 0 -12; wait 3; $(clap clap1 0.61 -12); $(clap clap5 1.45 -12); $(clap clap9 2.6 -12)"
C="$C; hero keys space 0.05; wait 20; hero keys space 0.05; wait 5; hero keys space 6; wait 380; hero state; hero dist 12; hero aim 0 -5; wait 30"
C="$C; hero keys lmb 0.05; wait 34; $(pair airpunch -5); wait 100; hero dist 22; hero aim 0 -12; wait 3"
C="$C; $(clap airclap1 0.61 -12); $(clap airclap5 1.45 -12); $(clap airclap9 2.6 -12); hero impacts; wait 3"
bash .optimize/bench/hero_impact.sh "$N" "$C" "$@"
cd .optimize/scratch/run/s8 && python -I - "$N" <<'PY'
import sys
from PIL import Image
n = sys.argv[1]
W, H = 640, 360
for sheet_name, rows in [('ground', ['punch', 'smash', 'clap1', 'clap5', 'clap9']), ('air', ['airpunch', 'airclap1', 'airclap5', 'airclap9'])]:
    sheet = Image.new('RGB', (W * 2, H * len(rows)))
    for i, r in enumerate(rows):
        for j, k in enumerate('bs'):
            try:
                sheet.paste(Image.open(f'{n}_{r}_{k}.png').convert('RGB').resize((W, H)), (j * W, i * H))
            except Exception:
                print('missing', r, k)
    sheet.save(f'{n}_{sheet_name}.png')
PY
