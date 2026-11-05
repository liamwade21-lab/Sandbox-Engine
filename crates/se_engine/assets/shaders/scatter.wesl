// The GPU scatter's compute passes (scatter.rs, scatter/gpu.rs):
// - place: a new cell's candidates from a hash of its key, kept with the job's chance, appended to
//   the cell's slot;
// - cull: every resident instance lifted onto the drawn ground (the triangle its terrain node draws
//   there, geomorph included), tested against the view, given its LOD by distance and appended to
//   its (model, LOD) bucket up to the bucket's cap;
// - finish: the indirect draws' instance counts and first instances from the buckets.
// Where plants grow comes from the ground's own biome weights (surface_class.wesl): their noises once
// per candidate at placement, the rest every frame from the ground as it is drawn.

import super::surface_class::{terra_noise_at, terra_climate_noise_at, snow_vary_at, se_terra_snow_at, se_terra_climate_at, plants_ptf_at, se_terra_weights_at};
import super::vegetation::{map_noise, map_forest, veg_classes, lin_to_srgb255};
import super::settlement::settlements;
import super::city::{city_pos, ct_building};

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
    // x how plants are decided (scatter.rs VegKind: 0 none, 1 the test, 2 SE's terra bands), y the
    // noises' footprint on the unit sphere
    veg: vec4<f32>,
    // towards the star in the body frame
    sun_body: vec4<f32>,
    // SE's climate parameters (surface_tex.rs climate_uniform 0-7) and the biomes' flags
    climate: array<vec4<f32>, 8>,
    biome_flags: vec4<f32>,
    // veg 3 (a colour map's plants, scatter/earth.rs): the patch's centre (body frame) and the tan of
    // its half-angle, its two axes
    patch_c: vec4<f32>,
    patch_e1: vec4<f32>,
    patch_e2: vec4<f32>,
    // the wind (render frame, m/s)
    wind: vec4<f32>,
    // SE's forest slot colour (linear), the trees' tint gain (scatter_draw.wesl inst_tint)
    forest: vec4<f32>,
    tint_gain: vec4<f32>,
    // rgb SE's grass slot colour (linear), w the grass's range (m)
    grass: vec4<f32>,
    // x Earth's species' first model, y the flora's, z 1 when the flora is this body's
    species: vec4<u32>,
    // x the ground's relief (m)
    flora_k: vec4<f32>,
    // Cities: x the civilization's level (0 none), y its culture seed (fraction), z its grid turn,
    // w the radius (m); city2 x the buildings' range (m), y 1 under SE_CITY_TEST, z the ground's
    // lowest point and w its height range (m)
    city: vec4<f32>,
    city2: vec4<f32>,
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
    // the first model its candidates take, and how many in the high 16 bits
    models: u32,
    // 0 plants, 1 buildings; the cell's corner in the city layout's frame (its 4,096 m tile, the
    // face metres in it) and its edge (face metres)
    kind: u32,
    tile_x: i32,
    tile_y: i32,
    local_x: f32,
    local_y: f32,
    edge: f32,
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
    // the node's centre direction (body frame)
    dir: vec3<f32>,
    pad2: f32,
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
    // The model's mean colour (linear, from its impostor views) and how much of its quad it covers.
    mean: vec4<f32>,
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
// Per kept candidate, the plants' noises at its spot (f16 x4: the latitude noise, the humidity, the
// climate noise, the snow line's noise).
@group(0) @binding(15) var<storage, read_write> noises: array<vec2<u32>>;
// veg 3: the plants' map patch (scatter/earth.rs): the map's colour, the land in alpha.
@group(0) @binding(16) var veg_color: texture_2d<f32>;
@group(0) @binding(17) var veg_smp: sampler;
// Its lights map (Earth's towns for the cities): the brightest channel.
@group(0) @binding(18) var veg_lights: texture_2d<f32>;

