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
import super::city::{city_pos, ct_building, ct_hut, Building, CityPos, ct_block, ct_block_mid, ct_arcology, ct_on_arcology, ct_mega, ct_bridge, ct_lane, ct_temple, ct_district, ct_dome, ct_under_dome, M_FUTURE, M_ARCO, M_BRIDGE, M_LANE, M_TEMPLE, M_LANDMARK, M_DOME, ct_grid, ct_local, ct_unlocal, Grid, ct_paint, ct_slot_car, SlotCar, ct_street_key, ct_art_key, ct_art_line, ct_art_at, ct_street_hash, STREET_ROAD_M, STREET_WALK_M, ART_ROAD_M, ART_WALK_M, RURAL_ROAD_M};

const SLOT_CAP: u32 = 8192u;
// The library's building models from the box on (ScatterLibrary::building): the box, the villages'
// hut and longhouse, level 3's future box, four towers, the arcology, the skybridge, the flying lane,
// the hero models (level 1's temple and watchtower, level 3's landmark), a colony's dome.
const BUILDING_MODELS: u32 = 15u;
// The street objects, the library's models after those (library.rs STREET_PROPS, STREET_CARS: keep
// them in step): three lamps, two benches, two bins, a hydrant, two traffic lights, the cars.
const S_LAMP: u32 = 0u;
const S_BENCH: u32 = 3u;
const S_BIN: u32 = 5u;
const S_HYDRANT: u32 = 7u;
const S_SIGNAL: u32 = 8u;
const S_CAR: u32 = 10u;
const S_CARS: u32 = 14u;
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
    // x the ground's relief (m), y the far buildings' range (m)
    flora_k: vec4<f32>,
    // Cities: x the civilization's level (0 none), y its culture seed (fraction), z its grid turn,
    // w the radius (m); city2 x the buildings' range (m), y 1 under SE_CITY_TEST, z the ground's
    // lowest point and w its height range (m)
    city: vec4<f32>,
    city2: vec4<f32>,
    // The street objects: x their range (m), y the ground's clock for the traffic (its `globals.time`)
    street: vec4<f32>,
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
    // Its corner in the city layout's frame and edge (`Job`), its metric at its middle (the cars, the
    // probe), its cube face (the probe's building ids).
    tile: vec2<i32>,
    local: vec2<f32>,
    edge: f32,
    k: f32,
    face: f32,
    pad1: f32,
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
    // x 1 for a broadleaf tree (its leaves follow the seasons: scatter_draw.wesl tree_season)
    flags: vec4<u32>,
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

// The Viltrumite mod's solid buildings (docs/spec-mods-viltrumite-powers.md F12, scatter/probe.rs). Both exist with
// the mod off (the pipelines are built at start), unused then: `body.street.z` is 0.
// The broken table (step 10 uses it; made empty, count 0): its header, then entries in 64 m squares.
struct BrokenHead {
    count: u32,
    clock: f32,
    pad0: u32,
    pad1: u32,
    // the capsule (render frame) its hiding is tested in, a.w its margin
    a: vec4<f32>,
    b: vec4<f32>,
}
struct BrokenEntry {
    // face metres x 4 (the probe's id), the face and the model, the time it broke
    x: i32,
    y: i32,
    face_model: u32,
    t0: f32,
}
struct BrokenTable {
    head: BrokenHead,
    entries: array<BrokenEntry>,
}
// The probe: the buildings inside a capsule round him, appended by `cull_building` (dropped past `cap`; `count` goes
// on counting), copied to the CPU every frame it's on. `seq` names the frame (the CPU's ring of cameras and bodies).
struct ProbeHead {
    count: atomic<u32>,
    seq: u32,
    cap: u32,
    // 1 the capsule after the range fade (as drawn), 2 the near and far lists in full (measuring)
    mode: u32,
    // its ends (render frame), a.w the margin round its axis (m) beyond each building's half diagonal
    a: vec4<f32>,
    b: vec4<f32>,
}
struct ProbeRec {
    inst: Inst,
    // the face and its face metres x 4 (the same building from a near and a far cell: its record's hash differs)
    id: vec2<i32>,
    // the face, the model (from the box) << 8, the cell's kind (1 near, 2 far) << 16 (`meta` is a reserved word)
    tag: u32,
    // the cell's metric (true m per face m)
    k: f32,
}
struct ProbeList {
    head: ProbeHead,
    recs: array<ProbeRec>,
}
@group(0) @binding(19) var<storage, read> broken: BrokenTable;
@group(0) @binding(20) var<storage, read_write> probe: ProbeList;

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

// Where a point of the city frame (tile `tile`, face m `p` in it) falls in the job's cell: 0..1 over
// the cell.
fn cell_frac(job: Job, tile: vec2<i32>, p: vec2<f32>) -> vec2<f32> {
    let rel = vec2<f32>(tile - vec2<i32>(job.tile_x, job.tile_y)) * 4096.0 + p - vec2<f32>(job.local_x, job.local_y);
    return rel / job.edge;
}

// Whether the grid point `gi` is the one whose square holds the cell point `fc` (so each object of the
// layout is placed by one point alone).
fn in_square(gi: vec2<u32>, fc: vec2<f32>) -> bool {
    return all(floor(fc * f32(BGRID)) == vec2<f32>(gi));
}

// The body-frame direction of a point of the city frame (for the town's density there).
fn city_dir(job: Job, tile: vec2<i32>, p: vec2<f32>) -> vec3<f32> {
    let uv = (vec2<f32>(f32(job.col), f32(job.row)) + cell_frac(job, tile, p)) / f32(1u << job.level);
    return normalize(uv_to_cube(job.face, uv.x, uv.y));
}

