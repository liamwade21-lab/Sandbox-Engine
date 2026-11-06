//! The towns' roads on the CPU: city.wesl's street layout (districts with bent arterials, each with
//! its turned grid of streets) and settlement.wesl's field, the same f32 arithmetic and hashes, so the
//! ground's heights can be levelled across the roads the ground's shader draws (se_engine road_level.rs;
//! the trees and buildings stand on those heights). Keep every function here in step with its WGSL
//! twin: a road levelled where the shader draws none is a terrace in a field, and the other way round a
//! road that tilts across a hillside.

use super::civilization::st_hash;

/// city.wesl's constants (face metres, true metres for the widths).
pub const CITY_TILE: f64 = 4096.0;
const DISTRICT_M: f32 = 512.0;
const BEND_M: f32 = 55.0;
const ART_ROAD_M: f32 = 8.5;
const ART_WALK_M: f32 = 3.5;
const RURAL_ROAD_M: f32 = 3.4;
const STREET_ROAD_M: f32 = 4.5;
const STREET_WALK_M: f32 = 2.5;
const DOME_R: f32 = 180.0;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// A point in the city frame: its 4,096 m tile and its face metres inside it (0..4,096).
#[derive(Clone, Copy, Debug)]
pub struct CityPos {
    pub tile: [i32; 2],
    pub q: [f32; 2],
}

impl CityPos {
    /// From face metres (the face's u, v times the radius times pi/2; f64, exact on any body).
    pub fn from_face_m(x: f64, y: f64) -> Self {
        let (tx, ty) = ((x / CITY_TILE).floor(), (y / CITY_TILE).floor());
        CityPos { tile: [tx as i32, ty as i32], q: [(x - tx * CITY_TILE) as f32, (y - ty * CITY_TILE) as f32] }
    }

    /// Face metres of a point given in this one's tile frame.
    pub fn face_m(&self, q: [f32; 2]) -> (f64, f64) {
        (self.tile[0] as f64 * CITY_TILE + q[0] as f64, self.tile[1] as f64 * CITY_TILE + q[1] as f64)
    }
}

fn bend(line: i32, cell: i32, r: f32, axis: i32, seed: u32) -> f32 {
    let k = cell >> 1;
    let f = ((cell & 1) as f32 * DISTRICT_M + r) / (2.0 * DISTRICT_M);
    let h0 = st_hash([line, k, axis], seed.wrapping_add(11))[0];
    let h1 = st_hash([line, k + 1, axis], seed.wrapping_add(11))[0];
    BEND_M * (2.0 * mix(h0, h1, smoothstep(0.0, 1.0, f)) - 1.0)
}

/// The two arterial lines nearest a point, one of each direction: `line` its number, `across` the
/// point's signed offset from it (face m). `x` a line of constant x, `y` of constant y. And its cell.
#[derive(Clone, Copy, Debug)]
struct Lines {
    cell: [i32; 2],
    x: (i32, f32),
    y: (i32, f32),
}

fn lines(cp: CityPos, seed: u32) -> Lines {
    let fl = [(cp.q[0] / DISTRICT_M).floor(), (cp.q[1] / DISTRICT_M).floor()];
    let c = [cp.tile[0] * 8 + fl[0] as i32, cp.tile[1] * 8 + fl[1] as i32];
    let r = [cp.q[0] - fl[0] * DISTRICT_M, cp.q[1] - fl[1] * DISTRICT_M];
    // Lines of constant x bend along y, and the other way round.
    let lx = r[0] - bend(c[0], c[1], r[1], 0, seed);
    let rx = DISTRICT_M + bend(c[0] + 1, c[1], r[1], 0, seed) - r[0];
    let ly = r[1] - bend(c[1], c[0], r[0], 1, seed);
    let ry = DISTRICT_M + bend(c[1] + 1, c[0], r[0], 1, seed) - r[1];
    let side = |l: f32, r: f32| if l < 0.0 { -1 } else if r < 0.0 { 1 } else { 0 };
    let near = |l: f32, r: f32, line: i32| if l.abs() < r.abs() { (line, l) } else { (line + 1, -r) };
    Lines { cell: [c[0] + side(lx, rx), c[1] + side(ly, ry)], x: near(lx, rx, c[0]), y: near(ly, ry, c[1]) }
}