// The towns' density at body-frame direction `dir`: from the lights map's patch on a map body (as
// the ground's from its lights tiles: planet.wesl), else the settlement field; 1 under SE_CITY_TEST.
fn town_at(dir: vec3<f32>) -> f32 {
    if body.city2.y > 0.5 {
        return 1.0;
    }
    if body.veg.x > 2.5 {
        let uv = patch_uv(dir);
        if any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
            return 0.0;
        }
        let lum = textureSampleLevel(veg_lights, veg_smp, uv, 0.0).r;
        let land = textureSampleLevel(veg_color, veg_smp, uv, 0.0).a;
        return smoothstep(0.04, 0.5, lum) * (1.0 - smoothstep(0.2, 0.6, 1.0 - land));
    }
    return settlements(dir, body.city.w, u32(body.city.x + 0.5), u32(body.city.y * 10000.0), 0.0).x;
}

// The forest, grassland and steppe shares at `uv` on the patch (none on water), as the ground
// classifies the colour it draws.
fn patch_classes(uv: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(veg_color, veg_smp, uv, 0.0);
    return veg_classes(lin_to_srgb255(c.rgb)) * c.a;
}

// The share of trees at body-frame direction `dir` from the map patch: its forests sharpened into
// woods with edges and clearings by the woods' noise `n` (vegetation.wesl, the map's texels are 1.2
// km on Earth), copses and hedgerows in grassland, lone trees on the steppe; fewer on steep ground.
// Where direction `dir` (body frame) falls on the map patch (0..1, outside when off it). A patch's
// corners are at most 0.66 rad from its centre (half-width 0.5 rad), so a point farther is off it,
// the far side of the body too (the projection would fold it back into the patch).
fn patch_uv(dir: vec3<f32>) -> vec2<f32> {
    let c = body.patch_c.xyz;
    if dot(dir, c) < 0.75 {
        return vec2<f32>(-1.0);
    }
    let q = dir / dot(dir, c) - c;
    return vec2<f32>(dot(q, body.patch_e1.xyz), dot(q, body.patch_e2.xyz)) / body.patch_c.w * 0.5 + 0.5;
}

// The trees' colour at `uv` for their tint: its square root in bytes 1-3 (the low byte is the
// model), 0 for none. In a forest the map's own colour; copses in grassland are darker than the
// fields round them, by Earth's forest over its farmland (24,39,12 over 39,45,13 sRGB, the Black
// Forest and the Paris basin); lone trees on the steppe olive, not the colour of the dry ground
// (about 60,64,36 on the Sahel's 154,132,91).
fn map_tint(uv: vec2<f32>) -> u32 {
    let c = textureSampleLevel(veg_color, veg_smp, uv, 0.0).rgb;
    let k = patch_classes(uv);
    let sum = k.x + k.y + k.z;
    var f = vec3<f32>(0.14, 0.22, 0.16);
    if sum > 0.01 {
        f = (k.x * vec3<f32>(1.0) + k.y * vec3<f32>(0.45, 0.77, 0.92) + k.z * vec3<f32>(0.14, 0.22, 0.16)) / sum;
    }
    return pack4x8unorm(vec4<f32>(0.0, sqrt(max(c * f, vec3<f32>(0.0))))) | 0x100u;
}

// The grass's share from the map patch: grassland and steppe, a little undergrowth in the woods,
// in patches by the woods' noise; none on rock steeper than about 14 degrees (Megaton Rainfall's rule).
fn map_grass(dir: vec3<f32>, slope: f32, n: f32) -> f32 {
    let uv = patch_uv(dir);
    if any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        return 0.0;
    }
    let k = patch_classes(uv);
    return clamp(k.y + 0.7 * k.z + 0.35 * k.x, 0.0, 1.0) * mix(0.55, 1.0, n) * (1.0 - smoothstep(0.03, 0.08, slope));
}

// A colour (linear) for the instances' tint (scatter_draw.wesl inst_tint): its square root in bytes
// 1-3 (the low byte is the model), never 0.
fn pack_tint(c: vec3<f32>) -> u32 {
    return pack4x8unorm(vec4<f32>(0.0, sqrt(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))))) | 0x100u;
}

