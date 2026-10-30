//! SpaceEngine's star octree of a galaxy, decompiled: `seStarNodeCache::CreateGalaxyBlock`
//! (FUN_14053b5d0), the node set-up (FUN_140543c70), the galaxy's root grid (`seGalSysModel::Build`,
//! FUN_1404a1a10 / FUN_1404a40b0), the magnitude bands (FUN_1401d7300) and the names (FUN_1401c0180).
//!
//! - **Roots.** The galaxy's box (half extents `dims`, record +0x40..+0x48) is cut into a regular grid
//!   of 2^d cells per axis, `d = clamp((int)log2(dims.x / (2000 / 2^sc)) + 1, 1, 6)`, `sc` the galaxy's
//!   size class `clamp((int)log2(4000 / R), 0, 5)`. Roots are numbered depth first in SE's octant order
//!   (octant bits 4, 2, 1 mark the lower x, y, z half), so a root's number is its base-8 octant path.
//! - **Nodes.** Every root holds an octree of `9 - sc` levels; a child's number is its parent's x 8
//!   plus its octant. Here a node is the cell of the global grid of depth `d + level`.
//! - **Bands.** Level `L` holds the stars of the `L`-th of `9 - sc` equal-probability slices of
//!   N(0, 6) over Mv -8 .. 30 (the last edge capped at p 0.9999, Mv 22.3): coarse nodes hold the rare
//!   bright stars, the deepest ones the dwarfs.
//! - **Candidates.** Every node tries the same number of points, `16384 x 0.3 x` the root volume over
//!   `(2000 / 2^sc)^3` pc^3 (at least `0.015 x 16384`), split among the galaxy's components (model
//!   disk 1 and bulge 1 for types S0..SBc, young disk 0.25 for S0..SBc, halo 0.1 for all but Irr),
//!   at the deepest level `1 / (1 - ProbPlanemo - ProbSolitaryBH)` times as many. Nodes past level 4
//!   with fewer than 64 tries, and nodes past level 2 that keep fewer than 64 stars, are empty.
//! - **Acceptance.** A galaxy with a sprite model's emission voxel map keeps a uniform point of the
//!   node with probability `0.15 x` the map's red channel there, as a young disk star. Without a map,
//!   SE's analytic components: young disk `exp(-|(3x, 3y, 120z)|)` inside the box's sphere, bulge
//!   `exp(-|(37x, 37y, 45z)|)` and halo `exp(-8|p|)` (old stars), `p` in box units.
//! - **Stars.** SE's star generator ([`cluster_stars::star_of`]); at the deepest level its leaf draw:
//!   45 % rogue planets (planemos), 3.3 % of the rest lone white dwarfs, neutron stars and black holes.
//! - **Random numbers.** SE's LCG pair seeded with `(node * 0x56a + level * 0x24db + root * 0x1430) x`
//!   the galaxy's seed, a component multiplier per stream, three position draws then one acceptance
//!   draw per point, as in the exe. SE's star generator draws from the same stream; here each kept point
//!   types its star from a stream of its own (seeded by the node and the point), so a star's position is
//!   SE's rule for SE's stream while its type is SE's rule for another stream.
//! - **Names.** `RS <galaxy>-<root>-<level>-<node>-<index>` (catalog galaxies) and
//!   `RS 0-<level>-<node>-<index>-<root>-<level>-<node>-<index>` (procedural ones); the index counts
//!   the stars the node keeps. SE's level-0 nodes first hold the catalog stars of their root, which shift
//!   SE's indices there; the port's start at 0.
//!
//! Positions are in the galaxy's model frame (parsecs, disk normal = -Z, as for catalog galaxies),
//! relative to the galaxy centre.

use std::sync::Arc;

use glam::{DQuat, DVec3};

use crate::cluster_stars::{self as cs, LeafDraw, Member, Population};
use crate::galaxy_model::DensityMap;
use crate::proc_galsys::GalaxyRef;
use crate::star::{SpectralClass, StarKind, blackbody_rgb};

/// SE's most stars (and tries per component) in one node (`0x4000`).
pub const NODE_CAP: usize = 16_384;
/// SE's deepest root grid (`GOCT_MAX_DEPTH`) and most star levels (`0x140762a88`).
const MAX_DEPTH: u32 = 6;
const MAX_LEVELS: u8 = 9;
/// Root cell edge (pc) of a size class 0 galaxy and the size class reference radius.
const CELL_PC: f64 = 2000.0;
const SIZE_CLASS_PC: f64 = 4000.0;
/// `1 / (1 - ProbPlanemo - ProbSolitaryBH)` (`0x140807fdc`): room for the deepest level's planemos.
const LEAF_BOOST: f32 = 1.0 / ((1.0 - 0.45) - 3e-5);
/// Voxel map gain (`0x1406d4190`).
const VOXEL_GAIN: f32 = 0.15;
/// The magnitude bands: N(MEAN, 6) from Mv -8 to 30 (`0x140808174` is never written: 0).
const BAND_MEAN: f64 = 0.0;
const BAND_SIGMA_SQRT2: f64 = 8.485_281_374_238_571;
/// Keep a node while its brightest band edge is this many magnitudes past the limit (star fade-in).
const REACH_MARGIN_MAG: f64 = 0.5;

