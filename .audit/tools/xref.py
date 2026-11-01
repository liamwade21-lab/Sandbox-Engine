"""Find RIP-relative references to addresses in a PE's .text. Usage: xref.py EXE FUNCS_JSON ADDR..."""
import bisect, json, struct, sys
import numpy as np
exe, fj, targets = sys.argv[1], sys.argv[2], [int(a, 16) for a in sys.argv[3:]]
b = open(exe, "rb").read()
pe = struct.unpack_from("<I", b, 0x3c)[0]
nsec = struct.unpack_from("<H", b, pe + 6)[0]
optsz = struct.unpack_from("<H", b, pe + 20)[0]
base = struct.unpack_from("<Q", b, pe + 24 + 24)[0]
funcs = sorted((int(f["address"], 16), f["size"]) for f in json.load(open(fj)))
starts = [f[0] for f in funcs]
for i in range(nsec):
    o = pe + 24 + optsz + i * 40
    name = b[o:o + 8].rstrip(b"\0").decode()
    vsz, va, rsz, rptr = struct.unpack_from("<IIII", b, o + 8)
    if name != ".text":
        continue
    text = b[rptr:rptr + rsz]
    for t in targets:
        hits = set()
        a = np.frombuffer(text, np.uint8).astype(np.int64)
        disp = (a[:-3] | a[1:-2] << 8 | a[2:-1] << 16 | a[3:] << 24)
        disp = np.where(disp >= 2**31, disp - 2**32, disp)
        pos = np.arange(len(disp), dtype=np.int64) + base + va
        for extra in (0, 1, 4):  # instruction may end 0, 1 or 4 bytes after the displacement
            for j in np.nonzero(pos + 4 + extra + disp == t)[0]:
                at = int(pos[j])
                k = bisect.bisect_right(starts, at) - 1
                f = funcs[k] if k >= 0 else (0, 0)
                inside = f[0] <= at < f[0] + f[1]
                hits.add((f"FUN_{f[0]:x}" if inside else "?", hex(at)))
        print(hex(t), sorted(hits))
