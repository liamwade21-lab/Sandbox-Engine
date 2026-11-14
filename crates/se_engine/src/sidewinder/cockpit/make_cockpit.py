"""Build the Sidewinder's cockpit (recreated, grade E) from the user's screenshots, export cockpit.obj, render the three views.

Run (background only, never the user's open Blender):
  "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe" --background --factory-startup --python make_cockpit.py
Then: python -I compare.py   (writes compare_1.png .. compare_3.png)

Space: metres, X right, Y up, -Z forward, origin at the pilot's eye. The camera of each screenshot comes from calib3.json
(calib3.py: one focal length for all three, each view's head turn and lean fitted to HUD points seen in two views). Points
the screenshots show are placed on the ray of the pixel they were read from (view 1 unless said), at a depth chosen to make
a cockpit that holds together (only views 2 and 3, whose eye has moved a few cm, see depth at all).
Blender's world is Z up: a point (x, y, z) here is (x, -z, y) in Blender; the OBJ is written Y up, -Z forward, so it is
in this file's space again.
"""
import json, math, os, sys
import bpy, bmesh
from mathutils import Vector, Matrix

HERE = os.path.dirname(os.path.abspath(__file__))
CAL = json.load(open(os.path.join(HERE, "calib3.json")))
F = CAL["f_px"]
W_PX, H_PX, CY = 2000, 821, 410.5


# ---------------------------------------------------------------- cameras and rays (the same model as calib.py)
def rot(yaw_deg, pitch_deg):
    y, p = math.radians(yaw_deg), math.radians(pitch_deg)
    ry = Matrix(((math.cos(y), 0, math.sin(y)), (0, 1, 0), (-math.sin(y), 0, math.cos(y))))
    rx = Matrix(((1, 0, 0), (0, math.cos(p), -math.sin(p)), (0, math.sin(p), math.cos(p))))
    return ry @ rx


def cam(view):
    yaw, pitch, t = CAL["views"][str(view)]
    return rot(yaw, pitch), Vector(t)


def ray(view, px):
    R, t = cam(view)
    d = R @ Vector(((px[0] - 1000) / F, -(px[1] - CY) / F, -1.0))
    return t, d.normalized()


def at_z(view, px, z):
    o, d = ray(view, px)
    return o + d * ((z - o.z) / d.z)


def at_x(view, px, x):
    o, d = ray(view, px)
    return o + d * ((x - o.x) / d.x)


V = Vector


# ---------------------------------------------------------------- the frame's key points
# Read off the screenshots (pixels), see notes.md. View 1 is straight ahead.
CROSS_Z = -1.9                       # the crossbar (triangulated from views 1 and 2: about -2.0)
J_L = at_z(1, (815, 258), CROSS_Z)   # where the crossbar, the lower beam and the rising beam meet, left
J_R = at_z(1, (1255, 258), CROSS_Z)
CROSSBAR = [J_L] + [at_z(1, p, CROSS_Z) for p in [(900, 251), (1000, 249), (1100, 251), (1180, 253)]] + [J_R]
KNEE_L = at_x(1, (590, 376), -0.58)  # lower beam meets the side rail (triangulated: (-0.58, 0.06, -1.06))
KNEE_R = at_x(1, (1335, 362), 0.60)
RAIL_X = 0.60
BACK_Z = 0.45                        # the bulkhead behind the seat


def lerp_path(view, pxs, a, b):
    """Points on the rays of pxs whose depth runs linearly (by pixel length) from point a to point b."""
    lens = [0.0]
    for i in range(1, len(pxs)):
        lens.append(lens[-1] + math.dist(pxs[i - 1], pxs[i]))
    out = []
    for p, l in zip(pxs, lens):
        s = l / lens[-1]
        out.append(at_z(view, p, a.z + (b.z - a.z) * s))
    out[0], out[-1] = a, b
    return out