/// What SE knows about a galaxy when it fills it with stars.
#[derive(Debug, Clone, Copy)]
pub struct GalaxyStarInput {
    /// Half extents of the galaxy's box along the model x, y, z axes (record +0x40..+0x48), pc.
    pub dims: DVec3,
    /// The galaxy's radius (record +0x20), pc: its size class.
    pub radius_pc: f64,
    /// Hubble type index 1..=16 (0 unknown).
    pub hubble: u8,
    /// Whether it has a sprite model (record +0x68 >= 0).
    pub has_model: bool,
    /// The galaxy's seed (record +0x60, [`crate::proc_galsys::GalaxyShape::seed`]).
    pub seed: i32,
    /// The galaxy, for the star names.
    pub galaxy: GalaxyRef,
}

/// How a galaxy accepts its candidate points.
#[derive(Clone, Debug)]
pub enum StarDensity {
    /// The emission voxel map of the galaxy's sprite model (SE's voxel branch).
    Voxel(Arc<DensityMap>),
    /// SE's analytic components, for galaxies without a voxel map.
    Analytic,
}

/// SE's `StarGenerator` limits in the home galaxy (main-def.cfg): stars brighter than `max_app_mag`
/// as seen from the Sun, or nearer to it than their class's minimum distance, are left to the
/// catalogs.
#[derive(Debug, Clone, Copy)]
pub struct HomeCut {
    /// The Sun, model frame (pc).
    pub sun: DVec3,
    pub max_app_mag: f64,
    pub min_dist_normal: f64,
    pub min_dist_brown: f64,
    pub min_dist_remnant: f64,
}

/// A galaxy's star octree.
#[derive(Debug, Clone)]
pub struct StarOctree {
    pub galaxy: GalaxyRef,
    pub dims: DVec3,
    pub size_class: u8,
    /// Depth of the root grid.
    pub depth: u8,
    /// Star levels per root (`9 - size_class`).
    pub levels: u8,
    seed: u32,
    /// Tries per node of the components (model disk, young disk, bulge, halo).
    counts: [u32; 4],
    /// Their sum, and at the deepest level the boosted sum.
    total: u32,
    leaf_total: u32,
    /// Absolute magnitude band of each level.
    bands: Vec<(f64, f64)>,
    /// Apparent magnitude limit the node reach is measured for.
    pub mag_limit: f64,
}

/// Whether the sphere at `centre` of radius `r` meets the cone from `apex` along the unit `axis`
/// with half-angle `half` (radians).
pub fn sphere_meets_cone(apex: DVec3, (axis, half): (DVec3, f64), centre: DVec3, r: f64) -> bool {
    let (sin_a, cos_a) = half.sin_cos();
    let v = centre - apex;
    let along = v.dot(axis);
    let across = (v - axis * along).length();
    if along * cos_a + across * sin_a < 0.0 {
        // Nearest to the apex.
        v.length() <= r
    } else {
        across * cos_a - along * sin_a <= r
    }
}

/// Node address: star level and the cell of the global grid of depth `depth + level` (cells per axis
/// `2^(depth + level)`, the grid spanning the galaxy's box).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey {
    pub level: u8,
    pub cell: [i32; 3],
}

#[derive(Debug, Clone)]
pub struct ProcStar {
    /// Model-frame position (pc) relative to the galaxy centre.
    pub pos: DVec3,
    /// Absolute V magnitude (infinite for a planemo).
    pub abs_mag: f32,
    pub teff: f32,
    pub rgb: [f32; 3],
    pub kind: StarKind,
    /// SE's class text ("G5.5V", "WC5", "C-N3", "DA9.5", "Q", "X"; "Planemo").
    pub class: String,
    /// Index within its node (stable).
    pub index: u32,
    /// SE's generated mass (solar), age (Gyr), [Fe/H] and C/O.
    pub mass_sol: f32,
    pub age_gyr: f32,
    pub feh: f32,
    pub c_to_o: f32,
}

impl ProcStar {
    /// Whether the star gives light (planemos are dark).
    pub fn luminous(&self) -> bool {
        self.abs_mag.is_finite()
    }
}

/// SE's LCG step (`x * 1664525 + 1013904223`).
fn lcg(s: &mut u32) -> u32 {
    *s = s.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
    *s
}