/// city.wesl `District`.
#[derive(Clone, Copy, Debug)]
pub struct District {
    pub cell: [i32; 2],
    pub art: f32,
    pub across: f32,
    pub along: f32,
    pub axis: i32,
    pub line: i32,
}

pub fn district(cp: CityPos, seed: u32) -> District {
    let l = lines(cp, seed);
    if l.x.1.abs() < l.y.1.abs() {
        District { cell: l.cell, art: l.x.1.abs(), across: l.x.1, along: cp.q[1], axis: 0, line: l.x.0 }
    } else {
        District { cell: l.cell, art: l.y.1.abs(), across: l.y.1, along: cp.q[0], axis: 1, line: l.y.0 }
    }
}

/// city.wesl `Grid`: a district's street grid.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub rot: [f32; 2],
    pub size: [f32; 2],
    pub off: [f32; 2],
}

pub fn grid(cell: [i32; 2], seed: u32, spread: f32) -> Grid {
    let h = st_hash([cell[0], cell[1], 77], seed);
    let h2 = st_hash([cell[0], cell[1], 78], seed);
    let a = (h[0] - 0.5) * 3.14159 * 0.5 * mix(0.15, 1.0, spread) + if h2[0] < 0.15 * spread { 0.7854 } else { 0.0 };
    let size = [mix(80.0, 150.0, h[1]), mix(60.0, 110.0, h[2])];
    Grid { rot: [a.cos(), a.sin()], size, off: [h[3] * size[0], h2[1] * size[1]] }
}

fn district_centre(cp: CityPos, cell: [i32; 2]) -> [f32; 2] {
    [
        (cell[0] - cp.tile[0] * 8) as f32 * DISTRICT_M + 0.5 * DISTRICT_M,
        (cell[1] - cp.tile[1] * 8) as f32 * DISTRICT_M + 0.5 * DISTRICT_M,
    ]
}

/// city.wesl `ct_local`: the point in its district's turned grid.
pub fn local(cp: CityPos, cell: [i32; 2], g: &Grid) -> [f32; 2] {
    let c = district_centre(cp, cell);
    let p = [cp.q[0] - c[0], cp.q[1] - c[1]];
    [g.rot[0] * p[0] + g.rot[1] * p[1] + g.off[0], -g.rot[1] * p[0] + g.rot[0] * p[1] + g.off[1]]
}

/// city.wesl `ct_unlocal`: a point of the grid back in the tile frame of `cp`.
pub fn unlocal(cp: CityPos, cell: [i32; 2], g: &Grid, lp: [f32; 2]) -> [f32; 2] {
    let c = [lp[0] - g.off[0], lp[1] - g.off[1]];
    let o = district_centre(cp, cell);
    [g.rot[0] * c[0] - g.rot[1] * c[1] + o[0], g.rot[1] * c[0] + g.rot[0] * c[1] + o[1]]
}

/// city.wesl `ct_dome_mask` near the ground (a footprint far under a district): 1 under a level 4
/// colony's dome, 0 between them (the domes' districts' streets alone are drawn there).
pub fn dome_mask(cp: CityPos, seed: u32) -> f32 {
    let c = [(cp.q[0] / DISTRICT_M).floor(), (cp.q[1] / DISTRICT_M).floor()];
    let cell = [cp.tile[0] * 8 + c[0] as i32, cp.tile[1] * 8 + c[1] as i32];
    let h = st_hash([cell[0], cell[1], 4141], seed.wrapping_add(53));
    let s = if h[0] < 0.75 { mix(0.75, 1.0, h[1]) } else { 0.0 };
    if s <= 0.0 {
        return 0.0;
    }
    let d = [cp.q[0] - (c[0] + 0.5) * DISTRICT_M, cp.q[1] - (c[1] + 0.5) * DISTRICT_M];
    1.0 - smoothstep(0.97, 1.0, (d[0] * d[0] + d[1] * d[1]).sqrt() / (DOME_R * s))
}

