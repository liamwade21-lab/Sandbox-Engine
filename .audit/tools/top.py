"""List kept gaps at or above a visibility: id, vis, effort, status, feature start. Usage: python -I .audit/tools/top.py [MINVIS] [WIDTH]"""
import glob, os, re, sys

AUD = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
minvis = int(sys.argv[1]) if len(sys.argv) > 1 else 4
width = int(sys.argv[2]) if len(sys.argv) > 2 else 150
gaps, checks = {}, {}
for p in glob.glob(os.path.join(AUD, "gaps", "*.txt")):
    for line in open(p, encoding="utf-8"):
        f = [x.strip() for x in line.split(" | ")]
        if len(f) >= 7 and re.match(r"^\d\d-\d{3}$", f[0]):
            gaps[f[0]] = f
for p in glob.glob(os.path.join(AUD, "checks", "*.txt")):
    for line in open(p, encoding="utf-8"):
        f = [x.strip() for x in line.split(" | ")]
        if len(f) >= 5 and f[0] in gaps:
            checks[f[0]] = f
rows = []
for gid, g in gaps.items():
    c = checks.get(gid)
    if not c or c[1].upper().startswith("REFUTED"):
        continue
    vis = int(re.search(r"\d", c[3]).group(0)) if re.search(r"\d", c[3]) else int(g[1])
    eff = c[2].replace("effort", "").strip()
    feat = c[5] if len(c) > 5 and c[5] not in ("-", "") else g[2]
    rows.append((vis, gid, eff, g[5], feat))
for r in sorted(rows, key=lambda r: (-r[0], r[1])):
    if r[0] >= minvis:
        print(f"{r[1]} v{r[0]} {r[2]} {r[3][:7]:7} {r[4][:width]}")