/// A uniform `[0, 1)` draw as SE makes it (`(float)u * 2^-32`).
fn unit(s: &mut u32) -> f32 {
    lcg(s) as f32 * 2.328_306_4e-10
}

/// An offset in `[-h, h)` from the node centre (SE's float arithmetic).
fn offset(s: &mut u32, h: f64) -> f32 {
    let u = lcg(s) as f32;
    (((h + h) as f32 * u * 2.328_306_4e-10) as f64 - h) as f32
}

/// SE's base-8 octant path of a cell's low `bits` bits (octant bits 4, 2, 1 for the lower x, y, z half).
fn path_of(cell: [i32; 3], bits: u32) -> u64 {
    (0..bits).rev().fold(0u64, |n, j| {
        let bit = |c: i32| 1 - ((c >> j) & 1) as u64;
        n * 8 + (bit(cell[0]) << 2 | bit(cell[1]) << 1 | bit(cell[2]))
    })
}

/// Inverse of [`path_of`]: the cell of a path of `bits` octants.
fn cell_of_path(path: u64, bits: u32) -> Option<[i32; 3]> {
    if bits > 20 || path >= 8u64.pow(bits) {
        return None;
    }
    let mut c = [0i32; 3];
    for j in (0..bits).rev() {
        let k = (path >> (3 * j)) & 7;
        c[0] = c[0] * 2 + (1 - ((k >> 2) & 1)) as i32;
        c[1] = c[1] * 2 + (1 - ((k >> 1) & 1)) as i32;
        c[2] = c[2] * 2 + (1 - (k & 1)) as i32;
    }
    Some(c)
}

/// SE's magnitude bands of a galaxy of `levels` levels (FUN_1401d7300).
fn bands(levels: u8) -> Vec<(f64, f64)> {
    let cdf = |m: f64| (crate::surface_gen::erf((m - BAND_MEAN) / BAND_SIGMA_SQRT2) + 1.0) * 0.5;
    let (lo, hi) = (cdf(-8.0), cdf(30.0));
    let step = (hi - lo) / levels as f64;
    let mut p = lo as f32;
    let mut edges = Vec::with_capacity(levels as usize + 1);
    for _ in 0..=levels {
        edges.push(BAND_MEAN + BAND_SIGMA_SQRT2 / std::f64::consts::SQRT_2 * crate::surface_gen::inv_norm_cdf(p as f64));
        p = ((p as f64 + step) as f32).min(0.9999);
    }
    edges.windows(2).map(|w| (w[0], w[1])).collect()
}

/// A galaxy's size class (FUN_1404648b0): `clamp((int)log2(4000 / R), 0, 5)`, `R` capped at 50 kpc.
pub fn size_class(radius_pc: f64) -> u8 {
    let r = (radius_pc as f32).min(50_000.0);
    ((SIZE_CLASS_PC / r as f64).log2() as i32).clamp(0, 5) as u8
}

impl StarOctree {
    pub fn new(g: &GalaxyStarInput, mag_limit: f64) -> Self {
        let sc = size_class(g.radius_pc);
        let unit = CELL_PC / (1u32 << sc) as f64;
        let d = (((g.dims.x as f32) / unit as f32).log2() as i32 + 1).clamp(1, 9).min(MAX_DEPTH as i32) as u32;
        let cells = (1u32 << d) as f32;
        let (sx, sy, sz) = (g.dims.x as f32, g.dims.y as f32, g.dims.z as f32);
        let vol = ((sx + sx) / cells) * ((sy + sy) / cells) * ((sz + sz) / cells);
        let mut base = (vol as f64 / unit.powi(3)) as f32 * 0.3;
        if base < 0.015 {
            base = 0.015;
        }
        // Component weights (FUN_140543c70).
        let disk = (9..=15).contains(&g.hubble);
        let model = (9..=16).contains(&g.hubble) && g.has_model;
        let w = [
            if model { 1.0f32 } else { 0.0 },
            if disk { 0.25 } else { 0.0 },
            if disk { 1.0 } else { 0.0 },
            if g.hubble & 0xef != 0 { 0.1 } else { 0.0 },
        ];
        let sum: f32 = w.iter().sum();
        let f = if sum != 0.0 { (base / sum) * 16384.0 } else { 0.0 };
        let counts = w.map(|w| (f * w) as u32);
        let total: u32 = counts.iter().sum();
        let levels = (MAX_LEVELS as i32 - sc as i32).clamp(1, MAX_LEVELS as i32) as u8;
        Self {
            galaxy: g.galaxy,
            dims: g.dims,
            size_class: sc,
            depth: d as u8,
            levels,
            seed: g.seed as u32,
            counts,
            total,
            leaf_total: (total as f32 * LEAF_BOOST) as u32,
            bands: bands(levels),
            mag_limit,
        }
    }

    /// Absolute-magnitude band [bright, faint) of a level.
    pub fn mag_band(&self, level: u8) -> (f64, f64) {
        self.bands[level.min(self.levels - 1) as usize]
    }