/// What kind of road a spot is: how its density comes from the town and countryside densities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadKind {
    /// A street, in town.
    Street,
    /// An arterial: in town, or through the countryside on the even lines.
    Arterial { even: bool },
}

impl RoadKind {
    /// How much of this road there is at the densities `town` and `rural` (0..1).
    pub fn density(self, town: f32, rural: f32) -> f32 {
        let streets = smoothstep(0.06, 0.2, town);
        match self {
            RoadKind::Street => streets,
            RoadKind::Arterial { even } => (rural * if even { 1.0 } else { 0.0 }).max(streets),
        }
    }
}

/// A road near a point.
#[derive(Clone, Copy, Debug)]
pub struct RoadSpot {
    /// How far the point follows the road's height, 0..1: 1 on the road and its sidewalks, fading out
    /// over the bank beside them.
    pub w: f32,
    /// Its share where several roads meet: falls smoothly from its middle line to 0 at the bank's end,
    /// so on a road climbing out of a crossing the height runs on without a step.
    pub share: f32,
    /// The point on the road's middle line across from this one (face metres in the point's tile
    /// frame), whose height the road has there.
    pub centre: [f32; 2],
    pub kind: RoadKind,
    /// `kind.density` at the densities it was found with (already in `w` and `share`).
    pub density: f32,
}

/// The roads a point lies on or beside, as city.wesl `city_ground` draws them: its district's two
/// nearest streets (lines of constant u and v of the turned grid) where the town is dense enough for
/// streets, and the two nearest arterials, one of each direction (in town, or through the countryside
/// on every other line). `k` true metres per face metre, `town` and `rural` the ground's densities
/// there (0..1), `seed` the layout's (the culture seed's 4 digits + 100), `spread` the culture's grid
/// turn, `bank_m` the width (true m) over which the levelling fades out beside a road.
///
/// Streets fade out inside the arterials' width: the shader paints the arterial over them, and the
/// street grid changes district on the arterial's middle line, which would be a step in the heights.
pub fn roads_near(cp: CityPos, k: f32, town: f32, rural: f32, seed: u32, spread: f32, bank_m: f32) -> [Option<RoadSpot>; 4] {
    let mut out = [None; 4];
    let k = k.max(1e-3);
    let ramp = |across_face: f32, half_m: f32| 1.0 - smoothstep(half_m, half_m + bank_m, across_face * k);
    let bell = |across_face: f32, half_m: f32| {
        let t = (1.0 - across_face * k / (half_m + bank_m)).max(0.0);
        t * t
    };
    let l = lines(cp, seed);
    let streets = smoothstep(0.06, 0.2, town);
    let half_a = mix(RURAL_ROAD_M, ART_ROAD_M, streets) + ART_WALK_M * streets;
    if streets > 0.0 {
        let art = l.x.1.abs().min(l.y.1.abs());
        let off_arterial = 1.0 - ramp(art, half_a);
        if off_arterial > 0.0 {
            let g = grid(l.cell, seed, spread);
            let lp = local(cp, l.cell, &g);
            let b = [lp[0] - (lp[0] / g.size[0]).floor() * g.size[0], lp[1] - (lp[1] / g.size[1]).floor() * g.size[1]];
            let half = STREET_ROAD_M + STREET_WALK_M;
            for axis in 0..2 {
                let s = b[axis].min(g.size[axis] - b[axis]);
                let w = streets * off_arterial * ramp(s, half);
                if w > 0.0 {
                    // The street's middle across from the point (its line of constant u, or of v).
                    let shift = if b[axis] < g.size[axis] - b[axis] { b[axis] } else { b[axis] - g.size[axis] };
                    let mut c = lp;
                    c[axis] -= shift;
                    out[axis] = Some(RoadSpot {
                        w,
                        share: streets * off_arterial * bell(s, half),
                        centre: unlocal(cp, l.cell, &g, c),
                        kind: RoadKind::Street,
                        density: streets,
                    });
                }
            }
        }
    }
    for (slot, axis, (line, across)) in [(2, 0, l.x), (3, 1, l.y)] {
        let kind = RoadKind::Arterial { even: line & 1 == 0 };
        let density = kind.density(town, rural);
        let w = density * ramp(across.abs(), half_a);
        if w > 0.0 {
            let mut c = cp.q;
            c[axis] -= across;
            out[slot] = Some(RoadSpot { w, share: density * bell(across.abs(), half_a), centre: c, kind, density });
        }
    }
    out
}

