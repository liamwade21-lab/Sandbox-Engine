# The port's hero models (the vegetation and civilizations plan, steps 8-9), modelled in Blender by
# script: level 1's temple and watchtower, level 3's landmark tower. Run it in Blender (Scripting tab,
# or the Blender tool's execute_blender_code as exec(open(P).read(), {"__file__": P})): it builds them in the collections SE_temple,
# SE_watchtower and SE_landmark and writes NAME.bin beside this file. After changing a model, update
# its real size in city.wesl (TEMPLE_SIZE, WATCH_SIZE, LANDMARK_SIZE: the export prints it).
#
# .bin: "SEHM", version 1 (u32), the real width, height and depth (f32, m), the vertex and index
# counts (u32); per vertex its position in a unit box (x and z -0.5..0.5, y 0..1, Y up as the port's),
# its normal, and uv in metres by box projection with the texture layer in v's ten-thousands (v is
# layer * 10000 + 5000 + metres: negative metres never fall into the layer below); then the indices
# (u32). Layers: the villages' array 0 thatch, 1 mud brick, 2 timber, 3 stone; level 3's 0
# glass, 1 panels, 2 gold fins, and the hero's own 4 metal, 5 light strip (scatter_draw.wesl
# `hero_surface`).
import bpy, bmesh, math, os, struct
from mathutils import Vector, Matrix

HERE = os.path.dirname(os.path.abspath(__file__)) if "__file__" in globals() else os.getcwd()


def coll(name):
    c = bpy.data.collections.get(name)
    if c is None:
        c = bpy.data.collections.new(name)
        bpy.context.scene.collection.children.link(c)
    return c


def clear(name):
    for ob in list(coll(name).objects):
        bpy.data.objects.remove(ob, do_unlink=True)


def new_obj(name, bm, collection, layer):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name, me)
    ob["se_layer"] = layer
    coll(collection).objects.link(ob)
    return ob


def box(name, collection, layer, centre, size, rot_z=0.0):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    m = Matrix.Translation(Vector(centre)) @ Matrix.Rotation(rot_z, 4, 'Z') @ Matrix.Diagonal((size[0], size[1], size[2], 1.0))
    bmesh.ops.transform(bm, matrix=m, verts=bm.verts)
    return new_obj(name, bm, collection, layer)


def cone(name, collection, layer, centre, r1, r2, depth, segs, rot_z=0.0, smooth=False):
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segs, radius1=r1, radius2=r2, depth=depth)
    m = Matrix.Translation(Vector(centre) + Vector((0, 0, depth / 2))) @ Matrix.Rotation(rot_z, 4, 'Z')
    bmesh.ops.transform(bm, matrix=m, verts=bm.verts)
    for f in bm.faces:
        f.smooth = smooth
    return new_obj(name, bm, collection, layer)


def export(collection, path):
    dg = bpy.context.evaluated_depsgraph_get()
    tris = []
    for ob in coll(collection).objects:
        if ob.type != 'MESH':
            continue
        layer = int(ob.get("se_layer", 0))
        ev = ob.evaluated_get(dg)
        me = ev.to_mesh()
        me.calc_loop_triangles()
        nm = ob.matrix_world.to_3x3().inverted().transposed()
        cn = me.corner_normals
        for lt in me.loop_triangles:
            fn = (nm @ lt.normal).normalized()
            ax = max(range(3), key=lambda i: abs(fn[i]))
            corner = []
            for li in lt.loops:
                p = ob.matrix_world @ me.vertices[me.loops[li].vertex_index].co
                n = (nm @ Vector(cn[li].vector)).normalized()
                uv = (p.x, p.y) if ax == 2 else ((p.y, p.z) if ax == 0 else (p.x, p.z))
                corner.append(((p.x, p.z, -p.y), (n.x, n.z, -n.y), (uv[0], uv[1] + 5000.0 + layer * 10000.0)))
            tris.append(corner)
        ev.to_mesh_clear()
    xs = [c[0][0] for t in tris for c in t]
    ys = [c[0][1] for t in tris for c in t]
    zs = [c[0][2] for t in tris for c in t]
    W, H, D = max(xs) - min(xs), max(ys) - min(ys), max(zs) - min(zs)
    cx, cz, y0 = 0.5 * (max(xs) + min(xs)), 0.5 * (max(zs) + min(zs)), min(ys)
    out = bytearray(b"SEHM") + struct.pack("<I3f2I", 1, W, H, D, 3 * len(tris), 3 * len(tris))
    for t in tris:
        for (p, n, uv) in t:
            out += struct.pack("<8f", (p[0] - cx) / W, (p[1] - y0) / H, (p[2] - cz) / D, n[0], n[1], n[2], uv[0], uv[1])
    out += struct.pack("<%dI" % (3 * len(tris)), *range(3 * len(tris)))
    with open(path, "wb") as f:
        f.write(out)
    return (len(tris), round(W, 2), round(H, 2), round(D, 2))


