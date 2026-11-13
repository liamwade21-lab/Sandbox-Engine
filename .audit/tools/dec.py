"""Decompile functions with the ghidra CLI into scratchpad/re/<name>.c. Usage: python -I dec.py FUN_x [FUN_y ...]"""
import json, subprocess, sys, os

REPO = r"C:\Users\liamw\Desktop\Sandbox Engine"
OUT = os.path.join(REPO, ".audit", "work", "dec")
os.makedirs(OUT, exist_ok=True)
for f in sys.argv[1:]:
    r = subprocess.run(["ghidra", "decompile", f, "--projects-dir", "re/ghidra_projects", "--project", "se"],
                       cwd=REPO, capture_output=True, text=True, encoding="utf-8", errors="replace")
    txt = r.stdout.strip()
    try:
        j = json.loads(txt)
        if isinstance(j, list):
            j = j[0]
        code = j.get("code", txt)
    except Exception:
        code = txt + r.stderr
    p = os.path.join(OUT, f + ".c")
    with open(p, "w", encoding="utf-8") as fh:
        fh.write(code)
    print(p, code.count("\n"), "lines")
