"""python -I freeze.py SRC_SHADERS DST_SHADERS: copy the shaders with every clock the pictures see stopped, so two runs can be compared bit for bit."""
import sys, os, shutil, re
src, dst = sys.argv[1], sys.argv[2]
shutil.rmtree(dst, ignore_errors=True); shutil.copytree(src, dst)
def patch(name, pairs):
    p = os.path.join(dst, name); b = open(p, "rb").read()
    for a, c in pairs:
        assert a in b, (name, a)
        b = b.replace(a, c)
    open(p, "wb").write(b)
patch("scatter_draw.wesl", [(b"body.lod2.z", b"1.0")])
patch("scatter.wesl", [(b"let t = body.street.y;", b"let t = 1.0;")])
patch("planet.wesl", [(b"globals.time", b"1.0")])