/// A height levelled across the roads: `h` the ground's own height at the point, `spots` from
/// [`roads_near`] with the ground's height at each road's middle. The roads' heights are blended by
/// their shares (continuous where roads meet), then the point moves that far towards them by the
/// strongest road's `w`.
pub fn levelled(h: f64, spots: &[(RoadSpot, f64)]) -> f64 {
    let (mut wmax, mut sum, mut hs) = (0.0f64, 0.0f64, 0.0f64);
    for (s, hc) in spots {
        wmax = wmax.max(s.w as f64);
        sum += s.share as f64;
        hs += s.share as f64 * hc;
    }
    if sum <= 0.0 { h } else { h + wmax * (hs / sum - h) }
}

/// One settlement of settlement.wesl's field near a node.
#[derive(Clone, Copy, Debug)]
struct Blob {
    cell: [i32; 3],
    hash: [f32; 3],
    r2: f32,
    halo2: f32,
}

/// The settlements of one size that can reach a node.
#[derive(Clone, Debug)]
struct BlobSize {
    cell_km: f32,
    blobs: Vec<Blob>,
}

/// settlement.wesl's field over one ground node: the few settlements that can reach it, found once, so
/// that a node in empty country skips all its road work and the others sum a blob or two a sample
/// instead of 24-32 hashes. A sample gives what `settlements` near the ground gives.
#[derive(Clone, Debug)]
pub struct NodeField {
    sizes: Vec<BlobSize>,
    radius_km: f32,
    urban_k: f32,
}

