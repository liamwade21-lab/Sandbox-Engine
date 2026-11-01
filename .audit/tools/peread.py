"""Read floats/doubles at virtual addresses from a PE file. Usage: peread.py EXE f|d ADDR..."""
import struct, sys
exe, kind, addrs = sys.argv[1], sys.argv[2], sys.argv[3:]
b = open(exe, "rb").read()
pe = struct.unpack_from("<I", b, 0x3c)[0]
nsec = struct.unpack_from("<H", b, pe + 6)[0]
optsz = struct.unpack_from("<H", b, pe + 20)[0]
base = struct.unpack_from("<Q", b, pe + 24 + 24)[0]
secs = []
for i in range(nsec):
    o = pe + 24 + optsz + i * 40
    name = b[o:o + 8].rstrip(b"\0").decode(errors="replace")
    vsz, va, rsz, rptr = struct.unpack_from("<IIII", b, o + 8)
    secs.append((name, va, vsz, rptr, rsz))
for a in addrs:
    rva = int(a, 16) - base
    for name, va, vsz, rptr, rsz in secs:
        if va <= rva < va + vsz:
            off = rptr + rva - va
            if rva - va >= rsz:
                print(a, name, "bss (zero-init)")
                break
            v = struct.unpack_from("<f" if kind == "f" else "<d", b, off)[0]
            print(a, name, v)
            break
    else:
        print(a, "not mapped")
