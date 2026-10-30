"""Merge .audit/gaps/*.txt with the checkers' verdicts in .audit/checks/*.txt into docs/missing_features.md.
Usage: python -I .audit/tools/compile.py [--check]   (--check only reports missing verdicts)"""
import glob, os, re, sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
AUD = os.path.join(ROOT, ".audit")
TICKETS = {
    "01": "Stars", "02": "Star clusters", "03": "Galaxies and nebulae", "04": "Planet surfaces",
    "05": "Atmospheres, rings, comets", "06": "Procedural generation", "07": "Camera and time",
    "08": "Interface", "09": "Graphics and settings", "10": "Scripts, audio, spacecraft",
}


def fields(line):
    return [f.strip() for f in line.split(" | ")]


gaps = {}
for p in sorted(glob.glob(os.path.join(AUD, "gaps", "*.txt"))):
    for line in open(p, encoding="utf-8"):
        f = fields(line.rstrip("\n"))
        if len(f) < 7 or not re.match(r"^\d\d-\d{3}$", f[0]):
            continue
        gaps[f[0]] = dict(id=f[0], vis=int(f[1]), feature=f[2], se=f[3], port=f[4], status=f[5], conf=f[6])

checks = {}
for p in sorted(glob.glob(os.path.join(AUD, "checks", "*.txt"))):
    for line in open(p, encoding="utf-8"):
        f = fields(line.rstrip("\n"))
        if len(f) < 5 or f[0] not in gaps:
            continue
        verdict = f[1].upper()
        eff = re.search(r"[SML]\b", f[2].replace("effort", "").strip())
        vis = re.search(r"\d", f[3])
        checks[f[0]] = dict(verdict=verdict, effort=eff.group(0) if eff else "?", vis=int(vis.group(0)) if vis else None,
                            reason=f[4], amended=f[5] if len(f) > 5 and f[5] not in ("-", "") else None)

missing = [g for g in gaps if g not in checks]

# Gaps closed in the port since the audit: `.audit/fixed.txt`, `ID | date | what was done`.
fixed = {}
fixed_path = os.path.join(AUD, "fixed.txt")
if os.path.exists(fixed_path):
    for line in open(fixed_path, encoding="utf-8"):
        f = fields(line.rstrip("\n"))
        if len(f) >= 3 and f[0] in gaps:
            fixed[f[0]] = (f[1], f[2])
# Gaps the owner deferred: `.audit/deferred.txt`, `ID | date | what`. Still open, listed apart, not counted.
deferred = {}
deferred_path = os.path.join(AUD, "deferred.txt")
if os.path.exists(deferred_path):
    for line in open(deferred_path, encoding="utf-8"):
        f = fields(line.rstrip("\n"))
        # `08-*` defers a whole area.
        ids = [g for g in gaps if g.startswith(f[0][:-1])] if f and f[0].endswith("*") else f[:1]
        for gid in ids if len(f) >= 2 else []:
            if gid in gaps:
                deferred.setdefault(gid, f[1])
if "--check" in sys.argv:
    print(f"{len(gaps)} gaps, {len(checks)} verdicts, {len(missing)} missing (tickets {sorted({m[:2] for m in missing})})")
    sys.exit(0)

kept, refuted = [], []
for gid, g in gaps.items():
    c = checks.get(gid)
    if c is None:
        continue
    if c["verdict"].startswith("REFUTED"):
        refuted.append((g, c))
        continue
    g = dict(g)
    g["verdict"], g["effort"], g["reason"] = c["verdict"], c["effort"], c["reason"]
    if c["vis"]:
        g["vis"] = c["vis"]
    if c["amended"]:
        g["feature"] = c["amended"]
    kept.append(g)

order = {"S": 0, "M": 1, "L": 2, "?": 3}
kept.sort(key=lambda g: (-g["vis"], g["id"][:2], order.get(g["effort"], 3), g["id"]))
done = [g for g in kept if g["id"] in fixed]
kept = [g for g in kept if g["id"] not in fixed]
held = [g for g in kept if g["id"] in deferred]
kept = [g for g in kept if g["id"] not in deferred]