    fn grid_bits(&self, level: u8) -> u32 {
        self.depth as u32 + level as u32
    }

    /// Half extents of a node of `level` (pc, model frame).
    pub fn node_half(&self, level: u8) -> DVec3 {
        self.dims / (1u64 << self.grid_bits(level)) as f64
    }

    pub fn node_center(&self, key: NodeKey) -> DVec3 {
        let h = self.node_half(key.level);
        let c = DVec3::new(key.cell[0] as f64, key.cell[1] as f64, key.cell[2] as f64);
        -self.dims + (c * 2.0 + DVec3::ONE) * h
    }

    /// Cell containing model position `p` at `level` (may lie outside the grid).
    pub fn cell_of(&self, level: u8, p: DVec3) -> [i32; 3] {
        let q = (p + self.dims) / (self.node_half(level) * 2.0);
        [q.x.floor() as i32, q.y.floor() as i32, q.z.floor() as i32]
    }

    fn in_grid(&self, level: u8, c: [i32; 3]) -> bool {
        let n = 1i64 << self.grid_bits(level);
        c.iter().all(|&v| v >= 0 && (v as i64) < n)
    }

    /// SE's root and node numbers of a node.
    pub fn root_node(&self, key: NodeKey) -> (u32, u64) {
        let l = key.level as u32;
        let root = path_of(key.cell.map(|c| c >> l), self.depth as u32) as u32;
        let node = path_of(key.cell.map(|c| c & ((1 << l) - 1)), l);
        (root, node)
    }

    /// The node of SE's root, level and node numbers.
    pub fn key_of(&self, root: u32, level: u8, node: u64) -> Option<NodeKey> {
        if level >= self.levels {
            return None;
        }
        let r = cell_of_path(root as u64, self.depth as u32)?;
        let n = cell_of_path(node, level as u32)?;
        let l = level as u32;
        Some(NodeKey { level, cell: [0, 1, 2].map(|a| (r[a] << l) | n[a]) })
    }

    /// SE's seed of a node (FUN_14053b5d0).
    pub fn seed(&self, key: NodeKey) -> u32 {
        let (root, node) = self.root_node(key);
        (node as u32)
            .wrapping_mul(0x56a)
            .wrapping_add((key.level as u32).wrapping_mul(0x24db))
            .wrapping_add(root.wrapping_mul(0x1430))
            .wrapping_mul(self.seed)
    }

    /// The star's name, `RS <galaxy>-<root>-<level>-<node>-<index>` (FUN_1401c0180).
    pub fn star_name(&self, key: NodeKey, index: u32) -> String {
        let (root, node) = self.root_node(key);
        match self.galaxy {
            GalaxyRef::Catalog(g) => format!("RS {g}-{root}-{}-{node}-{index}", key.level),
            GalaxyRef::Procedural(k, gi) => format!("RS 0-{}-{}-{gi}-{root}-{}-{node}-{index}", k.level, k.id, key.level),
        }
    }

    /// Node and index of a star name of this galaxy (any case), the inverse of [`star_name`](Self::star_name).
    pub fn parse_name(&self, name: &str) -> Option<(NodeKey, u32)> {
        let s = name.trim();
        if !s.get(..3)?.eq_ignore_ascii_case("RS ") {
            return None;
        }
        let n: Vec<u64> = s[3..].trim().split('-').map(|p| p.trim().parse::<u64>()).collect::<Result<_, _>>().ok()?;
        let tail = match (self.galaxy, n.as_slice()) {
            (GalaxyRef::Catalog(g), [a, rest @ ..]) if *a == g as u64 && rest.len() == 4 => rest,
            (GalaxyRef::Procedural(k, gi), [0, l, id, i, rest @ ..]) if *l == k.level as u64 && *id == k.id && *i == gi as u64 && rest.len() == 4 => rest,
            _ => return None,
        };
        let key = self.key_of(u32::try_from(tail[0]).ok()?, u8::try_from(tail[1]).ok()?, tail[2])?;
        Some((key, u32::try_from(tail[3]).ok()?))
    }

    /// How far (pc) a node of `level` stays loaded: until its brightest stars fall below the limit.
    pub fn reach(&self, level: u8) -> f64 {
        10f64.powf(1.0 + (self.mag_limit + REACH_MARGIN_MAG - self.mag_band(level).0) / 5.0)
    }

    /// Distance from `p` to a node's box (0 inside).
    pub fn box_distance(&self, key: NodeKey, p: DVec3) -> f64 {
        ((p - self.node_center(key)).abs() - self.node_half(key.level)).max(DVec3::ZERO).length()
    }