LOWER_L = lerp_path(1, [(815, 268), (740, 318), (660, 365), (590, 376)], J_L, KNEE_L)
LOWER_R = lerp_path(1, [(1255, 268), (1295, 312), (1335, 362)], J_R, KNEE_R)
RAIL_L = [KNEE_L, at_x(1, (300, 402), -RAIL_X), at_x(2, (300, 312), -RAIL_X), at_x(2, (0, 302), -RAIL_X), V((-RAIL_X, 0.05, BACK_Z))]
RAIL_R = [KNEE_R, at_x(1, (1600, 420), RAIL_X), at_x(1, (1990, 438), RAIL_X), at_x(3, (1500, 350), RAIL_X),
          at_x(3, (1990, 342), RAIL_X), V((RAIL_X, 0.05, BACK_Z))]
# The rising beam: from the junction up and back to the roof (view 1: (1255,245) -> (1370,115) -> (1445,25), seen dark
# against the planet behind the INFO panel; view 3: (800,170) -> (950,40)). The left one is its mirror (dark on dark space).
TOP_Y, TOP_X, ARCH_Z = 0.78, 0.56, -0.62
RISE_R = [J_R + V((0, 0.03, 0)), at_x(1, (1300, 195), 0.56), at_x(1, (1370, 115), 0.60), at_x(1, (1445, 25), 0.62),
          V((0.60, 0.70, -0.78)), V((TOP_X, TOP_Y, ARCH_Z))]
RISE_L = [V((-p.x, p.y, p.z)) for p in RISE_R]
RISE_L[0] = J_L + V((0, 0.03, 0))
# The arch overhead, side to side, just in front of the head.
ARCH = [V((TOP_X, TOP_Y, ARCH_Z)), V((0.40, 0.86, ARCH_Z)), V((0.0, 0.90, ARCH_Z)), V((-0.40, 0.86, ARCH_Z)), V((-TOP_X, TOP_Y, ARCH_Z))]
ROOF_R = [V((TOP_X, TOP_Y, ARCH_Z)), V((0.55, 0.80, -0.1)), V((0.52, 0.78, BACK_Z))]
ROOF_L = [V((-p.x, p.y, p.z)) for p in ROOF_R]

# The dashboard's top edge (the coaming, view 1's straight dark line at y = 576 across the whole width) and the sills: kept on
# the plane through the eye that this line is, so it lines up whatever the depth.
COAM_Z = -1.10
SILL_X = 0.62


def coaming(x, z):
    return at_z(1, (1000, 576), z).y * 1.0 + 0 * x


def coam_pt(x, z):
    return V((x, at_z(1, (1000, 576), z).y, z))


COAMING = [coam_pt(-SILL_X, -0.50), coam_pt(-SILL_X, -0.80), coam_pt(-0.52, COAM_Z), coam_pt(0.52, COAM_Z), coam_pt(SILL_X, -0.80), coam_pt(SILL_X, -0.50)]
SILL_Y_BACK = coam_pt(SILL_X, -0.50).y
SILL_L = [coam_pt(-SILL_X, -0.80), coam_pt(-SILL_X, -0.50), V((-SILL_X, SILL_Y_BACK, BACK_Z))]
SILL_R = [coam_pt(SILL_X, -0.80), coam_pt(SILL_X, -0.50), V((SILL_X, SILL_Y_BACK, BACK_Z))]
GLASS_BASE_Z, GLASS_BASE_Y, GLASS_BASE_X = -2.10, -0.62, 0.78   # where the windscreen meets the nose, under the dash
FLOOR_Y = -1.15