def md(s):
    return s.replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&").replace("|", "/")


out = []
w = out.append
intro = open(os.path.join(AUD, "work", "main", "doc_intro.md"), encoding="utf-8").read()
w(intro.rstrip() + "\n")
counts = {v: sum(1 for g in kept if g["vis"] == v) for v in range(5, 0, -1)}
eff = {e: sum(1 for g in kept if g["effort"] == e) for e in "SML"}
w(f"\n## Summary\n\n{len(gaps)} gaps reported by the auditors, {len(kept) + len(done)} kept after checking "
  f"({sum(1 for g in kept + done if g['verdict'].startswith('AMENDED'))} of them amended), {len(refuted)} refuted. "
  f"{len(done)} have been fixed since (listed at the end); the tables count the {len(kept)} still open. "
  f"{len(held)} more are open but deferred by the owner (listed after them, not counted).\n")
w("| Visibility | 5 | 4 | 3 | 2 | 1 |\n|---|---|---|---|---|---|")
w("| Gaps | " + " | ".join(str(counts[v]) for v in range(5, 0, -1)) + " |\n")
w("| Effort | S | M | L |\n|---|---|---|---|")
w(f"| Gaps | {eff['S']} | {eff['M']} | {eff['L']} |\n")
w("| Area | Kept |\n|---|---|")
for t, name in TICKETS.items():
    w(f"| {t} {name} | {sum(1 for g in kept if g['id'][:2] == t)} |")
w("\nVisibility: 5 = obvious in the first minutes with default settings, 4 = common activities, 3 = a specific "
  "common feature, 2 = niche, 1 = obscure. Effort: S about a day, M a few days, L a week or more or further "
  "reverse engineering. Status: missing / partial / differs. IDs are `<area>-<n>`; the raw auditor and checker "
  "lines are in `.audit/gaps/` and `.audit/checks/`.\n")

names = {5: "Visibility 5: obvious at once", 4: "Visibility 4: common activities", 3: "Visibility 3: specific features",
         2: "Visibility 2: niche", 1: "Visibility 1: obscure"}
cur = None
for g in kept:
    if g["vis"] != cur:
        cur = g["vis"]
        w(f"\n## {names[cur]}\n")
    w(f"### {g['id']} · {TICKETS[g['id'][:2]]}\n")
    w(f"{md(g['feature'])}\n")
    w(f"- Status: {g['status']}, effort **{g['effort']}**, auditor confidence {g['conf']}"
      + (", amended by the checker" if g["verdict"].startswith("AMENDED") else ""))
    w(f"- SE evidence: {md(g['se'])}")
    w(f"- Port: {md(g['port'])}")
    w(f"- Check: {md(g['reason'])}\n")

w("\n## Deferred by the owner\n")
w("Still open; left out of the work for now and of the counts above.\n")
for g in sorted(held, key=lambda g: g["id"]):
    w(f"### {g['id']} · {TICKETS[g['id'][:2]]} (deferred {deferred[g['id']]})\n")
    w(f"{md(g['feature'])}\n")
    w(f"- Status: {g['status']}, effort **{g['effort']}**, auditor confidence {g['conf']}")
    w(f"- SE evidence: {md(g['se'])}")
    w(f"- Port: {md(g['port'])}\n")

w("\n## Fixed since the audit\n")
for g in sorted(done, key=lambda g: g["id"]):
    date, what = fixed[g["id"]]
    w(f"- **{g['id']}** ({TICKETS[g['id'][:2]]}, fixed {date}): {md(g['feature'])[:110]}... Fix: {md(what)}")

w("\n## Refuted by the checkers\n")
w("Reported by an auditor, then disproved; listed so they are not re-reported.\n")
for g, c in sorted(refuted, key=lambda x: x[0]["id"]):
    w(f"- **{g['id']}** {md(g['feature'])[:140]}... Reason: {md(c['reason'])}")

open(os.path.join(ROOT, "docs", "missing_features.md"), "w", encoding="utf-8").write("\n".join(out) + "\n")
print(f"kept {len(kept)}, refuted {len(refuted)}, unchecked {len(missing)}")
