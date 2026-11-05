// A civilization's city layout as a function of a point (the vegetation and civilizations plan,
// step 7): the ground draws its roads, sidewalks and lots from it (planet.wesl), the scatter places
// one building per lot from it. No bindings: the frame, density and culture are passed in.
//
// The frame is the cube face's: a 4,096 m tile of face metres (integers, from the face's corner)
// and the point inside it (the ground's soil coordinate, exact), so the layout never loses
// precision, never repeats and joins across nodes and sectors by itself. Face metres are not true
// metres (0.64-1.27 of them across a face): widths are given in true metres and divided by the local
// metric `k`; block sizes simply vary with it. Every period divides 4,096 (no seam at tile edges).
//
// - Districts: cells of 512 face metres bounded by arterials, whose lines bend smoothly (a value
//   noise of 1,024 m along them, at most 55 m: they never cross).
// - Streets: each district's own grid, turned by its own angle (the culture sets how much), blocks
//   of 60-150 m; streets end at the district's arterials (T junctions).
// - Lots: each block in two rows of lots along its long side, 12-36 m wide (narrower downtown).

import super::settlement::st_hash;

const CITY_TILE: f32 = 4096.0;
const DISTRICT_M: f32 = 512.0;
const BEND_M: f32 = 55.0;

// Road widths (true metres): carriageway and sidewalk, urban arterials, rural roads, streets.
const ART_ROAD_M: f32 = 8.5;
const ART_WALK_M: f32 = 3.5;
const RURAL_ROAD_M: f32 = 3.4;
const STREET_ROAD_M: f32 = 4.5;
const STREET_WALK_M: f32 = 2.5;

// A point in the city frame: its 4,096 m tile and its face metres inside it (0..4,096).
struct CityPos {
    tile: vec2<i32>,
    q: vec2<f32>,
}

fn city_pos(tile0: vec2<i32>, local: vec2<f32>) -> CityPos {
    let t = floor(local / CITY_TILE);
    return CityPos(tile0 + vec2<i32>(t), local - t * CITY_TILE);
}

// The bend (face m) of arterial line `line` of `axis` at the position `cell` (a district index along
// the line) + `r` (m into it): a value noise with a lattice of 1,024 m.
fn ct_bend(line: i32, cell: i32, r: f32, axis: i32, seed: u32) -> f32 {
    let k = cell >> 1u;
    let f = (f32(cell & 1) * DISTRICT_M + r) / (2.0 * DISTRICT_M);
    let h0 = st_hash(vec3<i32>(line, k, axis), seed + 11u).x;
    let h1 = st_hash(vec3<i32>(line, k + 1, axis), seed + 11u).x;
    return BEND_M * (2.0 * mix(h0, h1, smoothstep(0.0, 1.0, f)) - 1.0);
}

// The district a point is in and its arterials: `cell` the district (global), `art` the distance
// (face m) to the nearest arterial and its signed across-offset, `along` the position along it (m,
// for its marks), `axis` 0 for a line of constant x, 1 of constant y.
struct District {
    cell: vec2<i32>,
    art: f32,
    across: f32,
    along: f32,
    axis: i32,
    line: i32,
}

fn ct_district(cp: CityPos, seed: u32) -> District {
    let c = cp.tile * 8 + vec2<i32>(floor(cp.q / DISTRICT_M));
    let r = cp.q - floor(cp.q / DISTRICT_M) * DISTRICT_M;
    // Lines of constant x bend along y, and the other way round.
    let lx = r.x - ct_bend(c.x, c.y, r.y, 0, seed);
    let rx = DISTRICT_M + ct_bend(c.x + 1, c.y, r.y, 0, seed) - r.x;
    let ly = r.y - ct_bend(c.y, c.x, r.x, 1, seed);
    let ry = DISTRICT_M + ct_bend(c.y + 1, c.x, r.x, 1, seed) - r.y;
    var d: District;
    d.cell = c + vec2<i32>(select(select(0, 1, rx < 0.0), -1, lx < 0.0), select(select(0, 1, ry < 0.0), -1, ly < 0.0));
    let ax = select(-rx, lx, abs(lx) < abs(rx));
    let ay = select(-ry, ly, abs(ly) < abs(ry));
    if abs(ax) < abs(ay) {
        d.art = abs(ax);
        d.across = ax;
        d.along = cp.q.y;
        d.axis = 0;
        d.line = select(c.x + 1, c.x, abs(lx) < abs(rx));
    } else {
        d.art = abs(ay);
        d.across = ay;
        d.along = cp.q.x;
        d.axis = 1;
        d.line = select(c.y + 1, c.y, abs(ly) < abs(ry));
    }
    return d;
}

// A district's street grid: its turn (cos, sin), block size (face m, along u and v), offsets.
struct Grid {
    rot: vec2<f32>,
    size: vec2<f32>,
    off: vec2<f32>,
}

// `spread` (0-1): how far a culture's grids turn from the face's axes (0 all aligned, 1 any angle).
fn ct_grid(cell: vec2<i32>, seed: u32, spread: f32) -> Grid {
    let h = st_hash(vec3<i32>(cell, 77), seed);
    let h2 = st_hash(vec3<i32>(cell, 78), seed);
    let a = (h.x - 0.5) * 3.14159 * 0.5 * mix(0.15, 1.0, spread) + select(0.0, 0.7854, h2.x < 0.15 * spread);
    var g: Grid;
    g.rot = vec2<f32>(cos(a), sin(a));
    g.size = vec2<f32>(mix(80.0, 150.0, h.y), mix(60.0, 110.0, h.z));
    g.off = vec2<f32>(h.w, h2.y) * g.size;
    return g;
}