# ---------------------------------------------------------------- mesh building (in this file's space)
class Part:
    def __init__(self, name):
        # facing: per face, the point it must face (a one-sided surface) or None (part of a closed solid: outward)
        self.name, self.v, self.f, self.facing, self.default_face = name, [], [], [], None

    def add(self, pts):
        base = len(self.v)
        self.v.extend(V(p) for p in pts)
        return list(range(base, base + len(pts)))

    def quad(self, a, b, c, d, face="default"):
        self.f.append([a, b, c, d]); self.facing.append(self.default_face if face == "default" else face)

    def poly(self, pts, face_in=None):
        """A polygon, fanned from its middle. face_in: a point the faces must face (their normal towards it)."""
        ids = self.add(pts)
        mid = self.add([sum((V(p) for p in pts), V()) / len(pts)])[0]
        for i in range(len(ids)):
            tri = [mid, ids[i], ids[(i + 1) % len(ids)]]
            if face_in is not None:
                a, b, c = (self.v[k] for k in tri)
                n = (b - a).cross(c - a)
                if n.dot(V(face_in) - (a + b + c) / 3) < 0:
                    tri = [tri[0], tri[2], tri[1]]
            self.f.append(tri); self.facing.append(None if face_in is None else V(face_in))

    def box(self, lo, hi):
        x0, y0, z0 = lo; x1, y1, z1 = hi
        i = self.add([(x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0), (x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)])
        for q in [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (2, 3, 7, 6), (1, 2, 6, 5), (0, 4, 7, 3)]:
            self.quad(*(i[k] for k in q), face=None)

    def solid(self, rings):
        """Close a list of rings (each the same number of points) into a tube with capped ends."""
        n = len(rings[0])
        ids = [self.add(r) for r in rings]
        for a, b in zip(ids, ids[1:]):
            for k in range(n):
                self.quad(a[k], a[(k + 1) % n], b[(k + 1) % n], b[k], face=None)
        self.f += [list(reversed(ids[0])), list(ids[-1])]; self.facing += [None, None]

    def cylinder(self, a, b, r, n=12, r2=None):
        a, b = V(a), V(b)
        ax = (b - a).normalized()
        u = ax.orthogonal().normalized(); w = ax.cross(u)
        ring = lambda c, rr: [c + (u * math.cos(2 * math.pi * k / n) + w * math.sin(2 * math.pi * k / n)) * rr for k in range(n)]
        self.solid([ring(a, r), ring(b, r if r2 is None else r2)])


def smooth(pts, sub=4):
    """Catmull-Rom through the points."""
    pts = [V(p) for p in pts]
    if len(pts) < 3:
        return pts
    out = []
    ext = [pts[0] * 2 - pts[1]] + pts + [pts[-1] * 2 - pts[-2]]
    for i in range(1, len(ext) - 2):
        p0, p1, p2, p3 = ext[i - 1], ext[i], ext[i + 1], ext[i + 2]
        for s in range(sub):
            t = s / sub
            out.append(0.5 * ((2 * p1) + (-p0 + p2) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t + (-p0 + 3 * p1 - 3 * p2 + p3) * t ** 3))
    out.append(pts[-1])
    return out


def beam(part, path, w, d, sub=4, bevel=0.25):
    """Sweep a bevelled rectangle (w across the line of sight, d along it) along a path."""
    pts = smooth(path, sub)
    rings = []
    for i, p in enumerate(pts):
        t = (pts[min(i + 1, len(pts) - 1)] - pts[max(i - 1, 0)]).normalized()
        r = p.normalized()
        s = t.cross(r).normalized()
        u = s.cross(t).normalized()
        b = bevel
        prof = [(-0.5, -0.5 + b), (-0.5 + b, -0.5), (0.5 - b, -0.5), (0.5, -0.5 + b), (0.5, 0.5 - b), (0.5 - b, 0.5), (-0.5 + b, 0.5), (-0.5, 0.5 - b)]
        rings.append([p + s * (a * w) + u * (c * d) for a, c in prof])
    part.solid(rings)


EYE = V((0, 0, 0))
frame, dash, console, seat, floor, glass = (Part(n) for n in ("frame", "dash", "console", "seat", "floor", "glass"))
dash.default_face = console.default_face = floor.default_face = EYE