// Appends one object of the layout to the job's slot: the model (an offset from the box) and its record
// (its middle in the cell, u16 x2, and `rec` the rest).
fn put_record(job: Job, i: u32, model: u32, fc: vec2<f32>, rec: u32) {
    let ks = atomicAdd(&counts[job.slot], 1u);
    if ks >= SLOT_CAP {
        return;
    }
    instances[job.slot * SLOT_CAP + ks] = i | (((job.models & 0xffu) + model) << 16u) | (job.kind << 24u);
    let q = vec2<u32>(clamp(fc * 65536.0, vec2<f32>(0.0), vec2<f32>(65535.0)));
    noises[job.slot * SLOT_CAP + ks] = vec2<u32>(q.x | (q.y << 16u), rec);
}

// A building's record: its width and depth (0.5 m steps; the arcology 4 m, the skybridge's length 1 m),
// height (the square root of a share of 1,600 m, a byte) and turn (a byte over -pi..pi). Placed by the
// grid point whose square holds its middle; the far class takes the tall ones alone (the skyline).
fn put_building(job: Job, i: u32, gi: vec2<u32>, tile: vec2<i32>, b: Building) {
    if !b.ok || (job.kind == 2u && b.height < 25.0) {
        return;
    }
    let fc = cell_frac(job, tile, b.centre);
    if !in_square(gi, fc) {
        return;
    }
    let big = b.model == M_ARCO || b.model == M_LANDMARK || b.model == M_DOME;
    let step = select(select(2.0, 1.0, b.model == M_BRIDGE), 0.25, big);
    let w = u32(clamp(round(b.size.x * step), 1.0, 255.0));
    let d = u32(clamp(round(b.size.y * select(2.0, 0.25, big)), 1.0, 255.0));
    let hc = u32(clamp(round(sqrt(b.height / 1600.0) * 255.0), 1.0, 255.0));
    let ac = u32(clamp(round((atan2(b.rot.y, b.rot.x) / 6.2831853 + 0.5) * 255.0), 0.0, 255.0));
    put_record(job, i, b.model, fc, w | (d << 8u) | (hc << 16u) | (ac << 24u));
}

// Level 3's objects at a point: the flying lanes' segments over the arterials, the district's arcology,
// the block's megatower and its skybridge, else the lot's building (the towns' box, taller, in the
// future facades).
fn place_future(job: Job, i: u32, gi: vec2<u32>, cp: CityPos, k: f32, urban: f32, seed: u32, level: u32) {
    // A colony without air (the scatter's `flora_k.w`): each town district under its dome, where the
    // town is dense enough at its middle; the buildings that fit beneath it; no lanes, arcologies or
    // skybridges.
    if body.flora_k.w > 0.5 {
        let dome = ct_dome(cp, ct_district(cp, seed).cell, k, seed);
        if !dome.ok || town_at(city_dir(job, cp.tile, dome.centre)) < 0.06 {
            return;
        }
        put_building(job, i, gi, cp.tile, dome);
        let blk = ct_block(cp, seed, body.city.z);
        let mega = ct_mega(cp, blk, k, town_at(city_dir(job, cp.tile, ct_block_mid(cp, blk))), seed);
        if mega.ok {
            let m = ct_under_dome(mega, dome, k);
            if m.ok {
                put_building(job, i, gi, cp.tile, m);
                return;
            }
        }
        var b = ct_building(cp, k, urban, seed, body.city.z);
        b.model = M_FUTURE;
        b.height *= 1.4;
        put_building(job, i, gi, cp.tile, ct_under_dome(b, dome, k));
        return;
    }
    // The lanes (dense town): two heights a segment (an instance each, its two ribbons the two ways).
    if urban >= 0.3 {
        for (var axis = 0; axis < 2; axis++) {
            let ln = ct_lane(cp, axis, k, seed);
            let fc = cell_frac(job, cp.tile, ln.centre);
            // Over every other arterial (about a kilometre apart).
            if (ln.line & 1) == 0 && in_square(gi, fc) {
                // Its record: length (1 m), the angle's offset from the axis (6 bits over +-0.2 rad),
                // the axis, the height (low, high), the segment and the line (8 bits each).
                let base = select(0.0, 1.5707963, axis == 0);
                let off = u32(clamp(round((ln.ang - base) / 0.4 * 63.0 + 31.5), 0.0, 63.0));
                let len = u32(clamp(round(ln.len), 1.0, 255.0));
                // (The axis bit: 1 for a line along v.)
                let lane_meta = off | (select(0u, 1u, axis == 0) << 6u);
                let tail = ((u32(ln.seg) & 0xffu) << 16u) | ((u32(ln.line) & 0xffu) << 24u);
                put_record(job, i, M_LANE, fc, len | (lane_meta << 8u) | tail);
                put_record(job, i, M_LANE, fc, len | ((lane_meta | 0x80u) << 8u) | tail);
            }
        }
    }
    let blk = ct_block(cp, seed, body.city.z);
    // The district's arcology or landmark, where the town is dense enough at its middle (every point
    // of the district asks there, so the lots under it are cleared only when it stands).
    var arco = ct_arcology(cp, blk.d.cell, k, seed, level);
    if arco.ok {
        arco.ok = town_at(city_dir(job, cp.tile, arco.centre)) >= 0.25;
        put_building(job, i, gi, cp.tile, arco);
    }
    // The block's megatower, by the town's density at its middle (the block's points agree).
    let mid = ct_block_mid(cp, blk);
    let mega = ct_mega(cp, blk, k, town_at(city_dir(job, cp.tile, mid)), seed);
    if mega.ok {
        if ct_on_arcology(arco, mega, k) {
            return;
        }
        put_building(job, i, gi, cp.tile, mega);
        // Its skybridges to the blocks on either side along u: a bridge's middle lies on the street
        // between them, so the point that places it may be in either block.
        var nb = blk;
        nb.bc.x += 1;
        let mega_n = ct_mega(cp, nb, k, town_at(city_dir(job, cp.tile, ct_block_mid(cp, nb))), seed);
        if mega_n.ok && !ct_on_arcology(arco, mega_n, k) {
            put_building(job, i, gi, cp.tile, ct_bridge(blk, mega, mega_n, k, seed));
        }
        var pb = blk;
        pb.bc.x -= 1;
        let mega_p = ct_mega(cp, pb, k, town_at(city_dir(job, cp.tile, ct_block_mid(cp, pb))), seed);
        if mega_p.ok && !ct_on_arcology(arco, mega_p, k) {
            put_building(job, i, gi, cp.tile, ct_bridge(pb, mega_p, mega, k, seed));
        }
        return;
    }
    var b = ct_building(cp, k, urban, seed, body.city.z);
    if ct_on_arcology(arco, b, k) {
        return;
    }
    b.model = M_FUTURE;
    b.height *= 1.4;
    put_building(job, i, gi, cp.tile, b);
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
    let seed = u32(body.city.y * 10000.0) + 100u;
    let level = u32(body.city.x + 0.5);
    if level >= 3u {
        place_future(job, i, gi, cp, k, urban, seed, level);
    } else if level == 2u {
        put_building(job, i, gi, cp.tile, ct_building(cp, k, urban, seed, body.city.z));
    } else {
        // The district's temple where the village is dense enough at its middle (every point of the
        // district asks there), the huts kept clear of its square.
        var tp = ct_temple(cp, ct_district(cp, seed).cell, seed);
        if tp.ok {
            tp.ok = town_at(city_dir(job, cp.tile, tp.centre)) >= 0.15;
            put_building(job, i, gi, cp.tile, tp);
        }
        let b = ct_hut(cp, k, urban, seed);
        if !(tp.ok && b.ok && length(b.centre - tp.centre) * k < 0.5 * length(tp.size) + 0.5 * length(b.size) + 8.0) {
            put_building(job, i, gi, cp.tile, b);
        }
    }
}