    /// Nodes to load for a camera at model position `cam`, by level (nearest first within a level):
    /// those within the level's [`reach`](Self::reach), and with `reach` > 1 (a magnitude limit
    /// `5 log10(reach)` above the one the reach is measured for) also those out to `reach` times as
    /// far whose bounding sphere meets `cone` (unit axis, half-angle in radians), or every one that
    /// far without a cone. SE keeps a node of stars while its brightest star could still show
    /// (FUN_1404a0e80) and clips them to the view frustum when the field of view is under
    /// `StarFrustumClipFOV` (FUN_14047b670). At most `max_far` nodes per level beyond the usual
    /// reach are kept, the nearest.
    pub fn wanted_nodes(&self, cam: DVec3, reach: f64, cone: Option<(DVec3, f64)>, max_far: usize) -> Vec<Vec<NodeKey>> {
        let reach = reach.max(1.0);
        (0..self.levels)
            .map(|level| {
                let near = self.reach(level);
                let far = near * reach;
                let half = self.node_half(level);
                let radius = half.length();
                let mut v: Vec<(f64, NodeKey)> = Vec::new();
                self.for_cells_near(level, cam, far, |key| {
                    let d = self.box_distance(key, cam);
                    if d <= near || (d <= far && cone.is_none_or(|k| sphere_meets_cone(cam, k, self.node_center(key), radius))) {
                        v.push((d, key));
                    }
                });
                v.sort_by(|a, b| a.0.total_cmp(&b.0));
                let n = v.partition_point(|(d, _)| *d <= near);
                v.truncate(n.saturating_add(max_far));
                v.into_iter().map(|(_, k)| k).collect()
            })
            .collect()
    }

    /// Calls `f` for every node of `level` in the grid whose cell meets the cube of half-side `r`
    /// around `p`.
    pub fn for_cells_near(&self, level: u8, p: DVec3, r: f64, mut f: impl FnMut(NodeKey)) {
        let n = (1i64 << self.grid_bits(level)) as i32;
        let lo = self.cell_of(level, p - DVec3::splat(r));
        let hi = self.cell_of(level, p + DVec3::splat(r));
        for x in lo[0].max(0)..=hi[0].min(n - 1) {
            for y in lo[1].max(0)..=hi[1].min(n - 1) {
                for z in lo[2].max(0)..=hi[2].min(n - 1) {
                    f(NodeKey { level, cell: [x, y, z] });
                }
            }
        }
    }

    /// Generate the stars of one node, keeping those whose position passes `keep` (the others are
    /// still generated, so every star keeps its index).
    pub fn generate_where(&self, key: NodeKey, density: &StarDensity, home: Option<&HomeCut>, keep: &dyn Fn(DVec3) -> bool) -> Vec<ProcStar> {
        if key.level >= self.levels || !self.in_grid(key.level, key.cell) {
            return Vec::new();
        }
        let leaf = key.level == self.levels - 1;
        let tries = if leaf { self.leaf_total } else { self.total };
        if key.level > 4 && tries < 64 {
            return Vec::new();
        }
        let seed = self.seed(key);
        let centre = self.node_center(key);
        let half = self.node_half(key.level);
        let (lo, hi) = self.mag_band(key.level);
        let dims = self.dims.as_vec3();
        let mut out: Vec<ProcStar> = Vec::new();
        let mut ordinal = 0u64;
        let cap = (tries as usize).min(NODE_CAP);
        // One component's loop: SE's stream multiplier, tries, and per point the acceptance and the
        // population (None: rejected).
        let mut run = |mult: u32, n: u32, accept: &mut dyn FnMut([f32; 3], &mut u32) -> Option<Population>, out: &mut Vec<ProcStar>| {
            let mut s = seed.wrapping_mul(mult);
            let n = (n as usize).min(NODE_CAP.saturating_sub(out.len()));
            for _ in 0..n {
                if out.len() >= cap {
                    break;
                }
                let o = [offset(&mut s, half.x), offset(&mut s, half.y), offset(&mut s, half.z)];
                let p = [
                    ((o[0] as f64 + centre.x) / dims.x as f64) as f32,
                    ((o[1] as f64 + centre.y) / dims.y as f64) as f32,
                    ((o[2] as f64 + centre.z) / dims.z as f64) as f32,
                ];
                let Some(pop) = accept(p, &mut s) else { continue };
                ordinal += 1;
                let pos = centre + DVec3::new(o[0] as f64, o[1] as f64, o[2] as f64);
                let mut rng = Rng(splitmix(seed as u64 ^ ordinal.wrapping_mul(0x9E37_79B9_7F4A_7C15)));
                let star = make_star(&mut rng, pop, lo, hi, leaf, pos, out.len() as u32);
                if home.is_some_and(|h| h.excludes(&star)) {
                    continue;
                }
                out.push(star);
            }
        };
        match density {
            StarDensity::Voxel(map) => {
                let mut accept = |p: [f32; 3], s: &mut u32| {
                    let v = map.sample_se(p[0], p[1], p[2]);
                    (unit(s) <= v * VOXEL_GAIN).then(|| Population::Disk { r: (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() as f64 })
                };
                run(0x2103, tries, &mut accept, &mut out);
            }
            StarDensity::Analytic => {
                // SE's model-disk component reads the model's 2D front map, which a galaxy without a
                // voxel map has not got here: its tries are skipped.
                let mut young = |p: [f32; 3], s: &mut u32| {
                    if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] >= 1.0 {
                        return None;
                    }
                    let r = ((p[1] * 3.0) * (p[1] * 3.0) + (p[0] * 3.0) * (p[0] * 3.0) + (p[2] * 120.0) * (p[2] * 120.0)).sqrt();
                    (unit(s) <= (-r).exp()).then_some(Population::Disk { r: r as f64 })
                };
                run(0x1651d, self.counts[1], &mut young, &mut out);
                let mut bulge = |p: [f32; 3], s: &mut u32| {
                    let q = [p[0] as f64 * 37.0, p[1] as f64 * 37.0, p[2] as f64 * 45.0].map(|v| v as f32);
                    let r = (q[1] * q[1] + q[0] * q[0] + q[2] * q[2]).sqrt();
                    (unit(s) <= (-r).exp()).then_some(Population::Old)
                };
                run(0x24d7, self.counts[2], &mut bulge, &mut out);
                let mut halo = |p: [f32; 3], s: &mut u32| {
                    let q = p.map(|v| (v as f64 * 8.0) as f32);
                    let r = (q[1] * q[1] + q[0] * q[0] + q[2] * q[2]).sqrt();
                    (unit(s) <= (-r).exp()).then_some(Population::Old)
                };
                run(0x4bf, self.counts[3], &mut halo, &mut out);
            }
        }
        if key.level > 2 && out.len() < 64 {
            return Vec::new();
        }
        out.retain(|s| keep(s.pos));
        out
    }
}

