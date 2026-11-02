"""Best (lowest) and median-of-runs per segment for the ab.sh output: python -I ab_table.py DIR."""
import sys, os, re, glob, statistics as st
d = sys.argv[1]; data = {}
for f in glob.glob(os.path.join(d, "*", "*.perf")):
    side = re.match(r"(base|new)\d+", os.path.basename(os.path.dirname(f))).group(1)
    for line in open(f, encoding="utf-8"):
        seg = line.split("\t")[0]
        g = lambda k: float(re.search(k + r" ([\d.]+)", line).group(1))
        data.setdefault(seg, {}).setdefault(side, []).append((g("median"), g("mean"), g("p99"), g("max"), g("1% low")))
print(f"{'segment':12} {'runs':>4}  {'median: best base/new':>22}  {'median: typical base/new':>25}  {'p99 typical base/new':>21}  {'max typical base/new':>21}")
for seg in sorted(data):
    b = data[seg].get("base", []); n = data[seg].get("new", [])
    if not b or not n: continue
    col = lambda xs, i, f: f([x[i] for x in xs])
    print(f"{seg:12} {len(b):>4}  {col(b,0,min):10.2f} {col(n,0,min):10.2f}  {col(b,0,st.median):12.2f} {col(n,0,st.median):11.2f}  {col(b,2,st.median):10.2f} {col(n,2,st.median):9.2f}  {col(b,3,st.median):10.2f} {col(n,3,st.median):9.2f}")