def temple():
    T = "SE_temple"
    clear(T)
    ox = -20.0
    box("t_tier1", T, 3, (ox, 0, 0.6), (22, 22, 1.2))
    box("t_tier2", T, 3, (ox, 1, 1.8), (17, 17, 1.2))
    box("t_tier3", T, 3, (ox, 2, 3.0), (12.5, 12.5, 1.2))
    for i in range(6):
        d = 1.375
        box(f"t_step{i}", T, 3, (ox, -12.5 + (i + 0.5) * d, (i + 1) * 0.3), (4, d, (i + 1) * 0.6))
    box("t_walls", T, 1, (ox, 2, 3.6 + 2.5), (8, 8, 5))
    box("t_door", T, 2, (ox, 2 - 4.05, 3.6 + 1.5), (1.8, 0.25, 3.0))
    for sx in (-1, 1):
        for sy in (-1, 1):
            box(f"t_post{sx}{sy}", T, 2, (ox + sx * 4.6, 2 + sy * 4.6, 3.6 + 2.6), (0.55, 0.55, 5.2))
    cone("t_roof", T, 0, (ox, 2, 8.6), 7.9, 0.0, 5.6, 4, rot_z=math.pi / 4)
    cone("t_eave", T, 0, (ox, 2, 8.3), 7.9, 7.9, 0.3, 4, rot_z=math.pi / 4)
    cone("t_finial", T, 2, (ox, 2, 14.2), 0.35, 0.0, 1.6, 6)


def watchtower():
    W = "SE_watchtower"
    clear(W)
    ox = 20.0
    for sx in (-1, 1):
        for sy in (-1, 1):
            box(f"w_post{sx}{sy}", W, 2, (ox + sx * 1.6, sy * 1.6, 4.6), (0.38, 0.38, 9.2))
            box(f"w_rpost{sx}{sy}", W, 2, (ox + sx * 2.0, sy * 2.0, 10.5), (0.22, 0.22, 1.6))
    for z in (2.6, 5.6):
        for s in (-1, 1):
            box(f"w_railx{z}{s}", W, 2, (ox, s * 1.6, z), (3.6, 0.2, 0.22))
            box(f"w_raily{z}{s}", W, 2, (ox + s * 1.6, 0, z), (0.2, 3.6, 0.22))
    box("w_floor", W, 2, (ox, 0, 9.05), (4.6, 4.6, 0.3))
    for s in (-1, 1):
        box(f"w_wallx{s}", W, 2, (ox, s * 2.2, 9.7), (4.6, 0.15, 1.0))
        box(f"w_wally{s}", W, 2, (ox + s * 2.2, 0, 9.7), (0.15, 4.6, 1.0))
    cone("w_roof", W, 0, (ox, 0, 11.2), 3.7, 0.0, 2.8, 4, rot_z=math.pi / 4)


def landmark():
    L = "SE_landmark"
    clear(L)
    oy = 400.0
    cone("l_core", L, 0, (0, oy, 0), 30.0, 20.0, 880.0, 20, smooth=True)
    for k in range(3):
        a = math.radians(90 + 120 * k)
        radial = Vector((math.cos(a), math.sin(a), 0.0))
        side = Vector((-math.sin(a), math.cos(a), 0.0))
        bm = bmesh.new()
        rings = []
        N = 14
        for i in range(N + 1):
            t = i / N
            z = 640.0 * t
            r = 130.0 * (1.0 - t) ** 1.6 + 10.0 * t
            hw = 24.0 - 10.0 * t
            c0 = Vector((0, oy, z)) + radial * r
            rings.append([bm.verts.new(c0 + radial * (sx * hw) + side * (sy * hw)) for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))])
        for i in range(N):
            for j in range(4):
                bm.faces.new((rings[i][j], rings[i][(j + 1) % 4], rings[i + 1][(j + 1) % 4], rings[i + 1][j]))
        bm.faces.new(rings[0][::-1])
        bm.faces.new(rings[-1])
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        new_obj(f"l_leg{k}", bm, L, 2)
    cone("l_deck", L, 0, (0, oy, 640), 78.0, 70.0, 26.0, 28, smooth=True)
    cone("l_deck_ring", L, 5, (0, oy, 650), 79.0, 79.0, 3.0, 28, smooth=True)
    cone("l_deck_roof", L, 4, (0, oy, 666), 70.0, 30.0, 10.0, 28, smooth=True)
    cone("l_deck2", L, 0, (0, oy, 800), 46.0, 40.0, 16.0, 24, smooth=True)
    cone("l_deck2_ring", L, 5, (0, oy, 806), 47.0, 47.0, 2.5, 24, smooth=True)
    cone("l_crown", L, 5, (0, oy, 866), 21.5, 21.5, 4.0, 20, smooth=True)
    cone("l_needle", L, 4, (0, oy, 880), 9.0, 0.0, 120.0, 8, smooth=True)


temple()
watchtower()
landmark()
for name in ("temple", "watchtower", "landmark"):
    print(name, export("SE_" + name, os.path.join(HERE, name + ".bin")))
