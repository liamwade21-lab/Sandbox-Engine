// The GPU scatter's compute passes (scatter.rs, scatter/gpu.rs):
// - place: a new cell's candidates from a hash of its key, kept with the job's chance, appended to
//   the cell's slot;
// - cull: every resident instance lifted onto the drawn ground (the triangle its terrain node draws
//   there, geomorph included), tested against the view, given its LOD by distance and appended to
//   its (model, LOD) bucket up to the bucket's cap;
// - finish: the indirect draws' instance counts and first instances from the buckets.

const SLOT_CAP: u32 = 8192u;
const GRID: u32 = 33u;
const PAGE_VERTS: u32 = 1089u;
const LEAF: u32 = 0x80000000u;
const NO_PAGE: u32 = 0x7fffffffu;

struct Body {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    centre: vec4<f32>,
    // mesh_far, band, impostor_far, thin_start
    lod: vec4<f32>,
    // thin_end, sink, time, lowest ground height (m)
    lod2: vec4<f32>,
    // slots, models, draws, buckets
    counts: vec4<u32>,
    sun: vec4<f32>,
    sun_color: vec4<f32>,
    levels: vec4<f32>,
    atm_flags: vec4<f32>,
}

struct Camera {
    clip_from_world: mat4x4<f32>,
    planes: array<vec4<f32>, 5>,
    viewport: vec4<f32>,
}

struct Job {
    slot: u32,
    face: u32,
    level: u32,
    row: u32,
    col: u32,
    keep: f32,
    seed: u32,
    pad: u32,
}

struct Cell {
    up: vec3<f32>,
    root: u32,
    off: vec2<u32>,
    // bits 0-7 the level difference to a coarser drawn node, bit 8 placed
    shift_live: u32,
    seed: u32,
}

struct PageInfo {
    t: vec3<f32>,
    split_m: f32,
    cam: vec3<f32>,
    pad: f32,
}

struct Model {
    half: f32,
    centre_y: f32,
    radius: f32,
    layer0: u32,
    crown_y: f32,
    crown_r: f32,
    mesh_bucket: u32,
    impostor_bucket: u32,
}

struct Bucket {
    base: u32,
    cap: u32,
    pad0: u32,
    pad1: u32,
}

struct Draw {
    bucket: u32,
    index_count: u32,
    first_index: u32,
    base_vertex: u32,
}

// A drawn instance (scatter_draw.wesl reads the same).
struct Inst {
    pos: vec3<f32>,
    // unorm16 x2: the fade (the far fade and the thinning), the mesh's share of the LOD band
    fades: u32,
    // the model's x axis and up (octahedral, snorm16 x2)
    xa: u32,
    up: u32,
    scale: f32,
    model: u32,
}

@group(0) @binding(0) var<uniform> body: Body;
@group(0) @binding(1) var<uniform> cam: Camera;
@group(0) @binding(2) var<storage, read> jobs: array<Job>;
// Per slot, the kept candidates: their number (bits 0-15) and model (16-23).
@group(0) @binding(3) var<storage, read_write> instances: array<u32>;
@group(0) @binding(4) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read> cells: array<Cell>;
@group(0) @binding(6) var<storage, read> tree: array<u32>;
@group(0) @binding(7) var<storage, read> page_pos: array<vec4<f32>>;
@group(0) @binding(8) var<storage, read> page_info: array<PageInfo>;
@group(0) @binding(9) var<storage, read> models: array<Model>;
@group(0) @binding(10) var<storage, read> buckets: array<Bucket>;
@group(0) @binding(11) var<storage, read> draws: array<Draw>;
@group(0) @binding(12) var<storage, read_write> counters: array<atomic<u32>>;
@group(0) @binding(13) var<storage, read_write> args: array<u32>;
@group(0) @binding(14) var<storage, read_write> drawn: array<Inst>;