# ---- the canopy frame
beam(frame, CROSSBAR, 0.095, 0.10)
beam(frame, LOWER_L, 0.05, 0.06); beam(frame, LOWER_R, 0.05, 0.06)
beam(frame, RAIL_L, 0.045, 0.05); beam(frame, RAIL_R, 0.045, 0.05)
beam(frame, RISE_L, 0.06, 0.07); beam(frame, RISE_R, 0.06, 0.07)
beam(frame, ARCH, 0.10, 0.09)
beam(frame, ROOF_L, 0.05, 0.05); beam(frame, ROOF_R, 0.05, 0.05)
# the windscreen's lower frame, under the dash, and the sill lips along the side windows
base_l, base_r = V((-GLASS_BASE_X, GLASS_BASE_Y, GLASS_BASE_Z)), V((GLASS_BASE_X, GLASS_BASE_Y, GLASS_BASE_Z))
beam(frame, [base_l, base_r], 0.06, 0.06, sub=1)
beam(frame, SILL_L, 0.03, 0.04, sub=1); beam(frame, SILL_R, 0.03, 0.04, sub=1)
# the bulkhead behind the seat, with the roof's back edge
frame.poly([(-SILL_X, FLOOR_Y, BACK_Z), (SILL_X, FLOOR_Y, BACK_Z), (SILL_X, SILL_Y_BACK, BACK_Z), (RAIL_X, 0.05, BACK_Z),
            (0.52, 0.78, BACK_Z), (0.0, 0.84, BACK_Z), (-0.52, 0.78, BACK_Z), (-RAIL_X, 0.05, BACK_Z), (-SILL_X, SILL_Y_BACK, BACK_Z)], EYE)

# ---- the glass, one-sided, facing in
glass.poly([base_l, base_r] + [J_R] + list(reversed(CROSSBAR[1:-1])) + [J_L], EYE)                       # windscreen
for s, lower, knee, rise, rail, sill, roof in ((1, LOWER_R, KNEE_R, RISE_R, RAIL_R, SILL_R, ROOF_R), (-1, LOWER_L, KNEE_L, RISE_L, RAIL_L, SILL_L, ROOF_L)):
    base = base_r if s > 0 else base_l
    glass.poly([base, sill[0], knee] + list(reversed(lower[:-1])), EYE)                                    # front quarter
    glass.poly([sill[0], sill[1], sill[2], rail[-1]] + list(reversed(rail[:-1])), EYE)                     # side, low
    glass.poly(list(lower) + rail[1:] + [roof[-1], roof[1]] + list(reversed(rise)), EYE)                   # side, high
glass.poly(CROSSBAR + RISE_R[1:] + list(reversed(ARCH[1:-1])) + list(reversed(RISE_L[1:])), EYE)           # roof, front
glass.poly([ARCH[0], ROOF_R[1], ROOF_R[2], V((0, 0.84, BACK_Z)), ROOF_L[2], ROOF_L[1], ARCH[-1]] + list(reversed(ARCH[1:-1])), EYE)  # roof, back

# ---- the dashboard: deck behind the coaming, the radar's well, the nose under the windscreen
c = COAMING
deck_y, well_y, face_z = c[2].y - 0.07, c[2].y - 0.12, -0.52
dl, dr = V((-0.52, deck_y, -0.80)), V((0.52, deck_y, -0.80))
dash.quad(*dash.add([c[2], c[3], dr, dl]))                                                                 # top deck
dash.quad(*dash.add([dl, dr, V((0.48, well_y, -0.62)), V((-0.48, well_y, -0.62))]))                         # slope to the well
dash.quad(*dash.add([V((-0.48, well_y, -0.62)), V((0.48, well_y, -0.62)), V((0.42, well_y - 0.02, face_z)), V((-0.42, well_y - 0.02, face_z))]))
dash.quad(*dash.add([V((-0.42, well_y - 0.02, face_z)), V((0.42, well_y - 0.02, face_z)), V((0.36, -0.72, face_z + 0.02)), V((-0.36, -0.72, face_z + 0.02))]))  # face
dash.quad(*dash.add([V((-0.36, -0.72, face_z + 0.02)), V((0.36, -0.72, face_z + 0.02)), V((0.36, -0.78, -1.0)), V((-0.36, -0.78, -1.0))]), face=V((0, -1.0, -0.3)))  # under (knees)
dash.quad(*dash.add([c[3], c[2], base_l, base_r]))                                                         # nose slope, out of sight
dash.quad(*dash.add([c[3], base_r, V((SILL_X, -0.75, -1.4)), c[4]]))
dash.quad(*dash.add([c[2], c[1], V((-SILL_X, -0.75, -1.4)), base_l]))
# the radar's housing and the two hologram pedestals (HUD points from calib3.json say where the holograms float)
dash.cylinder((0, well_y - 0.04, -0.72), (0, well_y + 0.025, -0.72), 0.20, n=24, r2=0.17)
for x, z in ((-0.30, -0.80), (0.30, -0.80)):
    dash.cylinder((x, deck_y - 0.03, z), (x, deck_y + 0.035, z), 0.10, n=16, r2=0.075)

