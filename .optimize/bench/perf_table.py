"""Table of base vs new frame times: python -I perf_table.py DIR (holds base1 base2 new1 new2)."""
import sys, os, re, glob, statistics as st
d = sys.argv[1]
def load(run):
    out = {}
    for f in glob.glob(os.path.join(d, run, "*.perf")):
        for line in open(f, encoding="utf-8"):
            parts = line.rstrip("\n").split("\t")
            g = lambda k: float(re.search(k + r" ([\d.]+)", line).group(1))
            out[parts[0]] = dict(mean=g("mean"), median=g("median"), p99=g("p99"), max=g("max"), low=g("1% low"))
        m = f[:-5] + ".mem"
        if os.path.exists(m):
            out.setdefault("_mem", {})[os.path.basename(f)[:-5]] = int(open(m).read().strip() or 0)
    return out
runs = {r: load(r) for r in ("base1", "base2", "new1", "new2")}
segs = sorted({k for r in runs.values() for k in r if not k.startswith("_")})
print(f"{'segment':12} {'median base':>12} {'median new':>11} {'mean base':>10} {'mean new':>9} {'p99 base':>9} {'p99 new':>8} {'max base':>9} {'max new':>8} {'1%low b':>8} {'1%low n':>8}")
for s in segs:
    v = lambda side, k: [runs[r][s][k] for r in (side + "1", side + "2") if s in runs[r]]
    f = lambda xs: f"{st.mean(xs):.2f}" if xs else "-"
    print(f"{s:12} {f(v('base','median')):>12} {f(v('new','median')):>11} {f(v('base','mean')):>10} {f(v('new','mean')):>9} {f(v('base','p99')):>9} {f(v('new','p99')):>8} {f(v('base','max')):>9} {f(v('new','max')):>8} {f(v('base','low')):>8} {f(v('new','low')):>8}")
for side in ("base", "new"):
    mems = {}
    for r in (side + "1", side + "2"):
        for k, v in runs[r].get("_mem", {}).items():
            mems.setdefault(k, []).append(v)
    print(side, "peak MB:", " ".join(f"{k} {max(v)}" for k, v in sorted(mems.items())))
