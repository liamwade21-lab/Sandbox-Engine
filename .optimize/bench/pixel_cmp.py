"""python -I cmp.py A B: per image, pixels that differ at all, mean abs difference, max, and where (bounding box)."""
import sys, os
import numpy as np
from PIL import Image
a, b = sys.argv[1], sys.argv[2]
for n in sorted(f for f in os.listdir(a) if f.endswith(".png")):
    p = os.path.join(b, n)
    if not os.path.exists(p):
        print(f"{n:18} missing"); continue
    x = np.asarray(Image.open(os.path.join(a, n)).convert("RGB")).astype(int)
    y = np.asarray(Image.open(p).convert("RGB")).astype(int)
    if x.shape != y.shape:
        print(f"{n:18} size differs"); continue
    d = np.abs(x - y).max(axis=2)
    ys, xs = np.nonzero(d)
    box = f"x {xs.min()}-{xs.max()} y {ys.min()}-{ys.max()}" if len(xs) else ""
    print(f"{n:18} differ {100*np.mean(d>0):7.3f}%  >2 {100*np.mean(d>2):7.3f}%  >8 {100*np.mean(d>8):7.3f}%  mean {np.abs(x-y).mean():7.4f}  max {d.max():3d}  {box}")