impl NodeField {
    /// For a node centred on body-frame direction `dir`, all its samples within `node_km` of it, on a
    /// body of `radius_m`, a civilization of `level` with the field's seed `seed`.
    pub fn new(dir: [f32; 3], node_km: f32, radius_m: f32, level: u32, seed: u32) -> Self {
        let rk = radius_m / 1000.0;
        let p = [dir[0] * rk, dir[1] * rk, dir[2] * rk];
        let big = level >= 3;
        let mut params: Vec<(f32, f32, f32, f32, f32, u32)> = vec![(14.0, 0.45, 0.25, 0.9, 6.0, seed)];
        if level >= 2 {
            params.push((55.0, 0.9, if big { 2.5 } else { 1.5 }, if big { 10.0 } else { 7.0 }, 4.0, seed.wrapping_add(1)));
            params.push((280.0, if big { 0.7 } else { 0.65 }, if big { 20.0 } else { 10.0 }, if big { 70.0 } else { 35.0 }, 3.0, seed.wrapping_add(2)));
        }
        if level >= 4 {
            params.push((420.0, 0.7, 45.0, 130.0, 2.0, seed.wrapping_add(3)));
        }
        let mut sizes = Vec::with_capacity(params.len());
        for (cell, chance, r_lo, r_hi, halo, s) in params {
            let q = [p[0] / cell, p[1] / cell, p[2] / cell];
            let reach = node_km / cell;
            let range = |a: f32| ((a - reach - 0.5).floor() as i32, (a + reach - 0.5).floor() as i32 + 1);
            let (xr, yr, zr) = (range(q[0]), range(q[1]), range(q[2]));
            let mut blobs = Vec::new();
            for cz in zr.0..=zr.1 {
                for cy in yr.0..=yr.1 {
                    for cx in xr.0..=xr.1 {
                        let c = [cx, cy, cz];
                        let h = st_hash(c, s);
                        if h[3] >= chance {
                            continue;
                        }
                        let r = r_lo * (r_hi / r_lo).powf((h[3] * 7.31 + h[0] * 3.17).fract());
                        let halo_r = r * halo;
                        // Its light at the node's nearest point: too little to matter, it is left out.
                        let d = [(q[0] - (cx as f32 + h[0])) * cell, (q[1] - (cy as f32 + h[1])) * cell, (q[2] - (cz as f32 + h[2])) * cell];
                        let near = ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - node_km).max(0.0);
                        if near * near / (halo_r * halo_r) > 7.0 {
                            continue;
                        }
                        blobs.push(Blob { cell: c, hash: [h[0], h[1], h[2]], r2: r * r, halo2: halo_r * halo_r });
                    }
                }
            }
            sizes.push(BlobSize { cell_km: cell, blobs });
        }
        NodeField { sizes, radius_km: rk, urban_k: if level == 1 { 0.6 } else { 1.0 } }
    }

    /// No settlement reaches the node.
    pub fn is_empty(&self) -> bool {
        self.sizes.iter().all(|s| s.blobs.is_empty())
    }

    /// (urban density, farmland), 0..1, at body-frame direction `dir`.
    pub fn at(&self, dir: [f32; 3]) -> (f32, f32) {
        let p = [dir[0] * self.radius_km, dir[1] * self.radius_km, dir[2] * self.radius_km];
        let (mut urban, mut farm) = (0.0f32, 0.0f32);
        for s in &self.sizes {
            let q = [p[0] / s.cell_km, p[1] / s.cell_km, p[2] / s.cell_km];
            let base = [(q[0] - 0.5).floor() as i32, (q[1] - 0.5).floor() as i32, (q[2] - 0.5).floor() as i32];
            for b in &s.blobs {
                // (The shader's field sums the 8 cells round the point.)
                if (0..3).any(|i| b.cell[i] < base[i] || b.cell[i] > base[i] + 1) {
                    continue;
                }
                let d = [
                    (q[0] - (b.cell[0] as f32 + b.hash[0])) * s.cell_km,
                    (q[1] - (b.cell[1] as f32 + b.hash[1])) * s.cell_km,
                    (q[2] - (b.cell[2] as f32 + b.hash[2])) * s.cell_km,
                ];
                let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                urban += (-d2 / b.r2).exp();
                farm += (-d2 / b.halo2).exp();
            }
        }
        ((urban * self.urban_k).min(1.0), farm.min(1.0))
    }
}

/// A generated planet's town and countryside densities where planet.wesl draws its streets and
/// roads: the settlement field `st` (urban, farmland) on land that suits at height `h` (m above the
/// radius): below the highlands, above the sea, near the coast as the culture likes it (`coast`).
/// `lo`, `span` the ground's range (`Terrain::ground_range_m`); `sea` whether the body has an ocean;
/// `test` 1 under `SE_CITY_TEST`; `fert_min` the land's lowest fertility factor (0.3 on a body with
/// biomes, 1 without): the shader's factor comes from the biomes, which only the GPU has, so the land
/// is taken as barren: a road left tilting is less wrong than shelves cut for a road the shader does not
/// draw. Returns (town, rural) as `city_ground` takes them.
#[allow(clippy::too_many_arguments)]
pub fn generated_density(st: (f32, f32), h: f32, lo: f32, span: f32, sea: bool, coast: f32, test: f32, fert_min: f32) -> (f32, f32) {
    let hr = (h - if sea { 0.0 } else { lo }) / span.max(100.0);
    let mut land = 1.0 - smoothstep(if sea { 0.25 } else { 0.55 }, if sea { 0.5 } else { 0.8 }, hr);
    if sea {
        land *= smoothstep(2.0, 20.0, h) * (1.0 + coast * (1.0 - smoothstep(0.0, 0.03, hr)));
    }
    land *= fert_min;
    let kl = smoothstep(0.03, 0.3, land);
    ((st.0.max(test) * kl).clamp(0.0, 1.0), smoothstep(0.1, 0.3, st.1 * kl))
}

