"""String -> referencing functions. Usage: python -I gx.py Substring [Substring ...] (exact value match preferred)"""
import json, subprocess, sys

REPO = r"C:\Users\liamw\Desktop\Space Engine Recomp"
P = ["--projects-dir", "re/ghidra_projects", "--project", "se"]


def run(args):
    r = subprocess.run(["ghidra"] + args + P, cwd=REPO, capture_output=True, text=True, encoding="utf-8", errors="replace")
    try:
        return json.loads(r.stdout)
    except Exception:
        print("  raw:", (r.stdout + r.stderr)[:300])
        return []


for s in sys.argv[1:]:
    strs = run(["strings", "list", "--filter", "value~" + s])
    exact = [x for x in strs if x["value"].strip() == s] or strs[:4]
    for x in exact:
        refs = run(["x-ref", "to", x["address"]])
        fns = sorted(set(f"{r['from_function']}@{r['from']}" for r in refs))
        print(f"{s!r} {x['address']} {x['value'][:50]!r}: {fns[:8]}")