// One street object of the grid point whose square holds it (`lp` in its district's grid), in that
// district and clear of the arterials (`clear` from their middles). `dir` (the face's frame) the way it
// faces: `cull_street`.
fn put_street(job: Job, i: u32, gi: vec2<u32>, cp: CityPos, cell: vec2<i32>, g: Grid, lp: vec2<f32>, model: u32, dir: vec2<f32>, clear: f32, seed: u32) {
    let p = ct_unlocal(cp, cell, g, lp);
    let fc = cell_frac(job, cp.tile, p);
    if !in_square(gi, fc) {
        return;
    }
    let dc = ct_district(city_pos(cp.tile, p), seed);
    if any(dc.cell != cell) || dc.art < clear {
        return;
    }
    let ac = u32(clamp(round((atan2(dir.y, dir.x) / 6.2831853 + 0.5) * 65535.0), 0.0, 65535.0));
    put_record(job, i, BUILDING_MODELS + model, fc, ac);
}

// A cell of street objects (kind 3): at each point the lamps along the kerbs where the ground's lamps
// are (`ct_lamps`), the furniture between them, the traffic lights at the street corners and, where a
// lane passes through its square, a record for the cars (worked out every frame: `cull_cars`).
fn place_street(job: Job, i: u32) {
    if i >= BGRID * BGRID {
        return;
    }
    let gi = vec2<u32>(i % BGRID, i / BGRID);
    let f = (vec2<f32>(gi) + 0.5) / f32(BGRID);
    let cp = city_pos(vec2<i32>(job.tile_x, job.tile_y), vec2<f32>(job.local_x, job.local_y) + f * job.edge);
    let n0 = 1.0 / f32(1u << job.level);
    let uv = (vec2<f32>(f32(job.col), f32(job.row)) + f) * n0;
    let k = face_metric(uv.x, uv.y);
    let urban = town_at(normalize(uv_to_cube(job.face, uv.x, uv.y)));
    let streets = smoothstep(0.06, 0.2, urban);
    if streets < 0.5 {
        return;
    }
    let seed = u32(body.city.y * 10000.0) + 100u;
    let d = ct_district(cp, seed);
    let g = ct_grid(d.cell, seed, body.city.z);
    let lp = ct_local(cp, d.cell, g);
    // Half the street's carriageway and its sidewalk; half the arterial's road, and with its sidewalks.
    let half = STREET_ROAD_M / k;
    let walk = STREET_WALK_M / k;
    let a_half = mix(RURAL_ROAD_M, ART_ROAD_M, streets) / k;
    let clear = a_half + ART_WALK_M * streets / k;
    let reach = 0.75 * job.edge / f32(BGRID);
    let off_line = lp - floor(lp / g.size + 0.5) * g.size;
    var lanes = 0u;
    if min(abs(off_line.x), abs(off_line.y)) < half + reach {
        lanes |= 1u;
    }
    let ax = cp.q.x - ct_art_at(cp.tile, 0, ct_art_line(cp, 0), cp.q.y, seed);
    let ay = cp.q.y - ct_art_at(cp.tile, 1, ct_art_line(cp, 1), cp.q.x, seed);
    if min(abs(ax), abs(ay)) < a_half + reach + 2.0 {
        lanes |= 2u;
    }
    if lanes != 0u {
        put_record(job, i, BUILDING_MODELS + S_CAR, f, lanes | (u32(streets * 255.0) << 8u));
    }
    // The arterials' lamps, the tall kind, where the ground has them (`ct_lamps` on an arterial).
    for (var axis = 0; axis < 2; axis++) {
        let line = ct_art_line(cp, axis);
        let along = select(cp.q.x, cp.q.y, axis == 0);
        let across = select(cp.q.y, cp.q.x, axis == 0) - ct_art_at(cp.tile, axis, line, along, seed);
        if abs(across) > a_half + 0.6 / k + reach + 2.0 {
            continue;
        }
        for (var sd = -1; sd <= 1; sd += 2) {
            let side = f32(sd);
            let off = select(0.0, 16.0, sd > 0);
            let lx = floor((along - off) / 32.0 + 0.5) * 32.0 + off;
            let a0 = ct_art_at(cp.tile, axis, line, lx, seed) + side * (a_half + 0.6 / k);
            let p = select(vec2<f32>(lx, a0), vec2<f32>(a0, lx), axis == 0);
            let fc = cell_frac(job, cp.tile, p);
            if !in_square(gi, fc) {
                continue;
            }
            // Not where a crossing arterial takes the ground.
            let dc = ct_district(city_pos(cp.tile, p), seed);
            if dc.axis != axis || dc.line != line {
                continue;
            }
            let dir = select(vec2<f32>(0.0, -side), vec2<f32>(-side, 0.0), axis == 0);
            let ac = u32(clamp(round((atan2(dir.y, dir.x) / 6.2831853 + 0.5) * 65535.0), 0.0, 65535.0));
            put_record(job, i, BUILDING_MODELS + S_LAMP + 1u, fc, ac);
        }
    }
    let ux = g.rot;
    let uy = vec2<f32>(-g.rot.y, g.rot.x);
    let style = S_LAMP + min(u32(ct_street_hash(vec3<i32>(d.cell, 31), seed + 91u).x * 3.0), 2u);
    for (var a = 0; a < 2; a++) {
        // Streets of constant x (they run along y), then of constant y.
        let across = select(lp.x, lp.y, a == 1);
        let along = select(lp.y, lp.x, a == 1);
        let size = select(g.size.x, g.size.y, a == 1);
        let size_o = select(g.size.y, g.size.x, a == 1);
        let line = floor(across / size + 0.5);
        if abs(across - line * size) > half + walk + reach {
            continue;
        }
        let ua = select(uy, ux, a == 1);
        let uc = select(ux, uy, a == 1);
        for (var sd = -1; sd <= 1; sd += 2) {
            let side = f32(sd);
            let off = select(0.0, 16.0, sd > 0);
            let lx = floor((along - off) / 32.0 + 0.5) * 32.0 + off;
            if abs(lx - floor(lx / size_o + 0.5) * size_o) > half + 0.5 / k {
                let y = line * size + side * (half + 0.6 / k);
                put_street(job, i, gi, cp, d.cell, g, select(vec2<f32>(y, lx), vec2<f32>(lx, y), a == 1), style, -side * uc, clear, seed);
            }
            // Between the lamps, clear of the crossing street's sidewalks.
            let fx = floor((along - off - 16.0) / 32.0 + 0.5) * 32.0 + off + 16.0;
            if abs(fx - floor(fx / size_o + 0.5) * size_o) > half + walk + 1.0 / k {
                let h = ct_street_hash(vec3<i32>(i32(line) * 2 + a, i32(floor(fx / 16.0)), d.cell.x * 256 + d.cell.y + sd), seed + 92u);
                var model = 255u;
                var yo = half + 0.5 / k;
                var dir = ua;
                if h.x < 0.08 {
                    // A bench on the sidewalk's far side, facing the road (its back, the model's +x,
                    // away from it).
                    model = S_BENCH + select(0u, 1u, h.y < 0.5);
                    yo = half + 1.6 / k;
                    dir = side * uc;
                } else if h.x < 0.2 {
                    model = S_BIN + select(0u, 1u, h.y < 0.5);
                } else if h.x < 0.24 {
                    model = S_HYDRANT;
                }
                if model != 255u {
                    let y = line * size + side * yo;
                    put_street(job, i, gi, cp, d.cell, g, select(vec2<f32>(y, fx), vec2<f32>(fx, y), a == 1), model, dir, clear, seed);
                }
            }
        }
    }
    // The traffic lights downtown: at two opposite corners of a street junction, over the road.
    if urban > 0.35 {
        let nl = floor(lp / g.size + 0.5);
        let h = ct_street_hash(vec3<i32>(vec2<i32>(nl), d.cell.x * 256 + d.cell.y), seed + 93u);
        if h.x < 0.6 {
            for (var sd = -1; sd <= 1; sd += 2) {
                let sx = f32(sd);
                let sy = select(-sx, sx, h.y < 0.5);
                let c = nl * g.size + vec2<f32>(sx, sy) * (half + 0.6 / k);
                put_street(job, i, gi, cp, d.cell, g, c, S_SIGNAL + select(0u, 1u, h.z < 0.5), -sx * ux, clear, seed);
            }
        }
    }
}

