#!/usr/bin/env bash
# A dive from orbit, 30 degrees down at full boost: pictures at the heat's stages, a sheet of them, and the log's measure of how much the
# picture's brightness jumps. bash .optimize/bench/hero_dive.sh NAME "date ...; select Earth; surface LAT LON 400 -30 90" PHOTO [extra run args]
REPO="/c/Users/liamw/Desktop/SpaceEngine Rust Port Version 2"; cd "$REPO" || exit 1
N="$1"; P="$2"; PH="$3"; shift 3
S=.optimize/scratch/run/s7
C="timing frame; $P; wait 400; mods on viltrumite; photo $PH; wait 200; hero boom @/$N.csv; hero aim 0 -30; hero keys w+shift 30; wait 836"
FR=(840 900 960 1020 1080 1140 1200 1320 1440 1484)
for i in "${!FR[@]}"; do
  f="${FR[$i]}"
  C="$C; shot @/${N}_$i.png"
  if [ "$i" -lt 9 ]; then C="$C; wait $(( ${FR[$((i+1))]} - f - 1 ))"; fi
done
C="$C; wait 3"
bash .optimize/bench/hero_fx.sh "$N" "$C" "$@"
cd "$S" && python -I - "$N" <<'PY'
import sys
from PIL import Image
import numpy as np
n=sys.argv[1]
W,H=640,360
sheet=Image.new('RGB',(W*3,H*4))
for i in range(10):
    try:
        im=Image.open(f'{n}_{i}.png').convert('RGB')
    except Exception as e:
        print('missing',i); continue
    a=np.asarray(im,dtype=float)/255
    print(i,'mean',round(a.mean(),3),'p95',round(float(np.percentile(a,95)),3))
    sheet.paste(im.resize((W,H)),((i%3)*W,(i//3)*H))
sheet.save(f'{n}_sheet.png')
PY
