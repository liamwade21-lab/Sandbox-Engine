"""Summarise a Bevy chrome trace (trace_chrome feature): the spans that take the most time.

python -I .optimize/bench/trace_top.py TRACE.json [TOP] [RUN.log SEGMENT]
Prints, per span name, the total and per-frame time, the call count and the longest call. With a
port log and a `perf SEGMENT` name, only spans inside that segment count (its start and end are
taken from the log's "command: perf" lines, relative to the log's first line) and the frame count
is the segment's.
"""
import re, sys
from collections import defaultdict
from datetime import datetime

path = sys.argv[1]
top = int(sys.argv[2]) if len(sys.argv) > 2 else 40
t0 = t1 = None; seg_frames = None
if len(sys.argv) > 4:
    stamp = lambda l: datetime.fromisoformat(l.split()[0].rstrip("Z")[:26])
    lines = [l for l in open(sys.argv[3], encoding="utf-8", errors="replace") if l[:2] == "20"]
    first = stamp(lines[0])
    marks = [(stamp(l), l.split("command: perf ")[1].strip()) for l in lines if "command: perf " in l]
    for i, (t, n) in enumerate(marks):
        if n == sys.argv[4]:
            t0 = (t - first).total_seconds() * 1e6
            t1 = (marks[i + 1][0] - first).total_seconds() * 1e6 if i + 1 < len(marks) else 1e18
    m = [l for l in lines if f"perf: {sys.argv[4]}	frames " in l]
    if m:
        seg_frames = int(re.search(r"frames (\d+)", m[0]).group(1))

# Stream the trace (one event per line; files reach several GB).
ev = re.compile(r'"name":"((?:[^"\\]|\\.)*)","ph":"([BEX])","pid":\d+,"tid":(\d+),"ts":([\d.]+)(?:,"dur":([\d.]+))?')
stacks = defaultdict(list)
tot = defaultdict(float); cnt = defaultdict(int); mx = defaultdict(float)
def add(name, rel, dur):
    if t0 is not None and not (t0 <= rel <= t1):
        return
    tot[name] += dur; cnt[name] += 1; mx[name] = max(mx[name], dur)
with open(path, "r", encoding="utf-8", errors="replace") as f:
    for line in f:
        m = ev.search(line)
        if not m:
            continue
        name, ph, tid, ts = m.group(1), m.group(2), m.group(3), float(m.group(4))
        if t1 is not None and ts > t1 + 5e6:
            break
        if ph == "B":
            stacks[tid].append((name, ts))
        elif ph == "E":
            st = stacks[tid]
            if st:
                n, b = st.pop()
                add(n, b, ts - b)
        else:
            add(name, ts, float(m.group(5) or 0))

frames = seg_frames or max([cnt[k] for k in cnt if k.lower() in ("main", "schedule:main", "main app", "update")] + [1])
print(f"frames ~{frames}")
print(f"{'total ms':>10} {'ms/frame':>9} {'calls':>7} {'max ms':>8}  span")
for k in sorted(tot, key=lambda k: -tot[k])[:top]:
    print(f"{tot[k]/1e3:10.1f} {tot[k]/1e3/frames:9.3f} {cnt[k]:7d} {mx[k]/1e3:8.2f}  {k[:110]}")