// (x the share, y the part of it that is the dry land's lone trees: acacias and shrubs.)
fn map_share(dir: vec3<f32>, slope: f32, n: f32) -> vec2<f32> {
    let uv = patch_uv(dir);
    if any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        return vec2<f32>(0.0);
    }
    let k = patch_classes(uv);
    let flat = 1.0 - smoothstep(0.18, 0.45, slope);
    let copses = k.y * 0.25 * smoothstep(0.6, 0.72, n) * flat;
    let lone = k.z * 0.05 * flat;
    let share = max(map_forest(k.x, n, slope), copses + lone);
    return vec2<f32>(share, select(0.0, lone / max(copses + lone, 1e-4), copses + lone >= share));
}

fn mix32(x: u32) -> u32 {
    var h = x * 0x9e3779b9u;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    return h ^ (h >> 16u);
}

// A species from a family (first model, count) by the random number `u` (0..1).
fn in_family(first: u32, count: u32, u: f32) -> u32 {
    return first + min(u32(u * f32(count)), count - 1u);
}

// Earth's species at body-frame direction `dir`, `h` m above the radius (`dry` the share of the
// dry land's lone trees, `u`, `u2` random): the coldness is the latitude plus 9 degrees a km of
// height; conifers in the cold (boreal forests, mountains), palms in the tropics (most on the
// coasts), acacias on the warm steppe and shrubs on the cooler one, MR's broadleaf elsewhere.
fn earth_species(dir: vec3<f32>, h: f32, dry: f32, u: f32, u2: f32) -> u32 {
    let e0 = body.species.x;
    let cold = degrees(asin(clamp(abs(dir.y), 0.0, 1.0))) + h * 0.009;
    if u < dry {
        if cold < 32.0 {
            return e0 + 5u;
        }
        return in_family(e0 + 6u, 2u, u2);
    }
    let v = (u - dry) / max(1.0 - dry, 1e-4);
    let conifer = smoothstep(42.0, 58.0, cold);
    let coast = 1.0 - smoothstep(5.0, 60.0, h);
    let palm = (1.0 - smoothstep(16.0, 26.0, cold)) * mix(0.12, 0.7, coast) * (1.0 - conifer);
    if v < conifer {
        return in_family(e0, 3u, u2);
    }
    if v < conifer + palm {
        return in_family(e0 + 3u, 2u, u2);
    }
    return in_family(0u, e0, u2);
}

// A generated planet's species (its flora, trees.rs: forest 0-2, highland 3-4, dry 5-6, shore 7)
// at `h` m above the radius: highland trees over the upper part of the relief, shore trees near the
// sea, dry land trees where the steppe's lone trees stand.
fn flora_species(h: f32, dry: f32, u: f32, u2: f32) -> u32 {
    let f0 = body.species.y;
    if u < dry {
        return in_family(f0 + 5u, 2u, u2);
    }
    let v = (u - dry) / max(1.0 - dry, 1e-4);
    // (Measured from SE's sea level: climate[5].z is it with an ocean and 0 without, climate[0].x
    // always, climate[4].z the relief's scale.)
    let above = h + (body.climate[5].z - body.climate[0].x) * body.climate[4].z;
    let high = smoothstep(0.3, 0.6, above / max(body.flora_k.x, 100.0));
    var shore = 0.0;
    if body.lod2.w > -1e8 {
        shore = (1.0 - smoothstep(3.0, 30.0, h - body.lod2.w)) * 0.6;
    }
    if v < high {
        return in_family(f0 + 3u, 2u, u2);
    }
    if v < high + shore * (1.0 - high) {
        return f0 + 7u;
    }
    return in_family(f0, 3u, u2);
}