// The point in its district's grid: the turned coordinates (face m) from the district's centre.
fn ct_local(cp: CityPos, cell: vec2<i32>, g: Grid) -> vec2<f32> {
    let centre = vec2<f32>(cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
    let p = cp.q - centre;
    return vec2<f32>(g.rot.x * p.x + g.rot.y * p.y, -g.rot.y * p.x + g.rot.x * p.y) + g.off;
}

// A line of width `w` at distance `d` from its middle, box-filtered by the footprint `fp` (all face
// m): the share of the pixel it covers (a line far thinner than the pixel covers w / fp of it).
fn ct_cover(d: f32, w: f32, fp: f32) -> f32 {
    let f = max(fp, 1e-4);
    let lo = max(d - 0.5 * f, -0.5 * w);
    let hi = min(d + 0.5 * f, 0.5 * w);
    return clamp((hi - lo) / f, 0.0, 1.0);
}

// Dashes of `on` m every `period` m (a divisor of 4,096) at `along`, filtered by `fp`.
fn ct_dash(along: f32, on: f32, period: f32, fp: f32) -> f32 {
    let x = along - floor(along / period) * period;
    let sharp = (1.0 - smoothstep(on, on + fp, x)) * smoothstep(0.0, fp, x + fp * 0.5);
    return mix(sharp, on / period, smoothstep(period * 0.25, period, fp));
}

// A lot of a block: the block's interior (inside its streets' sidewalks) in lots along u, their
// width from the block's own hash (14-30 face m: never the pixel's density, or the lots' edges
// would wander across a block), in two rows back to back along v. `b` the point in its block (face
// m from the block's corner, on its streets' middle lines), `bc` the block, `edge` half a street's
// full width (face m).
struct Lot {
    lo: vec2<f32>,
    hi: vec2<f32>,
    // -1: its street along lo.y (the first row), 1 along hi.y
    front: f32,
    key: vec3<i32>,
}

fn ct_lot(b: vec2<f32>, bc: vec2<i32>, cell: vec2<i32>, size: vec2<f32>, edge: f32, seed: u32) -> Lot {
    let lo0 = vec2<f32>(edge);
    let hi0 = max(size - vec2<f32>(edge), lo0 + vec2<f32>(1.0));
    let bh = st_hash(vec3<i32>(cell.x * 64 + bc.x, cell.y * 64 + bc.y, 4099), seed + 23u);
    let n = max(round((hi0.x - lo0.x) / mix(14.0, 30.0, bh.x)), 1.0);
    let w = (hi0.x - lo0.x) / n;
    let i = clamp(floor((b.x - lo0.x) / w), 0.0, n - 1.0);
    let second = b.y > 0.5 * size.y;
    var l: Lot;
    l.lo = vec2<f32>(lo0.x + i * w, select(lo0.y, 0.5 * size.y, second));
    l.hi = vec2<f32>(lo0.x + (i + 1.0) * w, select(0.5 * size.y, hi0.y, second));
    l.front = select(-1.0, 1.0, second);
    l.key = vec3<i32>(cell.x * 64 + bc.x, cell.y * 64 + bc.y, i32(i) * 2 + select(0, 1, second));
    return l;
}

// The business cores: a smooth value noise over the city (lattice 1,024 face m, in the tile frame:
// exact, never repeating), 0..1. Towers stand where it is high; their share thins out at its edges.
fn ct_core(cp: CityPos, at: vec2<f32>, seed: u32) -> f32 {
    let q = at / 1024.0;
    let c = cp.tile * 4 + vec2<i32>(floor(q));
    let f = q - floor(q);
    let s = f * f * (3.0 - 2.0 * f);
    let a = st_hash(vec3<i32>(c, 93), seed).x;
    let b = st_hash(vec3<i32>(c + vec2<i32>(1, 0), 93), seed).x;
    let e = st_hash(vec3<i32>(c + vec2<i32>(0, 1), 93), seed).x;
    let g = st_hash(vec3<i32>(c + vec2<i32>(1, 1), 93), seed).x;
    return mix(mix(a, b, s.x), mix(e, g, s.x), s.y);
}

// The building of the lot a point lies in (the scatter's placement, scatter.wesl): its middle (face
// m from the tile of `cp`), size along the grid's axes and height (true m), the grid's turn (cos,
// sin of the angle from the face's u axis), a hash. `urban` the town's density at the lot (0-1),
// `k` the metric. `ok` false where the lot holds no building: an arterial or another district
// there, a park, a town too thin.
struct Building {
    centre: vec2<f32>,
    size: vec2<f32>,
    height: f32,
    rot: vec2<f32>,
    hash: vec4<f32>,
    ok: bool,
    // The model: 0 the box (towns), 1 a round hut, 2 a longhouse (villages).
    model: u32,
}

fn ct_building(cp: CityPos, k: f32, urban: f32, seed: u32, spread: f32) -> Building {
    var o: Building;
    o.ok = false;
    let d = ct_district(cp, seed);
    let g = ct_grid(d.cell, seed, spread);
    let lp = ct_local(cp, d.cell, g);
    let bc = vec2<i32>(floor(lp / g.size));
    let b = lp - vec2<f32>(bc) * g.size;
    let s_all = (STREET_ROAD_M + STREET_WALK_M) * 2.0 / k;
    let lot = ct_lot(b, bc, d.cell, g.size, 0.5 * s_all, seed);
    let h = st_hash(lot.key, seed + 21u);
    let h2 = st_hash(lot.key, seed + 29u);
    o.hash = h2;
    // Some lots stay empty (squares, car parks, gardens); thin towns leave more.
    let streets = smoothstep(0.06, 0.2, urban);
    if h.w > streets * 0.93 {
        return o;
    }
    // How downtown the lot is: dense towns' blocks fill their lots, the cores' most of all (the core
    // field at the lot's point).
    let dense = smoothstep(0.35, 0.8, urban);
    let core = smoothstep(1.0 - 0.3 * dense * dense, 1.0 - 0.12 * dense * dense, ct_core(cp, cp.q, seed)) * dense;
    let down = dense * mix(0.55, 1.0, core);
    // Setbacks (true m): front, sides, back; houses in gardens, downtown to the lot's edges.
    let front = mix(mix(4.0, 8.0, h2.x), 0.5, down) / k;
    let side = mix(mix(1.5, 3.5, h2.y), 0.0, down) / k;
    let back = mix(mix(3.0, 9.0, h2.z), 0.5, down) / k;
    var lo = lot.lo + vec2<f32>(side, select(back, front, lot.front < 0.0));
    var hi = lot.hi - vec2<f32>(side, select(front, back, lot.front < 0.0));
    if any(hi - lo < vec2<f32>(4.0 / k)) {
        return o;
    }
    // Houses take part of their lot; buildings in town most of it.
    let fill = mix(mix(0.55, 0.85, h.y), 1.0, down);
    let mid = 0.5 * (lo + hi);
    let half = 0.5 * (hi - lo) * vec2<f32>(fill, mix(fill, 1.0, 0.5));
    // Back to the face's frame: the lot's middle, from the district's grid.
    let c_lp = vec2<f32>(bc) * g.size + mid - g.off;
    let p = vec2<f32>(g.rot.x * c_lp.x - g.rot.y * c_lp.y, g.rot.y * c_lp.x + g.rot.x * c_lp.y);
    let centre = p + vec2<f32>(d.cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
    // Its middle must lie in the same district and clear of the arterials (true m: their road and
    // walks, and half the building).
    let at = city_pos(cp.tile, centre);
    let dc = ct_district(at, seed);
    if any(dc.cell != d.cell) {
        return o;
    }
    let a_all = (ART_ROAD_M + ART_WALK_M) * 2.0 / k;
    if dc.art < 0.5 * a_all + length(half) {
        return o;
    }
    // Storeys: houses 1-3, in town more; towers in the business cores alone, so they cluster as
    // cities' do (Earth's lights are bright over whole metro areas), thinning out at the cores' edges.
    let tower = select(0.0, 1.0, h.z < 0.75 * core);
    let storeys = round(mix(1.0, 3.0, h.x) + urban * mix(0.5, 6.0, h.y * h.y) * mix(1.0, 1.6, core) + tower * mix(10.0, 55.0, h2.w * h2.w));
    o.centre = centre;
    o.size = 2.0 * half * k;
    o.height = storeys * 3.1 + 0.5;
    o.rot = g.rot;
    o.ok = true;
    o.model = 0u;
    return o;
}

// ---- villages (level 1) ----------------------------------------------------------------------
// A primitive people's villages: the same districts and lots, but the paths meander (each district's
// grid bent by a smooth warp of up to 16 face m over 128 m), dirt, without kerbs, lamps or traffic;
// round huts and longhouses on yards of packed earth; hearth fires by night.

const PATH_M: f32 = 1.3;
const TRACK_M: f32 = 2.2;

// The warp of a district's grid at its point `lp` (face m): a smooth value noise.
fn ct_warp(lp: vec2<f32>, cell: vec2<i32>, seed: u32) -> vec2<f32> {
    let q = lp / 128.0;
    let c = vec2<i32>(floor(q));
    let f = q - floor(q);
    let s = f * f * (3.0 - 2.0 * f);
    let key = cell.x * 977 + cell.y * 131;
    let a = st_hash(vec3<i32>(c, key), seed + 31u).xy;
    let b = st_hash(vec3<i32>(c + vec2<i32>(1, 0), key), seed + 31u).xy;
    let e = st_hash(vec3<i32>(c + vec2<i32>(0, 1), key), seed + 31u).xy;
    let g = st_hash(vec3<i32>(c + vec2<i32>(1, 1), key), seed + 31u).xy;
    return (mix(mix(a, b, s.x), mix(e, g, s.x), s.y) - 0.5) * 32.0;
}

// A village's hut or longhouse on the lot a point lies in (as `ct_building`).
fn ct_hut(cp: CityPos, k: f32, urban: f32, seed: u32) -> Building {
    var o: Building;
    o.ok = false;
    let d = ct_district(cp, seed);
    let g = ct_grid(d.cell, seed, 1.0);
    let l0 = ct_local(cp, d.cell, g);
    let lp = l0 + ct_warp(l0, d.cell, seed);
    let bc = vec2<i32>(floor(lp / g.size));
    let b = lp - vec2<f32>(bc) * g.size;
    let lot = ct_lot(b, bc, d.cell, g.size, PATH_M / k, seed);
    let h = st_hash(lot.key, seed + 21u);
    let h2 = st_hash(lot.key, seed + 29u);
    o.hash = h2;
    let streets = smoothstep(0.06, 0.2, urban);
    if h.w > streets * 0.93 {
        return o;
    }
    // A round hut (5-8 m) or a longhouse (6-8 m by 11-18 m along the lot), in the lot's middle.
    let room = (lot.hi - lot.lo) * k - vec2<f32>(3.0);
    let long = h.y >= 0.6;
    var size = vec2<f32>(mix(5.0, 8.0, h.x));
    var height = mix(5.0, 7.0, h2.x);
    if long {
        size = vec2<f32>(mix(6.0, 8.0, h.x), mix(11.0, 18.0, h2.y));
        height = mix(6.0, 8.0, h2.x);
    }
    if any(room < vec2<f32>(5.0)) {
        return o;
    }
    size = min(size, room);
    // Back through the warp (a few steps: it bends gently), then the grid, to the face's frame.
    let aim = vec2<f32>(bc) * g.size + 0.5 * (lot.lo + lot.hi);
    var l = aim;
    for (var i = 0; i < 4; i++) {
        l = aim - ct_warp(l, d.cell, seed);
    }
    let c_lp = l - g.off;
    let p = vec2<f32>(g.rot.x * c_lp.x - g.rot.y * c_lp.y, g.rot.y * c_lp.x + g.rot.x * c_lp.y);
    let centre = p + vec2<f32>(d.cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
    let at = city_pos(cp.tile, centre);
    let dc = ct_district(at, seed);
    if any(dc.cell != d.cell) || dc.art < (TRACK_M + 0.5 * length(size)) / k {
        return o;
    }
    o.centre = centre;
    o.size = size;
    o.height = height;
    o.rot = g.rot;
    o.ok = true;
    o.model = select(1u, 2u, long);
    // A watchtower now and then on the village's edges (the hero model).
    if h2.z < 0.05 && streets < 0.6 {
        let s = mix(0.85, 1.15, h.z);
        o.model = M_WATCH;
        o.size = WATCH_SIZE.xz * s;
        o.height = WATCH_SIZE.y * s;
    }
    return o;
}

// The hero models' real sizes (width, height, depth; m) as made in Blender (assets/models/hero).
const M_TEMPLE: u32 = 11u;
const M_WATCH: u32 = 12u;
const M_LANDMARK: u32 = 13u;
const TEMPLE_SIZE = vec3<f32>(22.0, 15.8, 23.5);
const WATCH_SIZE = vec3<f32>(5.23, 14.0, 5.23);
const LANDMARK_SIZE = vec3<f32>(290.74, 1000.0, 251.78);

// A village district's temple (4 in 10 districts): on its middle, along its grid, about 22 m across.
fn ct_temple(cp: CityPos, cell: vec2<i32>, seed: u32) -> Building {
    var o: Building;
    let h = st_hash(vec3<i32>(cell, 4131), seed + 47u);
    o.ok = h.x < 0.4;
    o.model = M_TEMPLE;
    let s = mix(0.85, 1.15, h.y);
    o.centre = vec2<f32>(cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
    o.size = TEMPLE_SIZE.xz * s;
    o.height = TEMPLE_SIZE.y * s;
    o.rot = ct_grid(cell, seed, 1.0).rot;
    o.hash = h;
    return o;
}

// A village's ground (as `city_ground`): yards of packed earth round the huts, the natural ground
// between, dirt paths, tracks between villages; by night the hearth fires (flickering) beside the huts.
fn village_ground(cp: CityPos, k: f32, fp_m: f32, urban: f32, roads: f32, seed: u32, base: vec3<f32>, t: f32) -> CityGround {
    let fp = fp_m / max(k, 1e-3);
    let dirt = vec3<f32>(0.19, 0.15, 0.105);
    let earth = vec3<f32>(0.25, 0.20, 0.14);
    var col = base;
    var built = 0.0;
    var lamp = 1.0;
    var emit = vec3<f32>(0.0);
    let d = ct_district(cp, seed);
    let streets = smoothstep(0.06, 0.2, urban);
    if streets > 0.0 {
        let g = ct_grid(d.cell, seed, 1.0);
        let l0 = ct_local(cp, d.cell, g);
        let lp = l0 + ct_warp(l0, d.cell, seed);
        let bc = vec2<i32>(floor(lp / g.size));
        let b = lp - vec2<f32>(bc) * g.size;
        let su = min(b.x, g.size.x - b.x);
        let sv = min(b.y, g.size.y - b.y);
        let lot = ct_lot(b, bc, d.cell, g.size, PATH_M / k, seed);
        let lh = st_hash(lot.key, seed + 21u);
        let mid = 0.5 * (lot.lo + lot.hi);
        let dm = length(b - mid) * k;
        let fine = 1.0 - smoothstep(0.25 * g.size.y, g.size.y, fp);
        let yard = (1.0 - smoothstep(5.0, 11.0, dm)) * select(0.0, 1.0, lh.w <= streets * 0.93);
        var c = mix(base, earth * (0.9 + 0.2 * lh.y), yard * 0.85);
        let path = max(ct_cover(su, 2.0 * PATH_M / k, fp), ct_cover(sv, 2.0 * PATH_M / k, fp));
        c = mix(c, dirt * (0.9 + 0.2 * lh.z), path);
        // The hearth: a fire before the hut's door (towards the lot's path), flickering.
        var fire = 0.0;
        if lh.w <= streets * 0.93 && fp < 40.0 {
            let door = vec2<f32>(mid.x, mid.y + lot.front * 6.0 / k);
            let fd = length(b - door);
            let flick = 0.75 + 0.25 * sin(t * (7.0 + 3.0 * lh.x) + lh.y * 40.0) * sin(t * 2.3 + lh.z * 17.0);
            fire = ct_glow(fd, 0.6 / k, fp) * flick;
            emit += vec3<f32>(1.0, 0.42, 0.10) * (2.5 * fire);
            // Its light on the yard round it.
            c += vec3<f32>(0.0);
            lamp = mix(1.0, 0.05 + 4.0 * exp(-fd * fd * k * k / 25.0), fine * streets);
        }
        let mean = mix(mix(base, earth, 0.3), dirt, 0.15);
        c = mix(mean, c, fine);
        col = mix(col, c, streets);
        built = streets * mix(0.3, max(path, yard * 0.6), fine);
    }
    // The tracks between villages (every other arterial line, about a kilometre apart).
    let art_on = max(roads * select(0.0, 1.0, (d.line & 1) == 0), streets);
    if art_on > 0.0 {
        let track = ct_cover(d.art, 2.0 * TRACK_M / k, fp);
        let fine = 1.0 - smoothstep(0.1 * DISTRICT_M, DISTRICT_M, fp);
        let share = clamp(4.0 * TRACK_M / k / DISTRICT_M, 0.0, 1.0);
        let c = mix(mix(col, dirt, share), mix(col, dirt, track), fine);
        col = mix(col, c, art_on);
        built = max(built, art_on * mix(share, track, fine));
    }
    var o: CityGround;
    o.col = col;
    o.built = built;
    o.lamp = lamp;
    o.emit = emit;
    return o;
}

// The city's ground: what lies at a point, as albedo over `base` (the natural ground). `k` the
// local metric (true m per face m), `fp` the pixel's footprint (true m), `urban` (0-1) the town's
// density here, `roads` (0-1) the countryside's road network here (arterials run through farmland),
// `spread` the culture's grid turn, `paved` its towns' colour. Returns the albedo and, in w, the
// built share (pavement and roads: no plants there).
// A small light (radius `r`) at distance `d` (all face m), blurred to the footprint `fp` keeping its
// light (as settlement.wesl's blobs): a point from afar, a spot close up.
fn ct_glow(d: f32, r: f32, fp: f32) -> f32 {
    let re2 = r * r + 0.25 * fp * fp;
    return r * r / re2 * exp(-d * d / re2);
}

// The traffic on a lane: cars of 4.5 x 1.8 m in slots of `period` face m (a divisor of 4,096) moving
// at `v` face m/s along it, each slot holding a car with chance `busy`; `x` the pixel along the lane
// (face m, in the lane's own frame), `y` across from its middle, `base` the slot number where `x` is 0
// (whole tiles of slots: a car keeps its slot across tile edges), `key` the lane. No state: a car's
// place is a hash of its slot and the clock `t`. Returns x the car's body over the pixel (its paint's
// brightness in y), z its headlights', w its tail lights' light (the lights face the way it drives).
fn ct_traffic(x: f32, y: f32, v: f32, period: f32, busy: f32, t: f32, base: i32, key: vec2<i32>, seed: u32, k: f32, fp: f32) -> vec4<f32> {
    let xm = x - v * t;
    let slot = floor(xm / period);
    let h = st_hash(vec3<i32>(key, base + i32(slot)), seed + 57u);
    if h.w >= busy {
        return vec4<f32>(0.0);
    }
    let len = 4.5 / k;
    let wid = 1.8 / k;
    let cx = xm - slot * period - h.x * (period - len);
    // The body: its box over the pixel.
    let f = max(fp, 1e-4);
    let bx = clamp((min(cx + 0.5 * f, len) - max(cx - 0.5 * f, 0.0)) / f, 0.0, 1.0);
    let body = bx * ct_cover(abs(y), wid, fp);
    // Its lights: two at the front, two at the back (the front is the end it drives towards).
    let front = select(0.0, len, v > 0.0);
    let back = len - front;
    let side = abs(abs(y) - 0.65 / k);
    let head = ct_glow(length(vec2<f32>(cx - front, side)), 0.22 / k, fp);
    let tail = ct_glow(length(vec2<f32>(cx - back, side)), 0.18 / k, fp);
    return vec4<f32>(body, h.y, head, tail);
}

// A car's paint by its hash: white, black, silver, grey, blue, red, green (Earth's mix).
fn ct_paint(h: f32) -> vec3<f32> {
    if h < 0.22 { return vec3<f32>(0.62, 0.62, 0.60); }
    if h < 0.42 { return vec3<f32>(0.025, 0.025, 0.028); }
    if h < 0.6 { return vec3<f32>(0.40, 0.41, 0.42); }
    if h < 0.74 { return vec3<f32>(0.16, 0.16, 0.17); }
    if h < 0.85 { return vec3<f32>(0.04, 0.09, 0.28); }
    if h < 0.95 { return vec3<f32>(0.42, 0.04, 0.03); }
    return vec3<f32>(0.05, 0.16, 0.08);
}

// The street lamps along a street (one a kerb every 32 face m, the two kerbs staggered): `x` along the
// street, `y` across from its middle, `half` half its carriageway (face m). x their pools of light on
// the ground (about 1 on a street's mean), y their heads' light.
fn ct_lamps(x: f32, y: f32, half: f32, k: f32, fp: f32) -> vec2<f32> {
    let kerb = half + 0.6 / k;
    let side = select(-1.0, 1.0, y > 0.0);
    let off = select(0.0, 16.0, y > 0.0);
    let lx = floor((x - off) / 32.0 + 0.5) * 32.0 + off;
    let d = vec2<f32>(x - lx, y - side * kerb);
    let r = 9.0 / k;
    let pool = exp(-dot(d, d) / (r * r)) * 1.9 + 0.15;
    let head = ct_glow(length(d), 0.35 / k, fp);
    // Far: the pools' mean.
    return vec2<f32>(mix(pool, 1.0, smoothstep(8.0, 32.0, fp)), head);
}

// What `city_ground` gives: the albedo, the built share (pavement and roads: no plants there) and
// the street lamps' share of the town's night light here: 1 from afar (where the pixel spans blocks,
// the glow map's or the field's light as it is); near, about 0.4 of it on a block's mean, the streets
// and sidewalks bright and the lots dim (the buildings' windows give the rest of a town's light).
struct CityGround {
    col: vec3<f32>,
    built: f32,
    lamp: f32,
    // The street lamps' heads and the cars' lights (glow map units, before the night switch).
    emit: vec3<f32>,
}

// `lamp_c` the street lamps' colour (Earth's sodium; a generated planet's culture's light).
fn city_ground(cp: CityPos, k: f32, fp_m: f32, urban: f32, roads: f32, seed: u32, spread: f32, base: vec3<f32>, paved: vec3<f32>, lamp_c: vec3<f32>, t: f32) -> CityGround {
    let fp = fp_m / max(k, 1e-3);
    let asphalt = vec3<f32>(0.055, 0.055, 0.06);
    let walk = vec3<f32>(0.19, 0.185, 0.175);
    let paint = vec3<f32>(0.45, 0.45, 0.42);
    var col = base;
    var built = 0.0;
    var lamp = 1.0;
    var emit = vec3<f32>(0.0);
    let head_c = vec3<f32>(1.0, 0.92, 0.75);
    let tail_c = vec3<f32>(1.0, 0.06, 0.02);

    let d = ct_district(cp, seed);
    let streets = smoothstep(0.06, 0.2, urban);
    // Lots: downtown paved, in town gardens with a paved strip along the street.
    if streets > 0.0 {
        let g = ct_grid(d.cell, seed, spread);
        let lp = ct_local(cp, d.cell, g);
        let b = lp - floor(lp / g.size) * g.size;
        // Distances to the district's streets (lines of constant u and of constant v).
        let su = min(b.x, g.size.x - b.x);
        let sv = min(b.y, g.size.y - b.y);
        let s_road = STREET_ROAD_M * 2.0 / k;
        let s_all = s_road + STREET_WALK_M * 2.0 / k;
        let down = smoothstep(0.35, 0.8, urban);
        // The lot's own ground: gardens (the natural ground) or paved, by the lot's hash.
        let bc = vec2<i32>(floor(lp / g.size));
        let lot = ct_lot(b, bc, d.cell, g.size, 0.5 * s_all, seed);
        let lh = st_hash(lot.key, seed + 21u);
        let pave = select(0.0, 1.0, lh.x < mix(0.25, 0.95, down));
        let lot_col = mix(base, paved * (0.85 + 0.3 * lh.y), pave * streets);
        // The ground's features fade to their mean once the pixel spans a block.
        let fine = 1.0 - smoothstep(0.25 * g.size.y, g.size.y, fp);
        let walk_c = max(ct_cover(su, s_all, fp), ct_cover(sv, s_all, fp));
        let road_c = max(ct_cover(su, s_road, fp), ct_cover(sv, s_road, fp));
        var c = mix(lot_col, walk, walk_c);
        // Kerbs and the road: asphalt, its centre line (dashed), zebra crossings by the junctions.
        var road = asphalt * (0.9 + 0.2 * lh.z);
        let on_u = sv < su;
        let cross_d = select(sv, su, on_u);
        let across = select(su, sv, on_u);
        let along_l = select(lp.y, lp.x, on_u);
        let centre = ct_cover(across, 0.12 / k, fp) * ct_dash(along_l, 3.0, 8.0, fp) * (1.0 - ct_cover(cross_d, s_all, fp));
        road = mix(road, paint, centre * 0.8);
        // In line with the crossing street's sidewalks.
        let zebra = smoothstep(s_road * 0.5, s_road * 0.5 + 0.3 / k, cross_d) * (1.0 - smoothstep(s_all * 0.5 + 0.5 / k, s_all * 0.5 + 0.8 / k, cross_d));
        let bars = ct_dash(select(lp.x, lp.y, on_u) * k, 0.5, 1.0, fp * k);
        road = mix(road, paint, zebra * bars * 0.85 * fine);
        // Traffic and lamps on the street the pixel is nearest (signed across its middle; right-hand
        // traffic), while cars are bigger than a few pixels' worth.
        var pools = 1.0;
        if road_c > 0.0 && fp < 12.0 {
            let half_s = 0.5 * select(g.size.x, g.size.y, on_u);
            let bv = select(b.x, b.y, on_u);
            let ys = select(bv, bv - 2.0 * half_s, bv > half_s);
            let line = vec2<i32>(floor(lp / g.size + 0.5));
            let lane = select(1, -1, ys > 0.0);
            let v = f32(lane) * 9.0 / k;
            let yl = ys + f32(lane) * 0.25 * s_road;
            let key = vec2<i32>(d.cell.x * 4096 + select(line.x, line.y, on_u) * 2 + select(0, 1, on_u), d.cell.y * 4 + lane + 2);
            let car = ct_traffic(along_l, yl, v, 24.0, 0.25 * streets + 0.1, t, 0, key, seed, k, fp) * (1.0 - smoothstep(4.0, 12.0, fp));
            road = mix(road, ct_paint(car.y), car.x);
            emit += (head_c * car.z * 3.0 + tail_c * car.w * 1.5);
            let lm = ct_lamps(along_l, ys, 0.5 * s_road, k, fp);
            pools = lm.x;
            emit += lamp_c * lm.y * 4.0;
        }
        c = mix(c, road, road_c);
        // The mean when the pixel spans blocks: lots, sidewalks and roads by their areas.
        let road_share = clamp(s_road / g.size.x + s_road / g.size.y, 0.0, 1.0);
        let walk_share = clamp((s_all - s_road) / g.size.x + (s_all - s_road) / g.size.y, 0.0, 1.0);
        let mean = mix(mix(base, paved, mix(0.25, 0.95, down)), walk, walk_share) * (1.0 - road_share) + asphalt * road_share;
        c = mix(mean, c, fine);
        col = mix(col, c, streets);
        let near_built = max(pave, max(walk_c, road_c));
        let mean_built = road_share + walk_share + (1.0 - road_share - walk_share) * mix(0.25, 0.95, down);
        built = streets * mix(mean_built, near_built, fine);
        lamp = mix(1.0, 0.06 + 0.64 * max(walk_c, road_c) * pools, fine * streets);
    }
    // The arterials (in town with sidewalks, in the countryside narrow and plain, every other line:
    // about a kilometre apart).
    let art_on = max(roads * select(0.0, 1.0, (d.line & 1) == 0), streets);
    if art_on > 0.0 {
        let a_road = mix(RURAL_ROAD_M, ART_ROAD_M, streets) * 2.0 / k;
        let a_all = a_road + ART_WALK_M * 2.0 / k * streets;
        let walk_c = ct_cover(d.art, a_all, fp) * streets;
        let road_c = ct_cover(d.art, a_road, fp);
        var road = asphalt;
        // Marks: a double centre line, lane dashes in town.
        let dbl = max(ct_cover(abs(d.across - 0.2 / k), 0.12 / k, fp), ct_cover(abs(d.across + 0.2 / k), 0.12 / k, fp));
        let lanes = ct_cover(abs(abs(d.across) - 3.6 / k), 0.12 / k, fp) * ct_dash(d.along, 3.0, 16.0, fp) * streets;
        road = mix(road, paint, max(dbl, lanes) * 0.8);
        if road_c > 0.0 && fp < 12.0 {
            // Four lanes in town (two out of it), right-hand traffic, slots of 32 face m (128 a tile).
            let lane = select(1, -1, d.across > 0.0);
            let outer = abs(d.across) > 3.6 / k && streets > 0.5;
            let yl = d.across + f32(lane) * select(1.9, 6.0, outer) / k;
            let v = f32(lane) * select(13.0, 16.0, outer) / k;
            let tile_a = select(cp.tile.x, cp.tile.y, d.axis == 0);
            let key = vec2<i32>(d.line * 8 + d.axis * 4 + lane + 1 + select(0, 2, outer), 77);
            let car = ct_traffic(d.along, yl, v, 32.0, 0.35 + 0.35 * streets, t, tile_a * 128, key, seed, k, fp) * (1.0 - smoothstep(4.0, 12.0, fp));
            road = mix(road, ct_paint(car.y), car.x);
            emit += (head_c * car.z * 3.0 + tail_c * car.w * 1.5);
            let lm = ct_lamps(d.along, d.across, 0.5 * a_road, k, fp);
            emit += lamp_c * lm.y * 4.0 * streets;
            lamp = max(lamp, mix(lamp, 0.8 * lm.x, road_c * streets * smoothstep(0.0, 1.0, 1.0 - fp / 64.0)));
        }
        // Far: the road's share of the district.
        let fine = 1.0 - smoothstep(0.1 * DISTRICT_M, DISTRICT_M, fp);
        let share = clamp(2.0 * a_all / DISTRICT_M * mix(0.5, 1.0, streets), 0.0, 1.0);
        var c = mix(mix(col, walk, walk_c), road, road_c);
        c = mix(mix(col, asphalt, share), c, fine);
        col = mix(col, c, art_on);
        built = max(built, art_on * mix(share, max(walk_c, road_c), fine));
        lamp = max(lamp, mix(lamp, 0.8, max(walk_c, road_c) * fine * art_on));
    }
    var o: CityGround;
    o.col = col;
    o.built = built;
    o.lamp = lamp;
    o.emit = emit;
    return o;
}

// ---- futuristic cities (level 3 and up) ------------------------------------------------------
// The same districts, blocks and lots, denser and taller. In the business cores (wider than level
// 2's) whole blocks become megatowers of 150-1,500 m (tapering, stepped, twisted or round), joined
// across their streets by skybridges; a few districts hold an arcology (a stepped pyramid over most of
// the district); the lots' buildings are the towns' boxes, taller, in the future facades. Flying
// traffic streams along lanes over the arterials at two heights.

// The models, as offsets from the box (the library's models after the villages').
const M_FUTURE: u32 = 3u;
const M_TAPER: u32 = 4u;
const M_STEP: u32 = 5u;
const M_TWIST: u32 = 6u;
const M_ROUND: u32 = 7u;
const M_ARCO: u32 = 8u;
const M_BRIDGE: u32 = 9u;
const M_LANE: u32 = 10u;

// A point's block: its district, the district's grid, the block and the point in it (face m).
struct Block {
    d: District,
    g: Grid,
    bc: vec2<i32>,
    b: vec2<f32>,
}

fn ct_block(cp: CityPos, seed: u32, spread: f32) -> Block {
    var o: Block;
    o.d = ct_district(cp, seed);
    o.g = ct_grid(o.d.cell, seed, spread);
    let lp = ct_local(cp, o.d.cell, o.g);
    o.bc = vec2<i32>(floor(lp / o.g.size));
    o.b = lp - vec2<f32>(o.bc) * o.g.size;
    return o;
}

// A point of a district's grid back in the tile frame of `cp` (`ct_local`'s inverse).
fn ct_unlocal(cp: CityPos, cell: vec2<i32>, g: Grid, lp: vec2<f32>) -> vec2<f32> {
    let c = lp - g.off;
    let p = vec2<f32>(g.rot.x * c.x - g.rot.y * c.y, g.rot.y * c.x + g.rot.x * c.y);
    return p + vec2<f32>(cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
}

// The middle of block `k` in the tile frame of `cp`.
fn ct_block_mid(cp: CityPos, k: Block) -> vec2<f32> {
    return ct_unlocal(cp, k.d.cell, k.g, (vec2<f32>(k.bc) + 0.5) * k.g.size);
}

// The district's arcology if it has one: in its middle, 260-360 m across (the face's axes), 350-800 m
// high. Decided by its hash and the core field alone (never the town's density: every point of the
// district must agree, and the scatter's points see different densities).
fn ct_arcology(cp: CityPos, cell: vec2<i32>, k: f32, seed: u32, level: u32) -> Building {
    var o: Building;
    o.ok = false;
    o.model = M_ARCO;
    let h = st_hash(vec3<i32>(cell, 4127), seed + 43u);
    let centre = vec2<f32>(cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
    let chance = select(0.07, 0.14, level >= 4u);
    let core = smoothstep(0.55, 0.8, ct_core(cp, centre, seed));
    if h.x >= chance * core {
        // The landmark (the hero model, 700-1,000 m) in a few of the strongest cores.
        if h.x < (chance + 0.02) * core * core {
            // (Inside its district, as the arcology.)
            let s = min(mix(0.7, 1.0, h.w), 0.62 * DISTRICT_M * k / LANDMARK_SIZE.x);
            o.ok = true;
            o.model = M_LANDMARK;
            o.centre = centre;
            o.size = LANDMARK_SIZE.xz * s;
            o.height = LANDMARK_SIZE.y * s;
            o.rot = vec2<f32>(1.0, 0.0);
            o.hash = st_hash(vec3<i32>(cell, 4128), seed + 43u);
        }
        return o;
    }
    o.centre = centre;
    o.size = vec2<f32>(min(mix(260.0, 360.0, h.y), 0.62 * DISTRICT_M * k));
    o.height = mix(350.0, 800.0, h.z);
    o.rot = vec2<f32>(1.0, 0.0);
    o.hash = st_hash(vec3<i32>(cell, 4128), seed + 43u);
    o.ok = true;
    return o;
}

// Whether building `b` (its middle, footprint) stands on the arcology's or landmark's `a` footprint
// (and a margin).
fn ct_on_arcology(a: Building, b: Building, k: f32) -> bool {
    let d = abs(b.centre - a.centre) * k;
    return a.ok && all(d < vec2<f32>(0.5 * max(a.size.x, a.size.y) + 0.5 * length(b.size) + 10.0));
}

// The megatower of block `k` if the block is one: in its middle, the block's interior less a plaza
// (6-16 m), at most 120 m across, 150-1,500 m high (the tallest in the cores' peaks, at most 14 times
// its width), tapering, stepped, twisted or round (the last two square). `urban` the town's density at
// the block's middle (every point of the block must agree).
fn ct_mega(cp: CityPos, k: Block, km: f32, urban: f32, seed: u32) -> Building {
    var o: Building;
    o.ok = false;
    let centre = ct_block_mid(cp, k);
    let core = smoothstep(0.45, 0.8, ct_core(cp, centre, seed)) * smoothstep(0.35, 0.8, urban);
    let key = vec3<i32>(k.d.cell.x * 64 + k.bc.x, k.d.cell.y * 64 + k.bc.y, 4111);
    let bh = st_hash(key, seed + 41u);
    if bh.x >= 0.85 * core {
        return o;
    }
    // The whole block in its district, clear of the arterials.
    let s_all = (STREET_ROAD_M + STREET_WALK_M) * 2.0 / km;
    let half = 0.5 * (k.g.size - vec2<f32>(s_all));
    let dc = ct_district(city_pos(cp.tile, centre), seed);
    let a_all = (ART_ROAD_M + ART_WALK_M) * 2.0 / km;
    if any(dc.cell != k.d.cell) || dc.art < 0.5 * a_all + length(half) {
        return o;
    }
    let room = 2.0 * half * km - vec2<f32>(2.0 * mix(6.0, 16.0, bh.z));
    if any(room < vec2<f32>(30.0)) {
        return o;
    }
    let h2 = st_hash(key + vec3<i32>(0, 0, 1), seed + 41u);
    var size = min(room, vec2<f32>(120.0));
    o.model = M_TAPER + min(u32(h2.x * 4.0), 3u);
    if o.model == M_TWIST || o.model == M_ROUND {
        size = vec2<f32>(min(size.x, size.y));
    }
    o.height = min(150.0 + 1350.0 * pow(bh.y, 1.6) * smoothstep(0.25, 1.0, core), 14.0 * min(size.x, size.y));
    o.centre = centre;
    o.size = size;
    o.rot = k.g.rot;
    o.hash = h2;
    o.ok = true;
    return o;
}

// The skybridge from block `k`'s megatower `a` to its neighbour's `b` along the grid's u axis (across
// the street between them), on 60 % of such pairs: from middle to middle (its ends inside the towers),
// 6 m wide, at 30-65 % of the lower one's height (`height` its elevation; the cull makes it 5 m deep).
fn ct_bridge(k: Block, a: Building, b: Building, km: f32, seed: u32) -> Building {
    var o: Building;
    let h = st_hash(vec3<i32>(k.d.cell.x * 64 + k.bc.x, k.d.cell.y * 64 + k.bc.y, 4113), seed + 41u);
    o.ok = a.ok && b.ok && h.x < 0.6;
    o.model = M_BRIDGE;
    o.centre = 0.5 * (a.centre + b.centre);
    o.size = vec2<f32>(length(b.centre - a.centre) * km, 6.0);
    o.height = min(a.height, b.height) * mix(0.3, 0.65, h.y);
    o.rot = a.rot;
    o.hash = h;
    return o;
}

// A flying lane's segment over the arterial line of `axis` nearest the point (axis 0 the lines of
// constant x, running along y): pieces of 128 face m of the bent line, their ends on it. Its middle
// (tile frame), true length, angle in the face frame (from u towards v), and its index along the line
// and the line's (global).
struct Lane {
    centre: vec2<f32>,
    len: f32,
    ang: f32,
    seg: i32,
    line: i32,
}

fn ct_lane(cp: CityPos, axis: i32, k: f32, seed: u32) -> Lane {
    // (across, along) in the axis's frame.
    let q = select(cp.q.yx, cp.q, axis == 0);
    let t = select(cp.tile.yx, cp.tile, axis == 0);
    let line = t.x * 8 + i32(round(q.x / DISTRICT_M));
    let seg = t.y * 32 + i32(floor(q.y / 128.0));
    let x0 = ct_bend(line, seg >> 2u, f32(seg & 3) * 128.0, axis, seed);
    let x1 = ct_bend(line, (seg + 1) >> 2u, f32((seg + 1) & 3) * 128.0, axis, seed);
    let base = f32(line - t.x * 8) * DISTRICT_M;
    let a0 = f32(seg - t.y * 32) * 128.0;
    let mid = vec2<f32>(base + 0.5 * (x0 + x1), a0 + 64.0);
    var o: Lane;
    o.centre = select(mid.yx, mid, axis == 0);
    o.len = length(vec2<f32>(x1 - x0, 128.0)) * k;
    let dir = select(vec2<f32>(128.0, x1 - x0), vec2<f32>(x1 - x0, 128.0), axis == 0);
    o.ang = atan2(dir.y, dir.x);
    o.seg = seg;
    o.line = line;
    return o;
}

// ---- domes (a level 4 civilization's colonies without air) ----------------------------------
// Three town districts in four stand under a glass dome on their middle, 135-180 face m in radius
// (inside their arterials, which bend at most 55 m), 0.8 of that high; their buildings fit beneath it;
// the arterials run between the domes as roads. A dome's district is the unbent cell of the point (the
// same as the bent one everywhere under a dome).

const DOME_R: f32 = 180.0;
const M_DOME: u32 = 14u;

// District `cell`'s dome: whether it has one and its radius's share of DOME_R.
fn ct_dome_scale(cell: vec2<i32>, seed: u32) -> f32 {
    let h = st_hash(vec3<i32>(cell, 4141), seed + 53u);
    return select(0.0, mix(0.75, 1.0, h.y), h.x < 0.75);
}

// A point's distance from its district's middle over its dome's radius (0 the middle, 1 the rim; 2
// where the district has none).
fn ct_dome_r(cp: CityPos, seed: u32) -> f32 {
    let c = floor(cp.q / DISTRICT_M);
    let s = ct_dome_scale(cp.tile * 8 + vec2<i32>(c), seed);
    if s <= 0.0 {
        return 2.0;
    }
    return length(cp.q - (c + 0.5) * DISTRICT_M) / (DOME_R * s);
}

// The share of a pixel of footprint `fp` (face m) under a dome: sharp near, the domes' share of the
// districts (pi 180^2 / 512^2, times 3 in 4 and their sizes' mean square) once it spans districts.
fn ct_dome_mask(cp: CityPos, fp: f32, seed: u32) -> f32 {
    let sharp = 1.0 - smoothstep(0.97, 1.0, ct_dome_r(cp, seed));
    return mix(sharp, 0.225, smoothstep(0.1 * DISTRICT_M, DISTRICT_M, fp));
}

// District `cell`'s dome (its true size from the metric `k`; `ok` false where it has none).
fn ct_dome(cp: CityPos, cell: vec2<i32>, k: f32, seed: u32) -> Building {
    var o: Building;
    let s = ct_dome_scale(cell, seed);
    o.ok = s > 0.0;
    o.model = M_DOME;
    o.centre = vec2<f32>(cell - cp.tile * 8) * DISTRICT_M + 0.5 * DISTRICT_M;
    o.size = vec2<f32>(2.0 * DOME_R * s * k);
    o.height = 0.8 * DOME_R * s * k;
    o.rot = vec2<f32>(1.0, 0.0);
    o.hash = vec4<f32>(0.5);
    return o;
}

// Building `b` under dome `d`: its footprint inside 92 % of the radius, its top 3 m under the dome
// (lowered to fit; none where too little room is left).
fn ct_under_dome(b: Building, d: Building, k: f32) -> Building {
    var o = b;
    let r = 0.5 * d.size.x;
    let reach = length(b.centre - d.centre) * k + 0.5 * length(b.size);
    let x = reach / (0.92 * r);
    if !b.ok || x >= 1.0 {
        o.ok = false;
        return o;
    }
    let room = d.height * sqrt(1.0 - x * x) - 3.0;
    o.height = min(b.height, room);
    o.ok = room > 5.0;
    return o;
}
