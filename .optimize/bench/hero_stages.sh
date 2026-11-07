#!/usr/bin/env bash
# The heat's stages by the table's speeds, each from behind (the real camera), 20 degrees off his line, and from the side, on a sheet:
#   bash .optimize/bench/hero_stages.sh NAME "date ...; select BODY; surface LAT LON KM -3 90" HEIGHT_M "V1 V2 V3 ..." [extra run args]
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
N="$1"; P="$2"; H="$3"; SPEEDS="$4"; shift 4
S=.optimize/scratch/run/s7
C="timing frame; $P; wait 900; mods on viltrumite; wait 200"
for v in $SPEEDS; do
  C="$C; hero cruise $v $H; wait 170; shot @/${N}_${v}_b.png; wait 1; hero aim 20 0; wait 40; shot @/${N}_${v}_t.png; wait 1; hero aim 70 0; wait 2; shot @/${N}_${v}_s.png; wait 1; hero aim -90 0; wait 2"
done
bash .optimize/bench/hero_fx.sh "$N" "$C" "$@"
cd "$S" && python -I - "$N" $SPEEDS <<'PY'
import sys
from PIL import Image
n,*vs=sys.argv[1:]
W,H=640,360
sheet=Image.new('RGB',(W*3,H*len(vs)))
for i,v in enumerate(vs):
    for j,k in enumerate('bts'):
        try:
            sheet.paste(Image.open(f'{n}_{v}_{k}.png').convert('RGB').resize((W,H)),(j*W,i*H))
        except Exception as e:
            print('missing',v,k)
sheet.save(f'{n}_sheet.png')
PY