// The unit-cube point of face `f` at face coordinates (u right, v down), as planet_tiles.rs uv_to_cube.
fn uv_to_cube(f: u32, u: f32, v: f32) -> vec3<f32> {
    let s = 2.0 * u - 1.0;
    let t = 2.0 * v - 1.0;
    switch f {
        case 0u: { return vec3<f32>(1.0, -t, -s); }
        case 1u: { return vec3<f32>(-1.0, -t, s); }
        case 2u: { return vec3<f32>(s, 1.0, t); }
        case 3u: { return vec3<f32>(s, -1.0, -t); }
        case 4u: { return vec3<f32>(s, -t, 1.0); }
        default: { return vec3<f32>(-s, -t, -1.0); }
    }
}

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

// ---- buildings (cities) ----------------------------------------------------------------------

// The buildings' candidates: the points of a 90 x 90 grid over the cell (about 9 m apart in an 800 m
// cell); a point keeps the building of the lot whose middle falls in its square (one a lot).
const BGRID: u32 = 90u;

// True metres per face metre at face coordinates (u, v) (the mean of the two axes): terrain.rs's
// `city_frame` at a node's centre.
fn face_metric(u: f32, v: f32) -> f32 {
    let s = 2.0 * u - 1.0;
    let t = 2.0 * v - 1.0;
    let q = 1.0 + s * s + t * t;
    return 2.0 * (sqrt(1.0 + t * t) + sqrt(1.0 + s * s)) / (3.14159265 * q);
}

fn place_building(job: Job, i: u32) {
    if i >= BGRID * BGRID {
        return;
    }
    let gi = vec2<u32>(i % BGRID, i / BGRID);
    let f = (vec2<f32>(gi) + 0.5) / f32(BGRID);
    let corner = vec2<f32>(job.local_x, job.local_y);
    let tile0 = vec2<i32>(job.tile_x, job.tile_y);
    let cp = city_pos(tile0, corner + f * job.edge);
    let n = 1.0 / f32(1u << job.level);
    let uv = (vec2<f32>(f32(job.col), f32(job.row)) + f) * n;
    let k = face_metric(uv.x, uv.y);
    // The town's density at the point (as the ground's; it varies over km).
    let urban = town_at(normalize(uv_to_cube(job.face, uv.x, uv.y)));
    if urban < 0.06 {
        return;
    }
    let b = ct_building(cp, k, urban, u32(body.city.y * 10000.0) + 100u, body.city.z);
    if !b.ok {
        return;
    }
    // Its middle in this point's square of the cell?
    let rel = vec2<f32>(cp.tile - tile0) * 4096.0 + b.centre - corner;
    let fc = rel / job.edge;
    if any(floor(fc * f32(BGRID)) != vec2<f32>(gi)) {
        return;
    }
    let ks = atomicAdd(&counts[job.slot], 1u);
    if ks >= SLOT_CAP {
        return;
    }
    instances[job.slot * SLOT_CAP + ks] = i | ((job.models & 0xffffu) << 16u);
    // Its record: its middle in the cell (u16 x2); its width and depth (0.5 m steps), height (the
    // square root of a share of 600 m, a byte) and turn (a byte over -pi..pi).
    let w = u32(clamp(round(b.size.x * 2.0), 1.0, 255.0));
    let d = u32(clamp(round(b.size.y * 2.0), 1.0, 255.0));
    let hc = u32(clamp(round(sqrt(b.height / 600.0) * 255.0), 1.0, 255.0));
    let ac = u32(clamp(round((atan2(b.rot.y, b.rot.x) / 6.2831853 + 0.5) * 255.0), 0.0, 255.0));
    let q = vec2<u32>(clamp(fc * 65536.0, vec2<f32>(0.0), vec2<f32>(65535.0)));
    noises[job.slot * SLOT_CAP + ks] = vec2<u32>(q.x | (q.y << 16u), w | (d << 8u) | (hc << 16u) | (ac << 24u));
}