# ---- the side consoles (the panels hang over them), from the sill in to the arm rests
for s in (1, -1):
    sx = lambda x: s * x
    top_in = -0.30
    pts_top = [V((sx(SILL_X), coam_pt(SILL_X, -0.80).y, -0.80)), V((sx(SILL_X), SILL_Y_BACK, -0.50)), V((sx(SILL_X), SILL_Y_BACK, BACK_Z))]
    inner = [V((sx(0.46), top_in, -0.70)), V((sx(0.46), top_in, -0.45)), V((sx(0.46), top_in, BACK_Z))]
    for a in range(2):
        console.quad(*console.add([pts_top[a], pts_top[a + 1], inner[a + 1], inner[a]]))
    console.quad(*console.add([inner[0], inner[2], V((sx(0.46), FLOOR_Y, BACK_Z)), V((sx(0.46), FLOOR_Y, -0.70))]))
    console.quad(*console.add([pts_top[0], inner[0], V((sx(0.46), FLOOR_Y, -0.70)), V((sx(SILL_X), FLOOR_Y, -0.80))]))
    console.quad(*console.add([coam_pt(sx(SILL_X), -0.80), c[1] if s < 0 else c[4], V((sx(SILL_X), -0.75, -1.4)), V((sx(SILL_X), FLOOR_Y, -0.80))]))
    console.quad(*console.add([pts_top[0], pts_top[2], V((sx(SILL_X), FLOOR_Y, BACK_Z)), V((sx(SILL_X), FLOOR_Y, -0.80))]))
    # the arm rest along the console's inner edge: the brown roll under the pilot's right arm at the bottom of view 3 and
    # the rounded end with buttons at the bottom of view 2 (the ribbed cylinder in view 2 and the white sleeve in view 3
    # are the commander's arms, not modelled)
    console.cylinder((sx(0.47), -0.31, -0.50), (sx(0.47), -0.31, 0.20), 0.055, n=12)
console.cylinder((0.42, -0.34, -0.40), (0.42, -0.22, -0.41), 0.022, n=10)          # the stick (under the right hand)
console.cylinder((-0.42, -0.34, -0.40), (-0.42, -0.24, -0.38), 0.026, n=10)        # the throttle (under the left hand)

# ---- the seat (only its top edges show from the eye)
seat.box((-0.27, -0.95, -0.30), (0.27, -0.82, 0.22))                               # cushion
seat.box((-0.29, -0.88, 0.22), (0.29, 0.02, 0.36))                                  # back
seat.box((-0.15, 0.02, 0.24), (0.15, 0.30, 0.34))                                   # head rest
for s in (1, -1):
    seat.box((s * 0.27 - 0.04, -0.62, -0.25), (s * 0.27 + 0.04, -0.55, 0.22))     # arm rests
    seat.box((s * 0.29 - 0.03, -0.88, 0.18), (s * 0.29 + 0.05, -0.10, 0.40))      # side bolsters

# ---- the floor
floor.poly([(-SILL_X, FLOOR_Y, -1.1), (SILL_X, FLOOR_Y, -1.1), (SILL_X, FLOOR_Y, BACK_Z), (-SILL_X, FLOOR_Y, BACK_Z)], (0, 0, 0))
floor.poly([(-0.36, -0.78, -1.0), (0.36, -0.78, -1.0), (0.36, FLOOR_Y, -1.1), (-0.36, FLOOR_Y, -1.1)], (0, 0, 0))