@compute @workgroup_size(64)
fn place(@builtin(global_invocation_id) gid: vec3<u32>) {
    let job = jobs[gid.y];
    let i = gid.x;
    if job.kind == 3u {
        place_street(job, i);
        return;
    }
    if job.kind >= 1u {
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
fn plant_weights(dir: vec3<f32>, h: f32, slope: f32, nz: vec2<u32>) -> vec4<f32> {
    let a = unpack2x16float(nz.x);
    let b = unpack2x16float(nz.y);
    let fp = body.veg.y;
    let hv = body.climate[5].z + h / max(body.climate[4].z, 1.0);
    let s = se_terra_snow_at(dir, hv, slope, a.x, b.y, body.climate[0], body.climate[1], body.climate[2], body.climate[6].w > 0.0, body.sun_body.xyz);
    let c = se_terra_climate_at(slope, s, 0.0, b.x, body.climate[0], body.climate[1]);
    let ptf = plants_ptf_at(dir, fp);
    // (The weights leave out n.y, the colour's vary: not stored.)
    let m = se_terra_weights_at(dir, hv, slope, fp, c, 0.0, vec3<f32>(a.x, 0.0, a.y), ptf, body.climate[0], body.climate[2], body.climate[3], body.climate[4], true, false);
    // (w the snow and ice: the trees near them take snow.)
    return vec4<f32>(m.w[9], m.w[10], m.w[11], m.w[6] + m.w[7]);
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
    let pm = (packed >> 16u) & 0xffu;
    if (packed >> 24u) == 3u {
        cull_street(slot, i, cell, pm, packed & 0xffffu);
        return;
    }
    if pm >= body.species.w && pm < body.species.w + BUILDING_MODELS && body.city.x > 0.5 {
        cull_building(slot, i, cell, packed >> 24u, pm);
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
    var snow = 0.0;
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
                snow = smoothstep(0.05, 0.4, w.w);
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
    let o = Inst(pos, pack4x8unorm(vec4<f32>(fade, mesh, snow, 0.0)), oct_encode(xa), oct_encode(up), scale, model | tint);
    if mesh > 0.0 {
        append(md.mesh_bucket, o);
    }
    if mesh < 1.0 {
        append(md.impostor_bucket, o);
    }
}

// The land as the ground's town needs it (planet.wesl's civ block: not the highlands, off the
// sea; on a map body its water mask, as the ground's: Earth's coastal towns stand a few metres up).
fn town_land(p: vec4<f32>, info: PageInfo) -> bool {
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
    return smoothstep(0.03, 0.3, land) >= 0.5;
}

// A street object (kind 3 cells): its record lifted onto the drawn ground, within its range (dithered
// out over the last 15 %, `street_near` handing over to the ground's flat ones there). The record's turn
// is a lamp's or a traffic light's arm (the model's z), the others' x axis.
fn cull_street(slot: u32, i: u32, cell: Cell, model: u32, gidx: u32) {
    let rec = noises[slot * SLOT_CAP + i];
    let mk = model - body.species.w - BUILDING_MODELS;
    if mk == S_CAR {
        cull_cars(cell, rec.y, gidx);
        return;
    }
    let lf = lift(cell, (vec2<f32>(f32(rec.x & 0xffffu), f32(rec.x >> 16u)) + 0.5) / 65536.0);
    if !lf.ok {
        return;
    }
    let info = page_info[lf.page];
    if !town_land(lf.p, info) {
        return;
    }
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let pos = m * lf.p.xyz + info.t;
    let fade = clamp((body.street.x - length(pos)) / (body.street.x * 0.15), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    let md = models[model];
    let up = cell.up;
    let bc = pos + up * md.centre_y;
    for (var c = 0u; c < 5u; c++) {
        let pl = cam.planes[c];
        if dot(pl.xyz, bc) + pl.w < -md.radius {
            return;
        }
    }
    let ang = (f32(rec.y & 0xffffu) / 65535.0 - 0.5) * 6.2831853;
    var dir = cos(ang) * normalize(m * lf.du) + sin(ang) * normalize(m * lf.dv);
    dir = normalize(dir - up * dot(dir, up));
    let arm = mk < S_BENCH || mk >= S_SIGNAL;
    let xa = select(dir, cross(up, dir), arm);
    append(md.mesh_bucket, Inst(pos, pack4x8unorm(vec4<f32>(1.0, fade, 0.0, 0.0)), oct_encode(xa), oct_encode(up), 1.0, model));
}

// A car with its middle at `p` (the tile frame of `tile`), driving along `ahead` (a unit vector in the
// face's frame): on the drawn ground, its two ends on the road where they lie in the cell, its paint in
// the instance's high bits (scatter_draw.wesl `vertex_mesh`).
fn put_car(cell: Cell, tile: vec2<i32>, p: vec2<f32>, ahead: vec2<f32>, car: SlotCar) {
    let fc = (vec2<f32>(tile - cell.tile) * 4096.0 + p - cell.local) / cell.edge;
    let lf = lift(cell, fc);
    if !lf.ok {
        return;
    }
    let info = page_info[lf.page];
    if !town_land(lf.p, info) {
        return;
    }
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    var pos = m * lf.p.xyz + info.t;
    let fade = clamp((body.street.x - length(pos)) / (body.street.x * 0.15), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    let model = body.species.w + BUILDING_MODELS + S_CAR + min(u32(car.pick * f32(S_CARS)), S_CARS - 1u);
    let md = models[model];
    let bc = pos + cell.up * md.centre_y;
    for (var c = 0u; c < 5u; c++) {
        let pl = cam.planes[c];
        if dot(pl.xyz, bc) + pl.w < -md.radius {
            return;
        }
    }
    var fwd = normalize(ahead.x * normalize(m * lf.du) + ahead.y * normalize(m * lf.dv));
    let reach = ahead * 2.0 / (cell.k * cell.edge);
    let fa = fc + reach;
    let fb = fc - reach;
    var tilted = false;
    if all(fa > vec2<f32>(0.0)) && all(fa < vec2<f32>(1.0)) && all(fb > vec2<f32>(0.0)) && all(fb < vec2<f32>(1.0)) {
        let la = lift(cell, fa);
        let lb = lift(cell, fb);
        if la.ok && lb.ok {
            let pa = m * la.p.xyz + page_info[la.page].t;
            let pb = m * lb.p.xyz + page_info[lb.page].t;
            pos = 0.5 * (pa + pb);
            fwd = normalize(pa - pb);
            tilted = true;
        }
    }
    if !tilted {
        var tri_n = normalize(m * cross(lf.vb.xyz - lf.va.xyz, lf.vc.xyz - lf.va.xyz));
        fwd = normalize(fwd - tri_n * dot(fwd, tri_n));
    }
    let up = normalize(cell.up - fwd * dot(cell.up, fwd));
    let paint = sqrt(ct_paint(car.paint));
    let packed = model | pack4x8unorm(vec4<f32>(0.0, paint));
    append(md.mesh_bucket, Inst(pos, pack4x8unorm(vec4<f32>(1.0, fade, 0.0, 0.0)), oct_encode(fwd), oct_encode(up), 1.0, packed));
}

// The cars whose middles are in a grid point's square this frame: the ground's traffic (`ct_traffic`:
// no state, a hash of the slot and the clock), each car in one square alone. `rec`: bit 0 a street's lane
// may pass here, bit 1 an arterial's, bits 8-15 the town's share (`streets`).
fn cull_cars(cell: Cell, rec: u32, gidx: u32) {
    let gi = vec2<u32>(gidx % BGRID, gidx / BGRID);
    let cp = city_pos(cell.tile, cell.local + (vec2<f32>(gi) + 0.5) / f32(BGRID) * cell.edge);
    let seed = u32(body.city.y * 10000.0) + 100u;
    let t = body.street.y;
    let k = cell.k;
    let reach = 0.75 * cell.edge / f32(BGRID);
    let streets = f32((rec >> 8u) & 0xffu) / 255.0;
    if (rec & 1u) != 0u {
        let half = STREET_ROAD_M / k;
        let clear = (mix(RURAL_ROAD_M, ART_ROAD_M, streets) + ART_WALK_M * streets) / k;
        let d = ct_district(cp, seed);
        let g = ct_grid(d.cell, seed, body.city.z);
        let lp = ct_local(cp, d.cell, g);
        let ux = g.rot;
        let uy = vec2<f32>(-g.rot.y, g.rot.x);
        for (var a = 0; a < 2; a++) {
            let across = select(lp.x, lp.y, a == 1);
            let along = select(lp.y, lp.x, a == 1);
            let size = select(g.size.x, g.size.y, a == 1);
            let line = floor(across / size + 0.5);
            if abs(across - line * size) > half + reach {
                continue;
            }
            let ua = select(uy, ux, a == 1);
            for (var lane = -1; lane <= 1; lane += 2) {
                let v = f32(lane) * 9.0;
                let yl = line * size - f32(lane) * 0.5 * half;
                let key = ct_street_key(d.cell, i32(line), a == 1, lane);
                let s1 = floor((along + reach - v * t) / 24.0);
                for (var s = floor((along - reach - v * t) / 24.0); s <= s1; s += 1.0) {
                    let car = ct_slot_car(s, v, 24.0, 0.25 * streets + 0.1, t, 0, key, seed, k);
                    if !car.ok {
                        continue;
                    }
                    let p = ct_unlocal(cp, d.cell, g, select(vec2<f32>(yl, car.x), vec2<f32>(car.x, yl), a == 1));
                    let fc = (vec2<f32>(cp.tile - cell.tile) * 4096.0 + p - cell.local) / cell.edge;
                    if !in_square(gi, fc) {
                        continue;
                    }
                    let dc = ct_district(city_pos(cp.tile, p), seed);
                    if any(dc.cell != d.cell) || dc.art < clear {
                        continue;
                    }
                    put_car(cell, cp.tile, p, f32(lane) * ua, car);
                }
            }
        }
    }
    if (rec & 2u) != 0u {
        for (var axis = 0; axis < 2; axis++) {
            let line = ct_art_line(cp, axis);
            let along = select(cp.q.x, cp.q.y, axis == 0);
            let across = select(cp.q.y, cp.q.x, axis == 0) - ct_art_at(cp.tile, axis, line, along, seed);
            if abs(across) > mix(RURAL_ROAD_M, ART_ROAD_M, streets) / k + reach + 2.0 {
                continue;
            }
            let tile_a = select(cp.tile.x, cp.tile.y, axis == 0);
            for (var lane = -1; lane <= 1; lane += 2) {
                for (var o = 0; o < 2; o++) {
                    let outer = o == 1;
                    // (The road narrows towards the town's edge: its outer lanes end before it.)
                    if outer && streets < 0.7 {
                        continue;
                    }
                    let off = -f32(lane) * select(1.9, 6.0, outer) / k;
                    let v = f32(lane) * select(13.0, 16.0, outer);
                    let key = ct_art_key(line, axis, lane, outer);
                    let s1 = floor((along + reach - v * t) / 32.0);
                    for (var s = floor((along - reach - v * t) / 32.0); s <= s1; s += 1.0) {
                        let car = ct_slot_car(s, v, 32.0, 0.35 + 0.35 * streets, t, tile_a * 128, key, seed, k);
                        if !car.ok {
                            continue;
                        }
                        let a0 = ct_art_at(cp.tile, axis, line, car.x, seed) + off;
                        let p = select(vec2<f32>(car.x, a0), vec2<f32>(a0, car.x), axis == 0);
                        let fc = (vec2<f32>(cp.tile - cell.tile) * 4096.0 + p - cell.local) / cell.edge;
                        if !in_square(gi, fc) {
                            continue;
                        }
                        let a1 = ct_art_at(cp.tile, axis, line, car.x + f32(lane) * 2.0, seed) + off;
                        let dv = vec2<f32>(f32(lane) * 2.0, a1 - a0);
                        put_car(cell, cp.tile, p, normalize(select(dv, dv.yx, axis == 0)), car);
                    }
                }
            }
        }
    }
}

// A building: its record lifted onto the drawn ground, standing on a plinth down to the lowest ground
// under its footprint, turned with its street grid, faded out over the last 15 % of the range.
fn cull_building(slot: u32, i: u32, cell: Cell, kind: u32, model: u32) {
    let rec = noises[slot * SLOT_CAP + i];
    let lf = lift(cell, (vec2<f32>(f32(rec.x & 0xffffu), f32(rec.x >> 16u)) + 0.5) / 65536.0);
    if !lf.ok {
        return;
    }
    let p = lf.p;
    let info = page_info[lf.page];
    if !town_land(p, info) {
        return;
    }
    let mk = model - body.species.w;
    if mk == M_LANE {
        cull_lane(rec, lf, info, cell, kind, model);
        return;
    }
    let big = mk == M_ARCO || mk == M_LANDMARK || mk == M_DOME;
    let wstep = select(select(0.5, 1.0, mk == M_BRIDGE), 4.0, big);
    let w = f32(rec.y & 0xffu) * wstep;
    let dpt = f32((rec.y >> 8u) & 0xffu) * select(0.5, 4.0, big);
    let hq = f32((rec.y >> 16u) & 0xffu) / 255.0;
    let h = hq * hq * 1600.0;
    let ang = (f32(rec.y >> 24u) / 255.0 - 0.5) * 6.2831853;
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let up = cell.up;
    // The plinth: down by the footprint's half width times the ground's slope, and a metre.
    let rd = body.centre.w;
    let dir = normalize(info.dir * rd + p.xyz);
    let tri_n = normalize(cross(lf.vb.xyz - lf.va.xyz, lf.vc.xyz - lf.va.xyz));
    let cs = clamp(abs(dot(tri_n, dir)), 0.2, 1.0);
    var plinth = 1.0 + 0.5 * max(w, dpt) * sqrt(1.0 - cs * cs) / cs;
    var scale = h + plinth;
    if mk == 1u || mk == 2u {
        // A village's hut or longhouse keeps its own height (a plinth would stretch its walls into
        // the ground): it only sinks a little into the slope.
        plinth = min(plinth, 0.5) - body.lod2.y;
        scale = h;
    } else if mk == M_BRIDGE {
        // A skybridge: 5 m deep at its height over the street.
        plinth = -h - body.lod2.y;
        scale = 5.0;
    } else if mk >= M_TEMPLE && mk != M_DOME {
        // A hero model keeps its shape: it sinks into the slope (its lowest 0.8 m below the ground
        // on the level).
        plinth -= 0.2;
        scale = h;
    }
    let pos0 = m * p.xyz + info.t - up * (body.lod2.y + plinth);
    var pos = pos0;
    // The mod's broken buildings (step 10, F12.3-5; one uniform branch with none): sunk into the ground over its collapse with
    // its lights out (a negative width), its rubble left as a low broken box. Placement is untouched (the "same decision" rule).
    var wsign = 1.0;
    if broken.head.count > 0u {
        let fc0 = (vec2<f32>(f32(rec.x & 0xffffu), f32(rec.x >> 16u)) + 0.5) / 65536.0;
        let bid = cell.tile * 16384 + vec2<i32>(round((cell.local + fc0 * cell.edge) * 4.0));
        let age = broken_age(bid, u32(cell.face) | (mk << 8u));
        if age >= 0.0 {
            let pr = clamp(age / clamp(2.5 + h / 200.0, 2.5, 7.0), 0.0, 1.0);
            if kind == 1u && mk != M_BRIDGE && mk != M_DOME {
                cull_rubble(rec, lf, pos0, up, w, dpt, h, plinth, ang, pr);
            }
            if pr >= 1.0 {
                return;
            }
            pos = pos0 - up * (scale * pr * pr);
            wsign = -1.0;
        }
    }
    let dist = length(pos);
    // The mod's probe (0 with the mod off: one uniform branch): measuring, the near and far lists in full,
    // whatever the camera's distance.
    let probing = body.street.z;
    if probing > 1.5 {
        probe_building(cell, rec, lf, pos, w, dpt, scale, ang, model, kind, 0.0);
    }
    // The near class to its range; the far class (the tall ones) from 85 % of it to its own (the two
    // draw the same box between: no seam, no flicker).
    var range = body.city2.x;
    if kind == 2u {
        if dist < 0.85 * body.city2.x {
            return;
        }
        range = body.flora_k.y;
    }
    let fade = clamp((range - dist) / (range * 0.15), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    // As drawn: after the range fade, before the view's planes (buildings behind the camera count).
    if probing > 0.5 && probing < 1.5 {
        probe_building(cell, rec, lf, pos, w, dpt, scale, ang, model, kind, fade);
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
    let o = Inst(pos, pack2x16float(vec2<f32>(w * wsign, dpt)), oct_encode(xa), oct_encode(up), scale, model | (u32(fade * 255.0) << 8u) | (hash << 16u));
    append(models[model].mesh_bucket, o);
}

// The mod's broken table (step 10): seconds since the building with this id (face metres x 4), face and model broke, or -1.
// A plain walk over the entries: there are few (`breakable.rs` keeps at most `BROKEN_MAX`).
fn broken_age(id: vec2<i32>, face_model: u32) -> f32 {
    let n = min(broken.head.count, 1024u);
    for (var i = 0u; i < n; i++) {
        let e = broken.entries[i];
        if e.face_model == face_model && abs(e.x - id.x) <= 16 && abs(e.y - id.y) <= 16 {
            return broken.head.clock - e.t0;
        }
    }
    return -1.0;
}

// A broken building's rubble (the near cells alone, so it's drawn once): the box, wider than the building and low, growing as
// it falls (to `min(0.18 h + 1, 30)` m over the ground), drawn dark and broken (a negative width), within the near range.
fn cull_rubble(rec: vec2<u32>, lf: Lift, foot: vec3<f32>, up: vec3<f32>, w: f32, dpt: f32, h: f32, plinth: f32, ang: f32, pr: f32) {
    let dist = length(foot);
    let range = body.city2.x;
    let fade = clamp((range - dist) / (range * 0.15), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    let tall = min(0.18 * h + 1.0, 30.0) * smoothstep(0.0, 0.7, pr) + max(plinth, 0.0);
    if tall < 0.05 {
        return;
    }
    let r = 0.5 * sqrt(1.6 * (w * w + dpt * dpt) + tall * tall);
    let bc = foot + up * (0.5 * tall);
    for (var k = 0u; k < 5u; k++) {
        let pl = cam.planes[k];
        if dot(pl.xyz, bc) + pl.w < -r {
            return;
        }
    }
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let fu = normalize(m * lf.du);
    let fv = normalize(m * lf.dv);
    var xa = cos(ang + 0.3) * fu + sin(ang + 0.3) * fv;
    xa = normalize(xa - up * dot(xa, up));
    let box = body.species.w;
    let hash = mix32(rec.y ^ (rec.x * 0x85ebca6bu)) & 0xffffu;
    let o = Inst(foot, pack2x16float(vec2<f32>(-1.25 * max(w, 4.0), 1.25 * max(dpt, 4.0))), oct_encode(xa), oct_encode(up), tall, box | (u32(fade * 255.0) << 8u) | (hash << 16u));
    append(models[box].mesh_bucket, o);
}

// The probe (the mod's F12.1): a building whose bounding sphere reaches within `probe.head.a.w` of the capsule's axis is
// appended with its id: its face and its middle's face metres x 4 from the face's corner (the layout's own frame, the
// same from a near and a far cell; the record's hash isn't), the Inst as the draw would take it, its cell's kind and metric.
fn probe_building(cell: Cell, rec: vec2<u32>, lf: Lift, pos: vec3<f32>, w: f32, dpt: f32, scale: f32, ang: f32, model: u32, kind: u32, fade: f32) {
    let up = cell.up;
    let r = 0.5 * sqrt(w * w + dpt * dpt + scale * scale);
    let bc = pos + up * (0.5 * scale);
    let a = probe.head.a.xyz;
    let ab = probe.head.b.xyz - a;
    let t = clamp(dot(bc - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    let d = bc - (a + ab * t);
    let reach = probe.head.a.w + r;
    if dot(d, d) > reach * reach {
        return;
    }
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let fu = normalize(m * lf.du);
    let fv = normalize(m * lf.dv);
    var xa = cos(ang) * fu + sin(ang) * fv;
    xa = normalize(xa - up * dot(xa, up));
    let hash = mix32(rec.x ^ (rec.y * 0x9e3779b9u)) & 0xffffu;
    let inst = Inst(pos, pack2x16float(vec2<f32>(w, dpt)), oct_encode(xa), oct_encode(up), scale, model | (u32(fade * 255.0) << 8u) | (hash << 16u));
    // Its middle in face metres: the cell's tile (whole, so no rounding) and the metres from it.
    let fc = (vec2<f32>(f32(rec.x & 0xffffu), f32(rec.x >> 16u)) + 0.5) / 65536.0;
    let id = cell.tile * 16384 + vec2<i32>(round((cell.local + fc * cell.edge) * 4.0));
    let mk = model - body.species.w;
    let k = atomicAdd(&probe.head.count, 1u);
    if k < probe.head.cap {
        probe.recs[k] = ProbeRec(inst, id, u32(cell.face) | (mk << 8u) | (kind << 16u), cell.k);
    }
}

// A flying lane's segment: its middle lifted onto the drawn ground and raised to its height (by the
// line: low lanes 70-110 m, high ones 180-280 m), along its line (tilted with the ground), its two
// ribbons 12 m apart. The draw's hash: the segment, the line, the axis and the height.
fn cull_lane(rec: vec2<u32>, lf: Lift, info: PageInfo, cell: Cell, kind: u32, model: u32) {
    let len = f32(rec.y & 0xffu);
    let lane_meta = (rec.y >> 8u) & 0xffu;
    let axis = (lane_meta >> 6u) & 1u;
    let high = lane_meta >> 7u;
    let seg = (rec.y >> 16u) & 0xffu;
    let line = rec.y >> 24u;
    let hl = f32(mix32(line * 0x9e3779b9u + axis * 0x85ebca6bu) & 0xffffu) / 65535.0;
    let hgt = select(mix(70.0, 110.0, hl), mix(180.0, 280.0, hl), high == 1u);
    let m = mat3x3<f32>(body.m0.xyz, body.m1.xyz, body.m2.xyz);
    let up = cell.up;
    let pos = m * lf.p.xyz + info.t + up * (hgt - body.lod2.y);
    let dist = length(pos);
    var range = body.city2.x;
    if kind == 2u {
        if dist < 0.85 * body.city2.x {
            return;
        }
        range = body.flora_k.y;
    }
    let fade = clamp((range - dist) / (range * 0.15), 0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    let r = 0.5 * len + 10.0;
    for (var k = 0u; k < 5u; k++) {
        let pl = cam.planes[k];
        if dot(pl.xyz, pos) + pl.w < -r {
            return;
        }
    }
    let ang = f32(axis) * 1.5707963 + (f32(lane_meta & 63u) - 31.5) / 63.0 * 0.4;
    let fu = normalize(m * lf.du);
    let fv = normalize(m * lf.dv);
    var xa = cos(ang) * fu + sin(ang) * fv;
    let tri_n = normalize(m * cross(lf.vb.xyz - lf.va.xyz, lf.vc.xyz - lf.va.xyz));
    xa = normalize(xa - tri_n * dot(xa, tri_n));
    let hash = seg | ((line & 0x1fu) << 8u) | (axis << 13u) | (high << 14u);
    let o = Inst(pos, pack2x16float(vec2<f32>(len, 12.0)), oct_encode(xa), oct_encode(up), 2.0, model | (u32(fade * 255.0) << 8u) | (hash << 16u));
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