/// Earth's (a map body's) town and countryside densities from its lights map's brightest channel
/// `lum` (linear) and its water mask's `water`, as planet.wesl takes them.
pub fn lights_density(lum: f32, water: f32) -> (f32, f32) {
    let dry = 1.0 - smoothstep(0.2, 0.6, water);
    (smoothstep(0.04, 0.5, lum) * dry, smoothstep(0.004, 0.03, lum) * dry)
}

/// The layout's seed for a culture seed: the shader's `u32(civ.y * 10000.0) + 100u`, with `civ.y`
/// the seed's 4 digits over 10,000 in f32 (so the same rounding).
pub fn layout_seed(culture_seed: u32) -> u32 {
    field_seed(culture_seed) + 100
}

/// The settlement field's seed for a culture seed: the shader's `u32(civ.y * 10000.0)`.
pub fn field_seed(culture_seed: u32) -> u32 {
    let y = (culture_seed % 10_000) as f32 / 10_000.0;
    (y * 10_000.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// settlement.wesl's `st_size` near the ground, as its shader sums it (the field `NodeField` must match).
    fn st_size(p: [f32; 3], cell: f32, chance: f32, r_lo: f32, r_hi: f32, halo: f32, seed: u32) -> (f32, f32) {
        let q = [p[0] / cell, p[1] / cell, p[2] / cell];
        let base = [(q[0] - 0.5).floor() as i32, (q[1] - 0.5).floor() as i32, (q[2] - 0.5).floor() as i32];
        let (mut urban, mut farm) = (0.0f32, 0.0f32);
        for k in 0..8 {
            let c = [base[0] + (k & 1), base[1] + ((k >> 1) & 1), base[2] + ((k >> 2) & 1)];
            let h = st_hash(c, seed);
            if h[3] >= chance {
                continue;
            }
            let r = r_lo * (r_hi / r_lo).powf((h[3] * 7.31 + h[0] * 3.17).fract());
            let d = [(q[0] - (c[0] as f32 + h[0])) * cell, (q[1] - (c[1] as f32 + h[1])) * cell, (q[2] - (c[2] as f32 + h[2])) * cell];
            let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            urban += (-d2 / (r * r)).exp();
            farm += (-d2 / (r * r * halo * halo)).exp();
        }
        (urban, farm)
    }

    fn settlements(dir: [f32; 3], radius_m: f32, level: u32, seed: u32) -> (f32, f32) {
        let rk = radius_m / 1000.0;
        let p = [dir[0] * rk, dir[1] * rk, dir[2] * rk];
        let mut s = st_size(p, 14.0, 0.45, 0.25, 0.9, 6.0, seed);
        let mut add = |o: (f32, f32)| {
            s.0 += o.0;
            s.1 += o.1;
        };
        if level >= 2 {
            let big = level >= 3;
            add(st_size(p, 55.0, 0.9, if big { 2.5 } else { 1.5 }, if big { 10.0 } else { 7.0 }, 4.0, seed.wrapping_add(1)));
            add(st_size(p, 280.0, if big { 0.7 } else { 0.65 }, if big { 20.0 } else { 10.0 }, if big { 70.0 } else { 35.0 }, 3.0, seed.wrapping_add(2)));
        }
        if level >= 4 {
            add(st_size(p, 420.0, 0.7, 45.0, 130.0, 2.0, seed.wrapping_add(3)));
        }
        let k = if level == 1 { 0.6 } else { 1.0 };
        ((s.0 * k).min(1.0), s.1.min(1.0))
    }

    #[test]
    fn node_field_matches_the_shaders_sum() {
        let radius = 6.0e6f32;
        let mut empty = 0;
        for i in 0..400 {
            // Scattered node centres, a few samples round each (within 0.3 km).
            let a = i as f32 * 0.137;
            let b = i as f32 * 0.071;
            let c = [a.cos() * b.cos(), b.sin(), a.sin() * b.cos()];
            let level = 2 + (i % 3) as u32;
            let f = NodeField::new(c, 0.4, radius, level, 4321);
            empty += f.is_empty() as u32;
            for j in 0..6 {
                let t = j as f32 * 0.05 / 6000.0;
                let d = [c[0] + t, c[1] - t * 0.7, c[2] + t * 0.3];
                let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let d = [d[0] / n, d[1] / n, d[2] / n];
                let (u0, f0) = settlements(d, radius, level, 4321);
                let (u1, f1) = f.at(d);
                // What it leaves out can never reach the road's thresholds (0.06 urban, 0.1 farmland).
                assert!((u0 - u1).abs() < 0.01 && (f0 - f1).abs() < 0.01, "{i} {j}: {u0} {u1} / {f0} {f1}");
            }
        }
        assert!(empty < 400, "some nodes have settlements");
    }

    #[test]
    fn grid_round_trips() {
        let cp = CityPos::from_face_m(1.234e6, -3.21e5);
        let d = district(cp, 4242);
        let g = grid(d.cell, 4242, 0.5);
        let lp = local(cp, d.cell, &g);
        let back = unlocal(cp, d.cell, &g, lp);
        assert!((back[0] - cp.q[0]).abs() < 1e-2 && (back[1] - cp.q[1]).abs() < 1e-2, "{back:?} {:?}", cp.q);
    }

    #[test]
    fn streets_lie_on_their_middle_line() {
        // Walk across a district: wherever a street is found, its middle point must be on a line
        // of the grid (distance 0 to it) and the point's distance to that middle within the street.
        let seed = 7123;
        let mut found = 0;
        for i in 0..400 {
            let cp = CityPos::from_face_m(2.0e6 + i as f64 * 1.37, 5.0e5 + i as f64 * 0.61);
            let spots = roads_near(cp, 1.0, 1.0, 1.0, seed, 0.3, 3.0);
            for s in spots.iter().take(2).flatten() {
                let c = CityPos { tile: cp.tile, q: s.centre };
                let d = district(cp, seed);
                let g = grid(d.cell, seed, 0.3);
                let lp = local(c, d.cell, &g);
                let b = [lp[0] - (lp[0] / g.size[0]).floor() * g.size[0], lp[1] - (lp[1] / g.size[1]).floor() * g.size[1]];
                let on = (0..2).any(|a| b[a].min(g.size[a] - b[a]) < 0.05);
                assert!(on, "middle {:?} not on a street line: {b:?}", s.centre);
                let dist = ((s.centre[0] - cp.q[0]).powi(2) + (s.centre[1] - cp.q[1]).powi(2)).sqrt();
                assert!(dist <= 7.0 + 3.0 + 1e-3, "{dist}");
                found += 1;
            }
        }
        assert!(found > 20, "{found}");
    }

    #[test]
    fn levelling_blends_and_keeps_open_ground() {
        let s = |w, share| RoadSpot { w, share, centre: [0.0; 2], kind: RoadKind::Street, density: 1.0 };
        assert_eq!(levelled(10.0, &[]), 10.0);
        assert_eq!(levelled(10.0, &[(s(0.0, 0.0), 4.0)]), 10.0);
        assert!((levelled(10.0, &[(s(1.0, 1.0), 4.0)]) - 4.0).abs() < 1e-12);
        assert!((levelled(10.0, &[(s(1.0, 1.0), 4.0), (s(1.0, 1.0), 6.0)]) - 5.0).abs() < 1e-12);
        assert!((levelled(10.0, &[(s(0.5, 0.5), 4.0)]) - 7.0).abs() < 1e-12);
    }

    /// The levelled height on a plane rising along `slope`, at (x, y) face metres (k = 1).
    fn level_on_plane(x: f64, y: f64, slope: (f64, f64), seed: u32) -> f64 {
        let h = |x: f64, y: f64| slope.0 * x + slope.1 * y;
        let cp = CityPos::from_face_m(x, y);
        let spots: Vec<(RoadSpot, f64)> = roads_near(cp, 1.0, 1.0, 1.0, seed, 0.3, 4.0)
            .into_iter()
            .flatten()
            .map(|s| {
                let (cx, cy) = cp.face_m(s.centre);
                (s, h(cx, cy))
            })
            .collect();
        levelled(h(x, y), &spots)
    }

    #[test]
    fn levelled_ground_has_no_steps() {
        // Round an arterial crossing (the lines bend at most 55 m), every metre: a pair of points 2 cm
        // apart must have nearly the same height wherever they fall. On a 30 % slope, along x and on the
        // diagonal: a step anywhere (a road's weight jumping on a diagonal or on a district's line) fails.
        let (x0, y0) = (512.0 * 4096.0, 512.0 * 2048.0);
        for slope in [(0.3, 0.0), (0.0, 0.3), (0.21, 0.21)] {
            let mut worst: (f64, f64, f64) = (0.0, 0.0, 0.0);
            for j in -130i32..130 {
                for i in -130i32..130 {
                    let (x, y) = (x0 + i as f64, y0 + j as f64);
                    let a = level_on_plane(x, y, slope, 7123);
                    let b = level_on_plane(x + 0.02, y + 0.01, slope, 7123);
                    if (a - b).abs() > worst.0 {
                        worst = ((a - b).abs(), x - x0, y - y0);
                    }
                }
            }
            assert!(worst.0 < 0.15, "a step of {} m at {:?} on slope {slope:?}", worst.0, (worst.1, worst.2));
        }
    }

    #[test]
    fn a_road_is_level_across_away_from_junctions() {
        // In town on a slope, along an arterial far from its crossings: the two sides match.
        let seed = 7123;
        let slope = (0.3, 0.0);
        let mut checked = 0;
        for j in 0..80 {
            let y = 512.0 * 2048.0 + 170.0 + j as f64 * 2.0;
            // The line of constant x nearest, from the layout.
            let cp = CityPos::from_face_m(512.0 * 4096.0, y);
            let d = lines(cp, seed);
            let mid = 512.0 * 4096.0 - d.x.1 as f64;
            let across = |o: f64| level_on_plane(mid + o, y, slope, seed);
            // (Where no street runs into it: their middles are at another height.)
            let cps = CityPos::from_face_m(mid + 5.0, y);
            if roads_near(cps, 1.0, 1.0, 1.0, seed, 0.3, 4.0)[0].is_some() || roads_near(cps, 1.0, 1.0, 1.0, seed, 0.3, 4.0)[1].is_some() {
                continue;
            }
            assert!((across(-5.0) - across(5.0)).abs() < 0.2, "{} {}", across(-5.0), across(5.0));
            checked += 1;
        }
        assert!(checked > 0, "no stretch without streets");
    }

    #[test]
    fn seeds_round_as_the_shader() {
        let y = (1_234_567u32 % 10_000) as f32 / 10_000.0;
        assert_eq!(field_seed(1_234_567), (y * 10_000.0) as u32);
        assert_eq!(layout_seed(1_234_567), field_seed(1_234_567) + 100);
    }

    #[test]
    fn density_follows_the_shader() {
        // Earth: a bright town on land, the same over water, dim countryside.
        assert_eq!(lights_density(0.8, 0.0).0, 1.0);
        assert_eq!(lights_density(0.8, 1.0), (0.0, 0.0));
        assert_eq!(lights_density(0.0, 0.0), (0.0, 0.0));
        // Generated: at the barren worst, highlands are left out, low land kept.
        let (town, _) = generated_density((1.0, 1.0), 100.0, 0.0, 10_000.0, true, 0.0, 0.0, 0.3);
        assert!(town > 0.99, "{town}");
        let (town, _) = generated_density((1.0, 1.0), 4000.0, 0.0, 10_000.0, true, 0.0, 0.0, 0.3);
        assert!(town < 0.5, "{town}");
    }
}