PARTS = [frame, dash, console, seat, floor, glass]

# ---------------------------------------------------------------- into Blender
COLOURS = {"frame": (0.05, 0.05, 0.055, 1), "dash": (0.20, 0.17, 0.15, 1), "console": (0.28, 0.24, 0.20, 1),
           "seat": (0.12, 0.10, 0.09, 1), "floor": (0.10, 0.10, 0.10, 1), "glass": (0.55, 0.70, 0.80, 0.15)}
ID_COLOURS = {"frame": (0, 1, 1, 1), "dash": (1, 0, 1, 1), "console": (1, 1, 0, 1), "seat": (0, 1, 0, 1),
              "floor": (1, 0.5, 0, 1), "glass": (1, 1, 1, 1)}
TO_BL = Matrix(((1, 0, 0), (0, 0, -1), (0, 1, 0)))     # this space -> Blender's


def make_object(part):
    me = bpy.data.meshes.new(part.name)
    bm = bmesh.new()
    vs = [bm.verts.new(TO_BL @ p) for p in part.v]
    made = []
    for f, face in zip(part.f, part.facing):
        try:
            made.append((bm.faces.new([vs[i] for i in f]), face))
        except ValueError:
            pass
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    made = [(f, face) for f, face in made if f.is_valid]
    bmesh.ops.recalc_face_normals(bm, faces=[f for f, face in made if face is None])   # closed solids: outward
    bm.normal_update()
    for f, face in made:                                                               # one-sided: towards their point
        if face is not None and f.normal.dot(TO_BL @ face - f.calc_center_median()) < 0:
            f.normal_flip()
    # uvs: a box projection, 1 uv unit a metre
    uv = bm.loops.layers.uv.new("UVMap")
    for face in bm.faces:
        n = face.normal
        ax = max(range(3), key=lambda k: abs(n[k]))
        for loop in face.loops:
            co = loop.vert.co
            loop[uv].uv = (co[1], co[2]) if ax == 0 else ((co[0], co[2]) if ax == 1 else (co[0], co[1]))
    bm.to_mesh(me); bm.free()
    for poly in me.polygons:
        poly.use_smooth = False
    mat = bpy.data.materials.get(part.name) or bpy.data.materials.new(part.name)
    mat.diffuse_color = COLOURS[part.name]
    mat.use_nodes = True
    bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf:
        bsdf.inputs["Base Color"].default_value = COLOURS[part.name]
        if part.name == "glass":
            bsdf.inputs["Alpha"].default_value = COLOURS["glass"][3]
    if part.name == "glass":
        mat.use_backface_culling = True
    me.materials.append(mat)
    ob = bpy.data.objects.new(part.name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def build():
    for ob in list(bpy.data.objects):
        bpy.data.objects.remove(ob, do_unlink=True)
    return {p.name: make_object(p) for p in PARTS}


def export(path):
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.wm.obj_export(filepath=path, export_selected_objects=False, forward_axis="NEGATIVE_Z", up_axis="Y",
                          export_triangulated_mesh=True, export_normals=True, export_uv=True, export_materials=True,
                          export_object_groups=False, export_material_groups=False, path_mode="RELATIVE")


def render(obs, out_dir):
    sc = bpy.context.scene
    sc.render.engine = "BLENDER_WORKBENCH"
    sc.render.resolution_x, sc.render.resolution_y, sc.render.resolution_percentage = W_PX, H_PX, 100
    sc.render.film_transparent = True
    sc.view_settings.view_transform = "Standard"
    sc.render.dither_intensity = 0.0
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_mode = "RGBA"
    sh = sc.display.shading
    cd = bpy.data.cameras.new("eye"); cd.sensor_fit = "VERTICAL"; cd.angle_y = 2 * math.atan(CY / F)
    cd.clip_start, cd.clip_end = 0.01, 100
    co = bpy.data.objects.new("eye", cd); sc.collection.objects.link(co); sc.camera = co
    for mode in ("ids", "shaded"):
        obs["glass"].hide_render = True
        if mode == "ids":       # each part one flat colour, for the outlines in compare.py
            sh.light, sh.color_type = "FLAT", "MATERIAL"
            sc.display.render_aa = "OFF"
            for name, ob in obs.items():
                ob.active_material.diffuse_color = ID_COLOURS[name]
        else:
            sh.light, sh.color_type = "STUDIO", "MATERIAL"
            sc.display.render_aa = "8"
            for name, ob in obs.items():
                ob.active_material.diffuse_color = COLOURS[name]
        sh.show_object_outline = False
        sh.show_cavity = mode == "shaded"
        for v in ("1", "2", "3"):
            R, t = cam(v)
            m = (TO_BL @ R).to_4x4(); m.translation = TO_BL @ t
            co.matrix_world = m
            sc.render.filepath = os.path.join(out_dir, f"render_{v}_{mode}.png")
            bpy.ops.render.render(write_still=True)
    # checks: the glass seen from the eye (view 1) and the whole cockpit from outside, behind and above on the left
    obs["glass"].hide_render = False
    sh.light, sh.color_type, sh.show_cavity = "STUDIO", "MATERIAL", False
    for name, ob in obs.items():
        ob.active_material.diffuse_color = ID_COLOURS[name] if name == "glass" else COLOURS[name]
    obs["glass"].active_material.diffuse_color = (0.3, 0.6, 1.0, 1)
    R, t = cam("1"); m = (TO_BL @ R).to_4x4(); co.matrix_world = m
    sc.render.filepath = os.path.join(out_dir, "check_glass_1.png"); bpy.ops.render.render(write_still=True)
    obs["glass"].hide_render = True
    sc.render.film_transparent = False
    eye = Vector((-2.3, 1.5, 1.6)); look = Vector((0, -0.25, -0.75))
    fwd = (look - eye).normalized(); right = fwd.cross(Vector((0, 1, 0))).normalized(); up = right.cross(fwd)
    m = (TO_BL @ Matrix((right, up, -fwd)).transposed()).to_4x4(); m.translation = TO_BL @ eye
    co.matrix_world = m
    sc.render.filepath = os.path.join(out_dir, "check_outside.png"); bpy.ops.render.render(write_still=True)
    obs["glass"].hide_render = False                                          # the glass, opaque, from outside
    sc.render.filepath = os.path.join(out_dir, "check_outside_glass.png"); bpy.ops.render.render(write_still=True)
    eye = Vector((1.8, 2.4, -3.2)); look = Vector((0, 0.0, -0.8))             # and from in front, above, on the right
    fwd = (look - eye).normalized(); right = fwd.cross(Vector((0, 1, 0))).normalized(); up = right.cross(fwd)
    m = (TO_BL @ Matrix((right, up, -fwd)).transposed()).to_4x4(); m.translation = TO_BL @ eye
    co.matrix_world = m
    sc.render.filepath = os.path.join(out_dir, "check_front_glass.png"); bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    obs = build()
    tris = sum(len(p.f) for p in PARTS)
    print("faces before triangulating:", tris, {p.name: len(p.f) for p in PARTS})
    export(os.path.join(HERE, "cockpit.obj"))
    keys = {"J_L": J_L, "J_R": J_R, "bar_step": at_z(1, (1068, 247), CROSS_Z), "knee_L": KNEE_L, "knee_R": KNEE_R,
            "rise_R_top": RISE_R[3], "rail_L_back": RAIL_L[3], "rail_R_back": RAIL_R[4], "rail_R_mid": RAIL_R[3]}
    json.dump({k: list(v) for k, v in keys.items()}, open(os.path.join(HERE, "work", "keypoints.json"), "w"), indent=1)
    work = os.path.join(HERE, "work"); os.makedirs(work, exist_ok=True)
    if "--no-render" not in sys.argv:
        render(obs, work)
    print("done")