@compute @workgroup_size(64)
fn place(@builtin(global_invocation_id) gid: vec3<u32>) {
    let job = jobs[gid.y];
    let i = gid.x;
    if job.kind == 1u {
        place_building(job, i);
        return;
    }
    let c = candidate(job.seed, i);
    if f32(c.y >> 8u) * (1.0 / 16777216.0) >= job.keep {
        return;
    }
    // In a civilization's towns (the settlement field, as the ground's streets) a few garden trees
    // and no grass: the streets, lots and buildings take the ground.
    if body.city.x > 0.5 {
        let n0 = 1.0 / f32(1u << job.level);
        let f0 = (vec2<f32>(f32(c.x & 0xffffu), f32(c.x >> 16u)) + 0.5) / 65536.0;
        let urban = town_at(normalize(uv_to_cube(job.face, (f32(job.col) + f0.x) * n0, (f32(job.row) + f0.y) * n0)));
        let streets = smoothstep(0.06, 0.2, urban);
        let grass_m = job.models >> 16u > 1u && (job.models & 0xffffu) >= u32(body.veg.z);
        let keep_town = select(1.0 - 0.9 * streets, 1.0 - streets, grass_m);
        if f32(c.z & 0xffu) / 255.0 >= keep_town {
            return;
        }
    }
    let k = atomicAdd(&counts[job.slot], 1u);
    if k >= SLOT_CAP {
        return;
    }
    // One of the job's models (the trees, or the grass tufts).
    let model = (job.models & 0xffffu) + (c.z >> 24u) % max(job.models >> 16u, 1u);
    instances[job.slot * SLOT_CAP + k] = i | (model << 16u);
    if body.veg.x > 2.5 {
        // The woods' noise at the candidate's spot, once.
        let n = 1.0 / f32(1u << job.level);
        let f = (vec2<f32>(f32(c.x & 0xffffu), f32(c.x >> 16u)) + 0.5) / 65536.0;
        let dir = normalize(uv_to_cube(job.face, (f32(job.col) + f.x) * n, (f32(job.row) + f.y) * n));
        noises[job.slot * SLOT_CAP + k] = vec2<u32>(pack2x16float(vec2<f32>(map_noise(dir, body.centre.w), 0.0)), 0u);
    } else if body.veg.x > 1.5 {
        // The plants' noises that vary only over kilometres, once: at the candidate's spot on the sphere.
        let n = 1.0 / f32(1u << job.level);
        let f = (vec2<f32>(f32(c.x & 0xffffu), f32(c.x >> 16u)) + 0.5) / 65536.0;
        let dir = normalize(uv_to_cube(job.face, (f32(job.col) + f.x) * n, (f32(job.row) + f.y) * n));
        let fp = body.veg.y;
        let rnd = body.climate[6].xyz;
        let tn = terra_noise_at(dir, fp, rnd, true);
        let cv = terra_climate_noise_at(dir, fp, rnd, body.climate[7], body.climate[5].y);
        let vary = snow_vary_at(dir, fp, rnd);
        noises[job.slot * SLOT_CAP + k] = vec2<u32>(pack2x16float(vec2<f32>(tn.x, tn.z)), pack2x16float(vec2<f32>(cv, vary)));
    }
}