impl HomeCut {
    /// Whether the catalogs, not the generator, hold this star: too bright as seen from the Sun, or
    /// too near it for its class (FUN_14053b5d0 with `StarMinDist*`).
    pub fn excludes(&self, s: &ProcStar) -> bool {
        let d = (s.pos - self.sun).length().max(1e-6);
        let min = match s.kind {
            StarKind::WhiteDwarf | StarKind::NeutronStar | StarKind::BlackHole | StarKind::Planemo => self.min_dist_remnant,
            StarKind::BrownDwarf => self.min_dist_brown,
            _ => self.min_dist_normal,
        };
        d <= min || (s.luminous() && s.abs_mag as f64 + 5.0 * (d / 10.0).log10() < self.max_app_mag)
    }
}

/// The star of a kept point (FUN_1401d63b0; `leaf`: the deepest level's draw).
fn make_star(rng: &mut Rng, pop: Population, lo: f64, hi: f64, leaf: bool, pos: DVec3, index: u32) -> ProcStar {
    let m: Member = if leaf {
        match cs::leaf_draw(rng, pop, lo, hi) {
            LeafDraw::Planemo { age_gyr } => {
                return ProcStar {
                    pos,
                    abs_mag: f32::INFINITY,
                    teff: 0.0,
                    rgb: [0.0; 3],
                    kind: StarKind::Planemo,
                    class: "Planemo".into(),
                    index,
                    mass_sol: 0.0,
                    age_gyr,
                    feh: 0.0,
                    c_to_o: 0.55,
                };
            }
            LeafDraw::Star(m) => m,
        }
    } else {
        cs::star_of(rng, pop, lo, hi)
    };
    let kind = m.class.kind();
    let rgb = match kind {
        StarKind::BlackHole => [0.0; 3],
        StarKind::NeutronStar => [0.75, 0.85, 1.0],
        _ => blackbody_rgb(m.teff as f64),
    };
    ProcStar {
        pos,
        abs_mag: m.abs_mag,
        teff: m.teff,
        rgb,
        kind,
        class: m.class.name(),
        index,
        mass_sol: m.mass,
        age_gyr: m.age_gyr,
        feh: m.feh,
        c_to_o: m.c_to_o,
    }
}

/// Rotation and placement of a galaxy's model frame in the universe.
#[derive(Debug, Clone, Copy)]
pub struct GalaxyFrame {
    /// Galaxy centre, parsecs from the Solar System (universe frame).
    pub center_pc: DVec3,
    /// Model -> universe rotation.
    pub rot: DQuat,
}

impl GalaxyFrame {
    pub fn to_model(&self, universe_pc: DVec3) -> DVec3 {
        self.rot.inverse() * (universe_pc - self.center_pc)
    }

    pub fn to_universe(&self, model_pc: DVec3) -> DVec3 {
        self.center_pc + self.rot * model_pc
    }
}