fn hash_u32(x: u32) -> u32 {
    var h = x;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

// ---- place -----------------------------------------------------------------------------------

// Candidate `i` of a cell of seed `seed`: x its place in the cell (u16 x2), y the keep test's value,
// z its turn (bits 8-15), size (16-23) and a random byte (24-31). The cull pass re-derives it from
// the number the placement keeps.
fn candidate(seed: u32, i: u32) -> vec3<u32> {
    let h0 = hash_u32(seed ^ hash_u32(i * 0x9e3779b9u + 0x632be5abu));
    let h1 = hash_u32(h0 ^ 0x5bd1e995u);
    let h2 = hash_u32(h1 ^ 0x1b873593u);
    let h3 = hash_u32(h2 ^ 0xcc9e2d51u);
    return vec3<u32>((h0 & 0xffffu) | (h1 << 16u), h2, h3);
}

@compute @workgroup_size(64)
fn place(@builtin(global_invocation_id) gid: vec3<u32>) {
    let job = jobs[gid.y];
    let i = gid.x;
    let c = candidate(job.seed, i);
    if f32(c.y >> 8u) * (1.0 / 16777216.0) >= job.keep {
        return;
    }
    let k = atomicAdd(&counts[job.slot], 1u);
    if k >= SLOT_CAP {
        return;
    }
    let model = 0u;
    instances[job.slot * SLOT_CAP + k] = i | (model << 16u);
}

// ---- cull ------------------------------------------------------------------------------------

// A page's vertex as the ground draws it: its own position, moved towards the parent's surface by
// the geomorph (planet.wesl `vertex`; the parent's surface from this node's own grid as
// terrain.rs `build_node` makes `morph`).
fn drawn_vertex(page: u32, i: u32, j: u32, info: PageInfo) -> vec4<f32> {
    let base = page * PAGE_VERTS;
    let p = page_pos[base + j * GRID + i];
    if info.split_m <= 0.0 {
        return p;
    }
    var goal = p;
    let oi = (i & 1u) == 1u;
    let oj = (j & 1u) == 1u;
    if oi && !oj {
        goal = 0.5 * (page_pos[base + j * GRID + i - 1u] + page_pos[base + j * GRID + i + 1u]);
    } else if !oi && oj {
        goal = 0.5 * (page_pos[base + (j - 1u) * GRID + i] + page_pos[base + (j + 1u) * GRID + i]);
    } else if oi && oj {
        // The parent splits its cells along the (x+1, y)-(x, y+1) diagonal.
        goal = 0.5 * (page_pos[base + (j - 1u) * GRID + i + 1u] + page_pos[base + (j + 1u) * GRID + i - 1u]);
    }
    let d = length(p.xyz - info.cam);
    let morph = clamp((2.0 * info.split_m - d) / (0.6 * info.split_m), 0.0, 1.0);
    return p + (goal - p) * (1.0 - morph);
}

fn oct_encode(n: vec3<f32>) -> u32 {
    let a = n / (abs(n.x) + abs(n.y) + abs(n.z));
    var e = a.xy;
    if a.z < 0.0 {
        e = (1.0 - abs(a.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), a.xy >= vec2<f32>(0.0));
    }
    return pack2x16snorm(e);
}

fn append(bucket: u32, inst: Inst) {
    let k = atomicAdd(&counters[bucket], 1u);
    let b = buckets[bucket];
    if k < b.cap {
        drawn[b.base + k] = inst;
    }
}

@compute @workgroup_size(64)
fn cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let slot = gid.y;
    let i = gid.x;
    let cell = cells[slot];
    if (cell.shift_live & 0x100u) == 0u || i >= atomicLoad(&counts[slot]) {
        return;
    }
    let packed = instances[slot * SLOT_CAP + i];
    let c = candidate(cell.seed, packed & 0xffffu);
    let attr = ((packed >> 16u) & 0xffu) | (c.z & 0xffffff00u);
    // The instance's place in the cell, then in the drawn node under it.
    var f = (vec2<f32>(f32(c.x & 0xffffu), f32(c.x >> 16u)) + 0.5) / 65536.0;
    var e = tree[cell.root];
    var g: vec2<f32>;
    let shift = cell.shift_live & 0xffu;
    if shift > 0u {
        g = (vec2<f32>(cell.off) + f) / f32(1u << shift) * 32.0;
    } else {
        for (var depth = 0u; depth < 20u && (e & LEAF) == 0u; depth++) {
            let q = floor(f * 2.0);
            f = f * 2.0 - q;
            e = tree[e + u32(q.y) * 2u + u32(q.x)];
        }
        g = f * 32.0;
    }
    if (e & LEAF) == 0u {
        return;
    }
    let page = e & NO_PAGE;
    if page == NO_PAGE {
        return;
    }
    let info = page_info[page];
    // The triangle the mesh draws there: cells split along the (x+1, y)-(x, y+1) diagonal.
    let x0 = min(u32(g.x), 31u);
    let y0 = min(u32(g.y), 31u);
    let t = g - vec2<f32>(f32(x0), f32(y0));
    var p: vec4<f32>;
    if t.x + t.y <= 1.0 {
        p = drawn_vertex(page, x0, y0, info) * (1.0 - t.x - t.y)
            + drawn_vertex(page, x0 + 1u, y0, info) * t.x
            + drawn_vertex(page, x0, y0 + 1u, info) * t.y;
    } else {
        p = drawn_vertex(page, x0 + 1u, y0 + 1u, info) * (t.x + t.y - 1.0)
            + drawn_vertex(page, x0, y0 + 1u, info) * (1.0 - t.x)
            + drawn_vertex(page, x0 + 1u, y0, info) * (1.0 - t.y);
    }
    // No trees in the sea or on its shore.
    if p.w < body.lod2.w {
        return;
    }
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let up = cell.up;
    let pos = m * p.xyz + info.t - up * body.lod2.y;
    let model = attr & 0xffu;
    let md = models[model];
    let rnd = f32((attr >> 24u) & 0xffu) / 255.0;
    var scale = 0.8 + 0.4 * f32((attr >> 16u) & 0xffu) / 255.0;
    let d = length(pos);
    // Thinning: three trees in four fade out, the fourth grows to keep the cover.
    var fade = 1.0;
    let thin = clamp((d - body.lod.w) / max(body.lod2.x - body.lod.w, 1.0), 0.0, 1.0);
    if rnd >= 0.25 {
        fade = 1.0 - thin;
    } else {
        scale *= 1.0 + thin;
    }
    fade *= clamp((body.lod.z - d) / (body.lod.y * 4.0), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    // The view: the model's bounding sphere against the side and near planes.
    let bc = pos + up * md.centre_y * scale;
    let r = md.radius * scale;
    for (var k = 0u; k < 5u; k++) {
        let pl = cam.planes[k];
        if dot(pl.xyz, bc) + pl.w < -r {
            return;
        }
    }
    // Its turn about the up axis.
    let yaw = f32((attr >> 8u) & 0xffu) / 256.0 * 6.2831853;
    var ref_axis = vec3<f32>(0.0, 0.0, 1.0);
    if abs(up.z) > 0.9 {
        ref_axis = vec3<f32>(1.0, 0.0, 0.0);
    }
    let t0 = normalize(cross(up, ref_axis));
    let t1 = cross(up, t0);
    let xa = t0 * cos(yaw) + t1 * sin(yaw);
    // The mesh draws to mesh_far, the impostor from a band below it: the mesh where the pixel's
    // dither is under `mesh`, the impostor elsewhere (scatter_draw.wesl).
    let mesh = clamp((body.lod.x - d) / body.lod.y, 0.0, 1.0);
    let o = Inst(pos, pack2x16unorm(vec2<f32>(fade, mesh)), oct_encode(xa), oct_encode(up), scale, model);
    if mesh > 0.0 {
        append(md.mesh_bucket, o);
    }
    if mesh < 1.0 {
        append(md.impostor_bucket, o);
    }
}

// ---- finish ----------------------------------------------------------------------------------

@compute @workgroup_size(64)
fn finish(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = gid.x;
    if e >= body.counts.z {
        return;
    }
    let dr = draws[e];
    let b = buckets[dr.bucket];
    let n = min(atomicLoad(&counters[dr.bucket]), b.cap);
    args[e * 5u] = dr.index_count;
    args[e * 5u + 1u] = n;
    args[e * 5u + 2u] = dr.first_index;
    args[e * 5u + 3u] = dr.base_vertex;
    args[e * 5u + 4u] = b.base;
}