// SE's steppe, grass and forest weights (slots 9-11) at body-frame direction `dir`, `h` m above
// the radius, on ground of `slope` (1 - cos of its tilt), from the candidate's stored noises: the
// ground's own weights there (planet.wesl biome_albedo's SE terra path, sharp as at its vertices).
fn plant_weights(dir: vec3<f32>, h: f32, slope: f32, nz: vec2<u32>) -> vec3<f32> {
    let a = unpack2x16float(nz.x);
    let b = unpack2x16float(nz.y);
    let fp = body.veg.y;
    let hv = body.climate[5].z + h / max(body.climate[4].z, 1.0);
    let s = se_terra_snow_at(dir, hv, slope, a.x, b.y, body.climate[0], body.climate[1], body.climate[2], body.climate[6].w > 0.0, body.sun_body.xyz);
    let c = se_terra_climate_at(slope, s, 0.0, b.x, body.climate[0], body.climate[1]);
    let ptf = plants_ptf_at(dir, fp);
    // (The weights leave out n.y, the colour's vary: not stored.)
    let m = se_terra_weights_at(dir, hv, slope, fp, c, 0.0, vec3<f32>(a.x, 0.0, a.y), ptf, body.climate[0], body.climate[2], body.climate[3], body.climate[4], true, false);
    return vec3<f32>(m.w[9], m.w[10], m.w[11]);
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

// A point of a cell (`f` 0..1 over it) lifted onto the drawn ground: the triangle the mesh draws
// there (its corners `va`, `vb`, `vc`, node-local), the point `p` (height in w), the node's page.
struct Lift {
    ok: bool,
    p: vec4<f32>,
    va: vec4<f32>,
    vb: vec4<f32>,
    vc: vec4<f32>,
    page: u32,
    // The page's vertex step along the face's u and v there (node-local).
    du: vec3<f32>,
    dv: vec3<f32>,
}

fn lift(cell: Cell, f_in: vec2<f32>) -> Lift {
    var o: Lift;
    o.ok = false;
    var f = f_in;
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
        return o;
    }
    let page = e & NO_PAGE;
    if page == NO_PAGE {
        return o;
    }
    let info = page_info[page];
    // The triangle the mesh draws there: cells split along the (x+1, y)-(x, y+1) diagonal.
    let x0 = min(u32(g.x), 31u);
    let y0 = min(u32(g.y), 31u);
    let t = g - vec2<f32>(f32(x0), f32(y0));
    var va: vec4<f32>;
    var vb: vec4<f32>;
    var vc: vec4<f32>;
    var w: vec3<f32>;
    if t.x + t.y <= 1.0 {
        va = drawn_vertex(page, x0, y0, info);
        vb = drawn_vertex(page, x0 + 1u, y0, info);
        vc = drawn_vertex(page, x0, y0 + 1u, info);
        w = vec3<f32>(1.0 - t.x - t.y, t.x, t.y);
    } else {
        va = drawn_vertex(page, x0 + 1u, y0 + 1u, info);
        vb = drawn_vertex(page, x0, y0 + 1u, info);
        vc = drawn_vertex(page, x0 + 1u, y0, info);
        w = vec3<f32>(t.x + t.y - 1.0, 1.0 - t.x, 1.0 - t.y);
    }
    o.p = va * w.x + vb * w.y + vc * w.z;
    o.va = va;
    o.vb = vb;
    o.vc = vc;
    o.page = page;
    let base = page * PAGE_VERTS + y0 * GRID + x0;
    o.du = page_pos[base + 1u].xyz - page_pos[base].xyz;
    o.dv = page_pos[base + GRID].xyz - page_pos[base].xyz;
    o.ok = true;
    return o;
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
    if ((packed >> 16u) & 0xffu) == body.species.w && body.city.x > 0.5 {
        cull_building(slot, i, cell);
        return;
    }
    let c = candidate(cell.seed, packed & 0xffffu);
    let attr = ((packed >> 16u) & 0xffu) | (c.z & 0xffffff00u);
    // The instance's place in the cell, then in the drawn node under it.
    let lf = lift(cell, (vec2<f32>(f32(c.x & 0xffffu), f32(c.x >> 16u)) + 0.5) / 65536.0);
    if !lf.ok {
        return;
    }
    let p = lf.p;
    let va = lf.va;
    let vb = lf.vb;
    let vc = lf.vc;
    let info = page_info[lf.page];
    // No trees in the sea or on its shore.
    if p.w < body.lod2.w {
        return;
    }
    // Where plants grow: the ground's weights at this point of the drawn ground.
    var share = 1.0;
    var tint = 0u;
    var model = attr & 0xffu;
    // Grass tufts (models from veg.z on): lit as level ground, half the crowns' tint for the same
    // gain (matched by eye to Earth's forest floor, 2026-11-04).
    let grass = (attr & 0xffu) >= u32(body.veg.z) && body.veg.w > 0.5;
    if body.veg.x > 1.5 {
        let rd = body.centre.w;
        let dir = normalize(info.dir * rd + p.xyz);
        let tri_n = cross(vb.xyz - va.xyz, vc.xyz - va.xyz);
        let slope = 1.0 - clamp(abs(dot(normalize(tri_n), dir)), 0.0, 1.0);
        let rs = mix32(c.x ^ (c.z * 0x632be5abu));
        let u = f32(rs & 0xffffu) / 65536.0;
        let u2 = f32(rs >> 16u) / 65536.0;
        if body.veg.x > 2.5 {
            let n = unpack2x16float(noises[slot * SLOT_CAP + i].x).x;
            if grass {
                share = map_grass(dir, slope, n);
                // Shaded under the woods as the forest floor (planet.wesl FLOOR_DARK).
                let k = patch_classes(patch_uv(dir)).x;
                let shade = 1.0 - 0.45 * map_forest(k, n, slope);
                tint = pack_tint(textureSampleLevel(veg_color, veg_smp, patch_uv(dir), 0.0).rgb * (0.5 * shade));
            } else {
                let ms = map_share(dir, slope, n);
                share = ms.x;
                tint = map_tint(patch_uv(dir));
                model = earth_species(dir, p.w, ms.y, u, u2);
            }
        } else {
            let w = plant_weights(dir, p.w, slope, noises[slot * SLOT_CAP + i]);
            if grass {
                share = clamp(w.y + 0.7 * w.x + 0.35 * w.z, 0.0, 1.0) * (1.0 - smoothstep(0.03, 0.08, slope));
                tint = pack_tint(body.grass.rgb * (0.5 * (1.0 - 0.45 * w.z)));
            } else {
                // The forest, and the steppe's lone trees (its dry species), once the planet's
                // flora is made (until then none).
                if body.species.z == 0u {
                    return;
                }
                let lone = w.x * 0.06;
                share = max(w.z, lone);
                tint = pack_tint(body.forest.rgb);
                model = flora_species(p.w, select(0.0, 1.0, lone > w.z), u, u2);
            }
        }
        // No grass within 8 m (height) of the sea.
        if grass && p.w < body.lod2.w + 8.0 {
            return;
        }
        // A second number of the candidate against the share, with a soft edge: trees near it
        // stand smaller (a dithered fade showed as hatching), so they grow and shrink rather than
        // switch where the ground's detail changes the share a little.
        // (d over 0..0.92 so a full share keeps every candidate at full size; a share of 0 none.)
        let d = f32((c.y >> 0u) & 0xffu) / 255.0 * 0.92;
        share = clamp((share - d) / 0.08, 0.0, 1.0);
        if share <= 0.0 {
            return;
        }
        share = 0.4 + 0.6 * share;
    }
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let up = cell.up;
    let pos = m * p.xyz + info.t - up * body.lod2.y;
    let md = models[model];
    let rnd = f32((attr >> 24u) & 0xffu) / 255.0;
    var scale = 0.8 + 0.4 * f32((attr >> 16u) & 0xffu) / 255.0;
    let d = length(pos);
    var fade = 1.0;
    if grass {
        // The grass shrinks to nothing over its last 10 m (Megaton Rainfall's).
        scale *= share * clamp((body.grass.w - d) / 10.0, 0.0, 1.0);
        if scale <= 0.0 {
            return;
        }
    } else {
    // Thinning: three trees in four fade out, the fourth grows to keep the cover.
    let thin = clamp((d - body.lod.w) / max(body.lod2.x - body.lod.w, 1.0), 0.0, 1.0);
    if rnd >= 0.25 {
        fade = 1.0 - thin;
    } else {
        scale *= 1.0 + thin;
    }
    // The hand-over to the ground's canopy (planet.wesl) over the last quarter of the range: the
    // trees shrink and thin out as the canopy comes in (a sharp end showed as a circle round the
    // camera from above).
    let far_k = clamp((body.lod.z - d) / (body.lod.z * 0.25), 0.0, 1.0);
    fade *= smoothstep(0.0, 0.6, far_k);
    scale *= share * mix(0.5, 1.0, far_k);
    }
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
    let o = Inst(pos, pack2x16unorm(vec2<f32>(fade, mesh)), oct_encode(xa), oct_encode(up), scale, model | tint);
    if mesh > 0.0 {
        append(md.mesh_bucket, o);
    }
    if mesh < 1.0 {
        append(md.impostor_bucket, o);
    }
}