/// Main-sequence spectral class letter and subclass for effective temperature `t`, e.g. "G2".
pub fn ms_class_name(t: f64) -> String {
    let table = [
        (30000.0, 'O', 50000.0),
        (10000.0, 'B', 30000.0),
        (7500.0, 'A', 10000.0),
        (6000.0, 'F', 7500.0),
        (5200.0, 'G', 6000.0),
        (3700.0, 'K', 5200.0),
        (2400.0, 'M', 3700.0),
        (0.0, 'L', 2400.0),
    ];
    for (lo, letter, hi) in table {
        if t >= lo {
            let sub = ((hi - t) / (hi - lo) * 10.0).clamp(0.0, 9.0).floor();
            return format!("{letter}{sub}");
        }
    }
    "M9".into()
}

/// The spectral class of a procedural star, for consumers that need it parsed.
pub fn parse_class(s: &ProcStar) -> SpectralClass {
    SpectralClass::parse(&s.class)
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn poisson(&mut self, lambda: f64) -> u64 {
        if lambda < 30.0 {
            let l = (-lambda).exp();
            let mut k = 0u64;
            let mut p = 1.0;
            loop {
                p *= self.f64();
                if p <= l {
                    return k;
                }
                k += 1;
            }
        }
        let u1 = self.f64().max(1e-300);
        let u2 = self.f64();
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        (lambda + z * lambda.sqrt()).round().max(0.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Milky Way like SE's: radius 19.33 kpc, cubic box (its model's scale is 1, 1, 1), type SBc.
    fn milky_way() -> GalaxyStarInput {
        let r = 19_329.9;
        GalaxyStarInput { dims: DVec3::splat(r), radius_pc: r, hubble: 15, has_model: true, seed: -8_628_293, galaxy: GalaxyRef::Catalog(8514) }
    }

    /// A voxel map whose red channel is `v` everywhere inside `|z| < 0.1` of the box.
    fn slab(v: f32) -> StarDensity {
        let (nx, nz) = (16, 20);
        let mut data = vec![0.0f32; nx * nx * nz * 3];
        for x in 0..nx {
            for y in 0..nx {
                for z in 8..12 {
                    data[((x * nx + y) * nz + z) * 3] = v;
                }
            }
        }
        StarDensity::Voxel(Arc::new(DensityMap { nx, ny: nx, nz, data }))
    }

    #[test]
    fn milky_way_layout_follows_se() {
        let t = StarOctree::new(&milky_way(), 7.0);
        assert_eq!((t.size_class, t.depth, t.levels), (0, 4, 9));
        // 0.3 x (2416.2 pc)^3 / 2000^3 x 16384 tries, split (1 + 0.25 + 1 + 0.1).
        assert!((8600..8700).contains(&t.total), "{}", t.total);
        assert_eq!(t.leaf_total, (t.total as f32 * LEAF_BOOST) as u32);
        // Bands: N(0, 6) slices from Mv -8, the last ending at the 0.9999 quantile.
        assert!((t.mag_band(0).0 + 8.0).abs() < 1e-3);
        assert!((t.mag_band(8).1 - 22.31).abs() < 0.02, "{:?}", t.mag_band(8));
        assert!((t.mag_band(4).0 + 0.075).abs() < 0.05, "{:?}", t.mag_band(4));
        for l in 1..t.levels {
            assert_eq!(t.mag_band(l).0, t.mag_band(l - 1).1);
        }
        // Dwarfs and small galaxies: fewer levels, finer roots.
        assert_eq!(size_class(881.0), 2);
        assert_eq!(size_class(60_000.0), 0);
    }

    #[test]
    fn se_root_and_node_numbers_round_trip() {
        let t = StarOctree::new(&milky_way(), 7.0);
        // Root 491 is the octant path 0, 7, 5, 3: (3/16, 5/16, 1/16) of the box from the centre.
        let k = t.key_of(491, 0, 0).unwrap();
        let c = t.node_center(k) / t.dims;
        assert!((c - DVec3::new(3.0 / 16.0, 5.0 / 16.0, 1.0 / 16.0)).length() < 1e-9, "{c}");
        let k = t.key_of(491, 6, 169_551).unwrap();
        assert_eq!(t.root_node(k), (491, 169_551));
        assert_eq!(t.star_name(k, 68), "RS 8514-491-6-169551-68");
        assert_eq!(t.parse_name("rs 8514-491-6-169551-68"), Some((k, 68)));
        assert_eq!(t.parse_name("RS 8514-491-9-0-0"), None);
        assert_eq!(t.parse_name("RS 8513-491-6-169551-68"), None);
        // A child is the parent's cell refined, numbered parent * 8 + octant.
        let parent = t.key_of(491, 5, 169_551 / 8).unwrap();
        assert_eq!(t.cell_of(5, t.node_center(k)), parent.cell);
    }

    #[test]
    fn nodes_fill_with_se_stars() {
        let t = StarOctree::new(&milky_way(), 7.0);
        let d = slab(0.3);
        let sun = DVec3::new(8_500.0, 0.0, 0.0);
        let all = |_: DVec3| true;
        let key = NodeKey { level: 6, cell: t.cell_of(6, sun) };
        let a = t.generate_where(key, &d, None, &all);
        let b = t.generate_where(key, &d, None, &all);
        // About 0.045 of the 8666 tries.
        assert!((250..550).contains(&a.len()), "{}", a.len());
        assert!(a.iter().zip(&b).all(|(x, y)| x.pos == y.pos && x.class == y.class));
        let (lo, hi) = t.mag_band(6);
        assert!(a.iter().all(|s| s.abs_mag as f64 >= lo - 1e-3 && s.abs_mag as f64 <= hi + 1e-3 || !s.kind.eq(&StarKind::Normal)));
        assert!(a.iter().enumerate().all(|(i, s)| s.index == i as u32));
        let h = t.node_half(6);
        assert!(a.iter().all(|s| ((s.pos - t.node_center(key)).abs() - h).max_element() <= 1e-3));
        // The deepest level: 45 % planemos, a few lone remnants.
        let leaf = NodeKey { level: 8, cell: t.cell_of(8, sun) };
        let l = t.generate_where(leaf, &d, None, &all);
        let pm = l.iter().filter(|s| s.kind == StarKind::Planemo).count() as f64 / l.len() as f64;
        assert!((pm - 0.45).abs() < 0.08, "{pm} of {}", l.len());
        // Below 64 kept stars past level 2 a node is empty.
        assert!(t.generate_where(key, &slab(0.02), None, &all).is_empty());
        // Positions kept by `keep` are the same stars.
        let near = |p: DVec3| p.x < t.node_center(key).x;
        let k = t.generate_where(key, &d, None, &near);
        assert!(k.iter().all(|s| a.iter().any(|x| x.index == s.index && x.pos == s.pos)));
    }

    #[test]
    fn analytic_galaxies_have_old_halos() {
        let e = GalaxyStarInput { dims: DVec3::new(8_000.0, 10_000.0, 8_000.0), radius_pc: 4_000.0, hubble: 3, has_model: false, seed: 77, galaxy: GalaxyRef::Catalog(5) };
        let t = StarOctree::new(&e, 7.0);
        assert_eq!(t.counts[..3], [0, 0, 0]);
        let all = |_: DVec3| true;
        let core = t.generate_where(NodeKey { level: 3, cell: t.cell_of(3, DVec3::splat(100.0)) }, &StarDensity::Analytic, None, &all);
        assert!(!core.is_empty());
        assert!(core.iter().all(|s| s.feh < 0.5));
        let far = t.generate_where(NodeKey { level: 3, cell: t.cell_of(3, DVec3::new(7_000.0, 0.0, 0.0)) }, &StarDensity::Analytic, None, &all);
        assert!(far.len() < core.len());
    }

    #[test]
    fn home_cut_leaves_bright_and_near_stars_to_the_catalogs() {
        let t = StarOctree::new(&milky_way(), 7.0);
        let sun = DVec3::new(8_500.0, 0.0, 0.0);
        let cut = HomeCut { sun, max_app_mag: 10.0, min_dist_normal: 2.5, min_dist_brown: 2.0, min_dist_remnant: 3.0 };
        let all = |_: DVec3| true;
        let key = NodeKey { level: 4, cell: t.cell_of(4, sun) };
        let stars = t.generate_where(key, &slab(0.6), Some(&cut), &all);
        for s in &stars {
            let d = (s.pos - sun).length();
            assert!(d > 2.0 && (!s.luminous() || s.abs_mag as f64 + 5.0 * (d / 10.0).log10() >= 10.0));
        }
    }

    #[test]
    fn wanted_nodes_reach_by_band() {
        let t = StarOctree::new(&milky_way(), 7.0);
        let cam = DVec3::new(8_500.0, 0.0, 0.0);
        let w = t.wanted_nodes(cam, 1.0, None, 0);
        assert_eq!(w.len(), 9);
        for (l, keys) in w.iter().enumerate() {
            assert!(!keys.is_empty(), "level {l}");
            assert!(keys.iter().all(|k| t.box_distance(*k, cam) <= t.reach(l as u8)));
        }
        // The camera's own node at every level.
        assert!(w[8].contains(&NodeKey { level: 8, cell: t.cell_of(8, cam) }));
        let zoomed = t.wanted_nodes(cam, 10.0, Some((DVec3::X, 2f64.to_radians())), 3);
        assert!(zoomed.iter().zip(&w).all(|(z, b)| z.len() <= b.len() + 3));
    }
}
