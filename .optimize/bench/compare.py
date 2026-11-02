"""Compare two folders of reference screenshots: python -I compare.py DIR_A DIR_B
Per image: mean absolute difference (0-255), the share of pixels off by more than 8, PSNR."""
import sys, os, math
from PIL import Image, ImageChops
a, b = sys.argv[1], sys.argv[2]
for n in sorted(f for f in os.listdir(a) if f.endswith(".png")):
    if not os.path.exists(os.path.join(b, n)):
        print(f"{n:16} missing in {b}"); continue
    ia = Image.open(os.path.join(a, n)).convert("RGB"); ib = Image.open(os.path.join(b, n)).convert("RGB")
    if ia.size != ib.size:
        print(f"{n:16} size differs"); continue
    d = ImageChops.difference(ia, ib).convert("L")
    h = d.histogram(); px = sum(h)
    mean = sum(i * c for i, c in enumerate(h)) / px
    over = sum(h[9:]) / px * 100
    mse = sum(i * i * c for i, c in enumerate(h)) / px
    psnr = 99.0 if mse == 0 else 10 * math.log10(255 * 255 / mse)
    print(f"{n:16} mean {mean:6.3f}  >8: {over:6.3f}%  psnr {psnr:5.1f} dB")