// A building: its record lifted onto the drawn ground, standing on a plinth down to the lowest ground
// under its footprint, turned with its street grid, faded out over the last 15 % of the range.
fn cull_building(slot: u32, i: u32, cell: Cell) {
    let rec = noises[slot * SLOT_CAP + i];
    let lf = lift(cell, (vec2<f32>(f32(rec.x & 0xffffu), f32(rec.x >> 16u)) + 0.5) / 65536.0);
    if !lf.ok {
        return;
    }
    let p = lf.p;
    let info = page_info[lf.page];
    // The land as the ground's town needs it (planet.wesl's civ block: not the highlands, off the
    // sea; on a map body its water mask, as the ground's: Earth's coastal towns stand a few metres up).
    let sea = body.lod2.w > -1e8;
    let hr = (p.w - select(body.city2.z, 0.0, sea)) / max(body.city2.w, 100.0);
    var land = 1.0 - smoothstep(select(0.55, 0.25, sea), select(0.8, 0.5, sea), hr);
    if body.veg.x > 2.5 {
        let dir_l = normalize(info.dir * body.centre.w + p.xyz);
        let uv = patch_uv(dir_l);
        land *= select(0.0, textureSampleLevel(veg_color, veg_smp, uv, 0.0).a, all(uv > vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0)));
    } else if sea {
        land *= smoothstep(2.0, 20.0, p.w);
    }
    if smoothstep(0.03, 0.3, land) < 0.5 {
        return;
    }
    let w = f32(rec.y & 0xffu) * 0.5;
    let dpt = f32((rec.y >> 8u) & 0xffu) * 0.5;
    let hq = f32((rec.y >> 16u) & 0xffu) / 255.0;
    let h = hq * hq * 600.0;
    let ang = (f32(rec.y >> 24u) / 255.0 - 0.5) * 6.2831853;
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let up = cell.up;
    // The plinth: down by the footprint's half width times the ground's slope, and a metre.
    let rd = body.centre.w;
    let dir = normalize(info.dir * rd + p.xyz);
    let tri_n = normalize(cross(lf.vb.xyz - lf.va.xyz, lf.vc.xyz - lf.va.xyz));
    let cs = clamp(abs(dot(tri_n, dir)), 0.2, 1.0);
    let plinth = 1.0 + 0.5 * max(w, dpt) * sqrt(1.0 - cs * cs) / cs;
    let pos = m * p.xyz + info.t - up * (body.lod2.y + plinth);
    let scale = h + plinth;
    let dist = length(pos);
    let range = body.city2.x;
    let fade = clamp((range - dist) / (range * 0.15), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    let r = 0.5 * sqrt(w * w + dpt * dpt + scale * scale);
    let bc = pos + up * (0.5 * scale);
    for (var k = 0u; k < 5u; k++) {
        let pl = cam.planes[k];
        if dot(pl.xyz, bc) + pl.w < -r {
            return;
        }
    }
    // Its x axis: the street grid's u axis (turned from the face's u towards its v), on the ground.
    let fu = normalize(m * lf.du);
    let fv = normalize(m * lf.dv);
    var xa = cos(ang) * fu + sin(ang) * fv;
    xa = normalize(xa - up * dot(xa, up));
    let hash = mix32(rec.x ^ (rec.y * 0x9e3779b9u)) & 0xffffu;
    let model = body.species.w;
    let o = Inst(pos, pack2x16float(vec2<f32>(w, dpt)), oct_encode(xa), oct_encode(up), scale, model | (u32(fade * 255.0) << 8u) | (hash << 16u));
    append(models[model].mesh_bucket, o);
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
