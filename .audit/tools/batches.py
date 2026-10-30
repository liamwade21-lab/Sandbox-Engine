"""Split .audit/gaps/<ticket>.txt into checker batches of about 5 gaps.
Usage: python -I .audit/tools/batches.py 02 03 08  -> writes .audit/checks/batches/<ticket>-b<k>.txt, prints names."""
import math, os, sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
OUT = os.path.join(ROOT, "checks", "batches")
os.makedirs(OUT, exist_ok=True)
for t in sys.argv[1:]:
    with open(os.path.join(ROOT, "gaps", t + ".txt"), encoding="utf-8") as fh:
        lines = [l.rstrip("\n") for l in fh if l.strip() and not l.startswith("TICKET")]
    n = max(1, round(len(lines) / 5))
    size = math.ceil(len(lines) / n)
    for k in range(n):
        part = lines[k * size:(k + 1) * size]
        if not part:
            continue
        name = f"{t}-b{k + 1}"
        with open(os.path.join(OUT, name + ".txt"), "w", encoding="utf-8") as fh:
            fh.write("\n".join(part) + "\n")
        print(name, len(part))
