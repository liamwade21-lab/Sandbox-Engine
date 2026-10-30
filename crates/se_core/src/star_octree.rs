//! Procedural stars of a galaxy, organised as a magnitude-stratified octree.
//!
//! The galaxy's bounding cube is subdivided into an octree. Each level `L` owns a
//! band of absolute magnitudes chosen so that its stars are visible (above the
//! magnitude limit) from up to about `VIEW_FACTOR` node sizes away: bright, rare
//! stars live in big shallow nodes; faint, common stars in small deep ones. To
//! draw the sky from any point, load for every level the few nodes around the
//! camera. Each node's stars are generated deterministically from its address.
//!
//! Positions are in the galaxy's model frame (parsecs, disk normal = -Z, as for
//! catalog galaxies), relative to the galaxy centre.

use std::sync::{Arc, OnceLock};

use glam::{DQuat, DVec3};

use crate::galaxy_model::DensityMap;
use crate::star::{SpectralClass, StarKind, blackbody_rgb, main_sequence_abs_mag};

/// Stars are kept visible this many node sizes from the camera.
pub const VIEW_FACTOR: f64 = 1.5;
/// Faint limit of the procedural population (absolute magnitude).
pub const FAINTEST_MAG: f64 = 16.0;

/// Solar-neighbourhood luminosity function: stars per pc^3 per magnitude
/// (Wielen-like, all luminosity classes). Integrates to ~0.1 stars/pc^3.
const LF: [(f64, f64); 15] = [
    (-10.0, 1.0e-10),
    (-8.0, 1.0e-9),
    (-6.0, 2.0e-8),
    (-4.0, 3.0e-7),
    (-2.0, 4.0e-6),
    (0.0, 6.0e-5),
    (2.0, 4.0e-4),
    (4.0, 2.0e-3),
    (6.0, 3.5e-3),
    (8.0, 5.0e-3),
    (10.0, 1.0e-2),
    (12.0, 1.5e-2),
    (13.0, 1.6e-2),
    (14.0, 1.4e-2),
    (16.0, 8.0e-3),
];

/// No star is brighter than this (the most luminous hypergiants reach about -10). Level 0 of a
/// big galaxy spans magnitudes -30 to about -10; a flat tail there filled it with stars as
/// bright as whole galaxies, and their radii (tenths of a light year) broke the sphere mesh.
const BRIGHTEST_MAG: f64 = -10.5;

fn lf(m: f64) -> f64 {
    if m < BRIGHTEST_MAG {
        return 0.0;
    }
    if m <= LF[0].0 {
        // Continue the -10..-8 slope.
        let (m0, p0) = LF[0];
        let (m1, p1) = LF[1];
        return (p0.ln() + (p1.ln() - p0.ln()) * (m - m0) / (m1 - m0)).exp();
    }
    for w in LF.windows(2) {
        let (m0, p0) = w[0];
        let (m1, p1) = w[1];
        if m <= m1 {
            let t = (m - m0) / (m1 - m0);
            return (p0.ln() + (p1.ln() - p0.ln()) * t).exp();
        }
    }
    LF[LF.len() - 1].1
}

/// Integral of the luminosity function over [m0, m1] (stars per pc^3).
pub fn lf_integral(m0: f64, m1: f64) -> f64 {
    if m1 <= m0 {
        return 0.0;
    }
    let n = ((m1 - m0) / 0.05).ceil().max(1.0) as usize;
    let h = (m1 - m0) / n as f64;
    (0..n).map(|i| lf(m0 + h * (i as f64 + 0.5)) * h).sum()
}

/// Brightest absolute magnitude of the old (bulge / halo) stellar population: the tip of the red giant branch.
const OLD_POPULATION_MAG_LIMIT: f64 = -2.0;
/// Dead stars per pc^3 at density 1 (the solar neighbourhood): about 15 black holes and 40 neutron
/// stars within 100 light years.
const BLACK_HOLES_PER_PC3: f64 = 1.2e-4;
const NEUTRON_STARS_PER_PC3: f64 = 3.0e-4;

/// Stellar number density model, relative to the solar neighbourhood (= 1).
pub trait DensityModel: Send + Sync {
    /// `p` in parsecs, galaxy model frame.
    fn density(&self, p: DVec3) -> f64;

    /// Fraction of the stars at `p` that belong to a young population (disk). Luminous supergiants and OB
    /// stars only exist there; the old bulge/halo population has none.
    fn young_fraction(&self, _p: DVec3) -> f64 {
        1.0
    }
}

/// Analytic Milky Way: thin + thick exponential disks, a bulge and a halo.
#[derive(Debug, Clone)]
pub struct MilkyWayDensity {
    norm: f64,
    pub radius_pc: f64,
}

impl MilkyWayDensity {
    pub fn new(radius_pc: f64, sun_model_pos: DVec3) -> Self {
        let mut m = Self { norm: 1.0, radius_pc };
        let at_sun = m.raw(sun_model_pos);
        m.norm = 1.0 / at_sun.max(1e-12);
        m
    }

    /// `(disk, bulge + halo)` unnormalised densities at `p`.
    fn parts(&self, p: DVec3) -> (f64, f64) {
        let r_cyl = (p.x * p.x + p.y * p.y).sqrt();
        let z = p.z.abs();
        let r = p.length();
        let thin = (-r_cyl / 2600.0).exp() * (-z / 300.0).exp();
        let thick = 0.12 * (-r_cyl / 3600.0).exp() * (-z / 900.0).exp();
        let bulge = 30.0 * (-(r / 700.0).powf(1.8)).exp();
        let halo = 0.0015 * (1.0 + r / 2000.0).powf(-3.0);
        // Truncate beyond the visible disk.
        let edge = 1.0 / (1.0 + ((r_cyl - self.radius_pc * 1.3) / 800.0).exp());
        ((thin + thick) * edge, bulge + halo)
    }

    fn raw(&self, p: DVec3) -> f64 {
        let (disk, spheroid) = self.parts(p);
        disk + spheroid
    }
}

impl DensityModel for MilkyWayDensity {
    fn density(&self, p: DVec3) -> f64 {
        self.raw(p) * self.norm
    }

    fn young_fraction(&self, p: DVec3) -> f64 {
        let (disk, spheroid) = self.parts(p);
        disk / (disk + spheroid).max(1e-12)
    }
}

/// Radius of the Milky Way in SE's catalog (`galaxies/LocalGroup.sc`): the reference
/// [`GalaxyStarDensity`] scales other galaxies from.
pub const MILKY_WAY_RADIUS_PC: f64 = 19_329.9;
/// The Sun's distance from the Milky Way centre in the same catalog (pc).
const SUN_GALACTOCENTRIC_PC: f64 = 8_584.74;

/// Hubble-type family of a galaxy, which sets the shape of its interior star density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GalaxyForm {
    /// Disk galaxy (S0, spirals); `bulge` scales the central bulge relative to the Milky Way's.
    Disk { bulge: f64 },
    /// Elliptical with axis ratio `q` (E0 = 1 ... E7 = 0.3).
    Elliptical { q: f64 },
    Irregular,
}

impl GalaxyForm {
    /// From a catalog Hubble type (`"E3"`, `"S0"`, `"SBb"`, `"Irr"`...).
    pub fn from_type(t: &str) -> Self {
        let b = t.trim().as_bytes();
        match b.first() {
            Some(b'E') => {
                let n = b.get(1).filter(|c| c.is_ascii_digit()).map(|c| (c - b'0') as f64).unwrap_or(3.0);
                Self::Elliptical { q: 1.0 - n.min(7.0) / 10.0 }
            }
            Some(b'I') => Self::Irregular,
            Some(b'S') => {
                let sub = if b.get(1) == Some(&b'B') { b.get(2) } else { b.get(1) };
                let bulge = match sub {
                    Some(b'0') => 2.0,
                    Some(b'a') => 1.6,
                    Some(b'b') => 1.0,
                    Some(b'c') => 0.6,
                    _ => 0.3,
                };
                Self::Disk { bulge }
            }
            _ => Self::Disk { bulge: 1.0 },
        }
    }
}

/// Spiral-arm contrast from a galaxy model's emission voxel map (the map SpaceEngine's
/// galaxy-interior star generator samples): the mid-plane brightness relative to its mean around
/// the same radius, so stars crowd into the arms the model draws while every ring keeps its count.
#[derive(Debug, Clone)]
pub struct ArmMap {
    map: Arc<DensityMap>,
    /// Parsecs per model unit along x, y, z (the map spans -1..1 in x and y).
    scale_pc: DVec3,
    /// Mean mid-plane brightness in `RINGS` rings out to model radius 1.
    ring_mean: Vec<f32>,
}

const RINGS: usize = 64;

impl ArmMap {
    pub fn new(map: Arc<DensityMap>, scale_pc: DVec3) -> Self {
        let mut ring_mean = vec![0.0f32; RINGS];
        for (i, m) in ring_mean.iter_mut().enumerate() {
            let r = (i as f32 + 0.5) / RINGS as f32;
            let n = 96;
            let sum: f32 = (0..n)
                .map(|k| {
                    let a = k as f32 / n as f32 * std::f32::consts::TAU;
                    map.sample(r * a.cos(), r * a.sin(), 0.0)
                })
                .sum();
            *m = sum / n as f32;
        }
        Self { map, scale_pc, ring_mean }
    }

    /// Density factor at `p` (pc, model frame): 1 on average around every ring, `0.3` in empty
    /// gaps between arms, and 1 outside the map.
    pub fn factor(&self, p: DVec3) -> f64 {
        let (x, y) = ((p.x / self.scale_pc.x) as f32, (p.y / self.scale_pc.y) as f32);
        let r = (x * x + y * y).sqrt();
        let i = (r * RINGS as f32) as usize;
        let Some(&mean) = self.ring_mean.get(i) else { return 1.0 };
        if mean <= 1e-6 {
            return 1.0;
        }
        let v = self.map.sample(x, y, 0.0);
        (0.3 + 0.7 * (v / mean)).min(4.0) as f64
    }
}

/// Stellar density of any galaxy, for the stars generated inside it: the Milky Way's model
/// stretched to the galaxy's radius and reshaped for its Hubble type, optionally with the arms of
/// its sprite model. Like SpaceEngine it ignores the galaxy's luminosity: every galaxy is as full
/// as a Milky Way twin of its size and type (the Milky Way, normalised to 1 at the Sun, being the
/// reference). Scaling by light/size^3 instead made small bright dwarfs walls of stars and faint
/// ellipticals nearly empty, the reverse of SE (compared inside the LMC, a dwarf irregular and an
/// E7 dwarf, 2026-10-27). Units as [`DensityModel`].
#[derive(Debug, Clone)]
pub struct GalaxyStarDensity {
    form: GalaxyForm,
    /// Radius in Milky Way radii.
    s: f64,
    /// Vertical stretch: disks thicken relative to their size as galaxies get smaller.
    vz: f64,
    arms: Option<ArmMap>,
    norm: f64,
}

/// Share of luminous (young) stars in ellipticals. Real ones have none, but SE fills its ellipticals with
/// the same mix as everything else (blue and red supergiants in the clouds around E galaxies, 2026-10-27);
/// without them nothing in an elliptical is visible beyond a few hundred parsecs.
const ELLIPTICAL_YOUNG: f64 = 0.3;

/// Fraction of the Milky Way's light in its disk.
const MILKY_WAY_DISK_LIGHT: f64 = 0.8;

/// Stars of a Milky Way twin, in pc^3 of solar-neighbourhood density: the
/// integral of the Milky Way model's disk (which matches the solar neighbourhood) over its share
/// of the light. Its bulge is left out: it holds 60% of the model's stars, far more than its
/// share of the light, and would crowd the disks of other galaxies.
fn stars_per_milky_way() -> f64 {
    static V: OnceLock<f64> = OnceLock::new();
    *V.get_or_init(|| {
        let mw = MilkyWayDensity::new(MILKY_WAY_RADIUS_PC, DVec3::new(SUN_GALACTOCENTRIC_PC, 0.0, 0.0));
        let disk = integrate_axisymmetric(|p| mw.density(p) * mw.young_fraction(p), MILKY_WAY_RADIUS_PC * 1.5);
        disk / MILKY_WAY_DISK_LIGHT
    })
}

/// Volume integral of an axisymmetric (about z) density over `R, |z| < r_max`: log-spaced rings and
/// layers finest at the plane, so thin disks and steep cusps both resolve.
fn integrate_axisymmetric(f: impl Fn(DVec3) -> f64, r_max: f64) -> f64 {
    let (nr, nz) = (160, 96);
    let r_min = r_max * 1e-5;
    let lr = (r_max / r_min).ln();
    let z0 = (r_max * 1e-4).max(0.5);
    let tz = (1.0 + r_max / z0).ln();
    let mut sum = 0.0;
    for i in 0..nr {
        let a = r_min * (lr * i as f64 / nr as f64).exp();
        let b = r_min * (lr * (i + 1) as f64 / nr as f64).exp();
        let (r, area) = (0.5 * (a + b), std::f64::consts::PI * (b * b - a * a));
        for j in 0..nz {
            let za = z0 * ((tz * j as f64 / nz as f64).exp() - 1.0);
            let zb = z0 * ((tz * (j + 1) as f64 / nz as f64).exp() - 1.0);
            sum += 2.0 * f(DVec3::new(r, 0.0, 0.5 * (za + zb))) * area * (zb - za);
        }
    }
    sum
}

impl GalaxyStarDensity {
    pub fn new(form: GalaxyForm, radius_pc: f64, arms: Option<ArmMap>) -> Self {
        let s = (radius_pc / MILKY_WAY_RADIUS_PC).max(1e-3);
        let mut d = Self { form, s, vz: s.sqrt(), arms, norm: 1.0 };
        let shape = integrate_axisymmetric(
            |p| {
                let (disk, spheroid) = d.parts(p);
                disk + spheroid
            },
            radius_pc * 1.5,
        );
        // The twin's stars, times the volume of the stretch.
        d.norm = stars_per_milky_way() * d.s * d.s * d.vz / shape.max(1e-12);
        d
    }

    /// `(disk, spheroid)` unnormalised densities without arms, in the Milky Way's units after
    /// stretching `p` to its size.
    fn parts(&self, p: DVec3) -> (f64, f64) {
        let rc = (p.x * p.x + p.y * p.y).sqrt() / self.s;
        let z = p.z.abs() / self.vz;
        let r = (rc * rc + z * z).sqrt();
        let edge = |x: f64, w: f64| 1.0 / (1.0 + ((x - MILKY_WAY_RADIUS_PC * 1.3) / w).exp());
        match self.form {
            GalaxyForm::Disk { bulge } => {
                let thin = (-rc / 2600.0).exp() * (-z / 300.0).exp();
                let thick = 0.12 * (-rc / 3600.0).exp() * (-z / 900.0).exp();
                let bulge = 30.0 * bulge * (-(r / 700.0).powf(1.8)).exp();
                let halo = 0.0015 * (1.0 + r / 2000.0).powf(-3.0);
                ((thin + thick) * edge(rc, 800.0), bulge + halo)
            }
            GalaxyForm::Elliptical { q } => {
                // Cored Hernquist profile (scale 12% of the radius), flattened to `q`.
                let zq = p.z.abs() / self.s / q;
                let m = (rc * rc + zq * zq).sqrt() / (0.12 * MILKY_WAY_RADIUS_PC);
                (0.0, edge(m * 0.12 * MILKY_WAY_RADIUS_PC, 1000.0) / ((m + 0.02) * (1.0 + m).powi(3)))
            }
            GalaxyForm::Irregular => {
                // A thick exponential ellipsoid (axis ratio 0.5), young throughout.
                let m = (rc * rc + 4.0 * (p.z / self.s).powi(2)).sqrt();
                ((-m / (0.3 * MILKY_WAY_RADIUS_PC)).exp() * edge(m, 800.0), 0.0)
            }
        }
    }

    fn arm(&self, p: DVec3) -> f64 {
        self.arms.as_ref().map_or(1.0, |a| a.factor(p))
    }
}

impl DensityModel for GalaxyStarDensity {
    fn density(&self, p: DVec3) -> f64 {
        let (disk, spheroid) = self.parts(p);
        (disk * self.arm(p) + spheroid) * self.norm
    }

    fn young_fraction(&self, p: DVec3) -> f64 {
        if let GalaxyForm::Elliptical { .. } = self.form {
            return ELLIPTICAL_YOUNG;
        }
        let (disk, spheroid) = self.parts(p);
        let disk = disk * self.arm(p);
        disk / (disk + spheroid).max(1e-300)
    }
}

/// Octree parameters for one galaxy.
#[derive(Debug, Clone)]
pub struct StarOctree {
    /// Galaxy id used in seeds and names.
    pub galaxy_id: u32,
    /// Side of the root cube, parsecs.
    pub root_size: f64,
    /// Apparent magnitude limit the strata are designed for.
    pub mag_limit: f64,
    /// Local stellar density at density 1 (stars / pc^3).
    pub n0: f64,
    pub levels: u8,
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

/// Node address: level and integer cell coordinates (cells of side `root/2^level`
/// with the root cube centred on the galaxy centre).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey {
    pub level: u8,
    pub cell: [i32; 3],
}

#[derive(Debug, Clone)]
pub struct ProcStar {
    /// Model-frame position (pc) relative to the galaxy centre.
    pub pos: DVec3,
    pub abs_mag: f32,
    pub teff: f32,
    pub rgb: [f32; 3],
    pub kind: StarKind,
    pub class: String,
    /// Index within its node (stable).
    pub index: u32,
}

impl StarOctree {
    pub fn new(galaxy_id: u32, galaxy_radius_pc: f64, mag_limit: f64) -> Self {
        let root_size = galaxy_radius_pc * 3.0;
        let mut t = Self {
            galaxy_id,
            root_size,
            mag_limit,
            n0: 0.1,
            levels: 0,
        };
        // Deepest level whose bright limit is still brighter than the faint end.
        let mut l = 0u8;
        while l < 30 && t.mag_band(l).0 < FAINTEST_MAG {
            l += 1;
        }
        t.levels = l;
        t
    }

    pub fn node_size(&self, level: u8) -> f64 {
        self.root_size / (1u64 << level) as f64
    }

    /// Faintest absolute magnitude visible at distance `d` pc (with fade margin).
    fn faint_mag_at(&self, d: f64) -> f64 {
        self.mag_limit + 1.5 - 5.0 * (d.max(1e-6) / 10.0).log10()
    }

    /// Absolute-magnitude band [bright, faint) stored at a level.
    pub fn mag_band(&self, level: u8) -> (f64, f64) {
        let faint = self.faint_mag_at(VIEW_FACTOR * self.node_size(level)).min(FAINTEST_MAG).max(BRIGHTEST_MAG);
        // Clamped to the brightest star: `sample_mag` falls back to a uniform draw over the band
        // when rejection keeps failing, which on level 0's old -30..-10 band gave stars of
        // Mv -20 and radii of hundreds of AU.
        let bright = if level == 0 {
            BRIGHTEST_MAG
        } else {
            self.faint_mag_at(VIEW_FACTOR * self.node_size(level - 1)).min(FAINTEST_MAG).max(BRIGHTEST_MAG)
        };
        (bright, faint)
    }

    pub fn node_center(&self, key: NodeKey) -> DVec3 {
        let s = self.node_size(key.level);
        let off = -self.root_size * 0.5;
        DVec3::new(
            off + (key.cell[0] as f64 + 0.5) * s,
            off + (key.cell[1] as f64 + 0.5) * s,
            off + (key.cell[2] as f64 + 0.5) * s,
        )
    }

    /// Cell containing model position `p` at `level`.
    pub fn cell_of(&self, level: u8, p: DVec3) -> [i32; 3] {
        let s = self.node_size(level);
        let q = (p + DVec3::splat(self.root_size * 0.5)) / s;
        [q.x.floor() as i32, q.y.floor() as i32, q.z.floor() as i32]
    }

    /// Nodes of `level` that should be loaded for a camera at model position `cam`.
    pub fn nodes_around(&self, level: u8, cam: DVec3) -> Vec<NodeKey> {
        let s = self.node_size(level);
        let r = VIEW_FACTOR * s;
        let lo = self.cell_of(level, cam - DVec3::splat(r));
        let hi = self.cell_of(level, cam + DVec3::splat(r));
        let n = 1i64 << level;
        let mut out = Vec::new();
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if x < 0 || y < 0 || z < 0 || x as i64 >= n || y as i64 >= n || z as i64 >= n {
                        continue;
                    }
                    let key = NodeKey { level, cell: [x, y, z] };
                    // Sphere/box test.
                    let c = self.node_center(key);
                    let d = (cam - c).abs() - DVec3::splat(s * 0.5);
                    if d.max(DVec3::ZERO).length() <= r {
                        out.push(key);
                    }
                }
            }
        }
        out
    }

    /// Nodes to load for a camera at model position `cam`, by level (coarse first, nearest first
    /// within a level): those of [`nodes_around`](Self::nodes_around), and with `reach` > 1 (a
    /// magnitude limit `5 log10(reach)` above the one the strata are designed for) also those
    /// out to `reach` times as far whose bounding sphere meets `cone` (unit axis, half-angle in
    /// radians), or every one that far without a cone. SE keeps a node of stars while its
    /// brightest star could still show (FUN_1404a0e80) and clips them to the view frustum when
    /// the field of view is under `StarFrustumClipFOV` (FUN_14047b670). At most `max_far` nodes
    /// per level beyond the usual reach are kept, the nearest.
    pub fn wanted_nodes(&self, cam: DVec3, reach: f64, cone: Option<(DVec3, f64)>, max_far: usize) -> Vec<Vec<NodeKey>> {
        let reach = reach.max(1.0);
        let in_cone = |c: DVec3, r: f64| cone.is_none_or(|k| sphere_meets_cone(cam, k, c, r));
        let mut out: Vec<Vec<(f64, NodeKey)>> = vec![Vec::new(); self.levels as usize];
        let mut stack = vec![NodeKey { level: 0, cell: [0, 0, 0] }];
        while let Some(key) = stack.pop() {
            let s = self.node_size(key.level);
            let c = self.node_center(key);
            let d = ((cam - c).abs() - DVec3::splat(s * 0.5)).max(DVec3::ZERO).length();
            let near = VIEW_FACTOR * s;
            let seen = d <= near * reach && in_cone(c, s * 0.5 * 3f64.sqrt());
            if d <= near || seen {
                out[key.level as usize].push((d, key));
            }
            // Children are half the size, so they reach half as far.
            if key.level + 1 < self.levels && (d <= near * 0.5 || (seen && d <= near * reach * 0.5)) {
                for i in 0..8 {
                    let cell = [0, 1, 2].map(|a| key.cell[a] * 2 + ((i >> a) & 1));
                    stack.push(NodeKey { level: key.level + 1, cell });
                }
            }
        }
        out.into_iter()
            .enumerate()
            .map(|(level, mut v)| {
                v.sort_by(|a, b| a.0.total_cmp(&b.0));
                let near = VIEW_FACTOR * self.node_size(level as u8);
                let n = v.partition_point(|(d, _)| *d <= near);
                v.truncate(n.saturating_add(max_far));
                v.into_iter().map(|(_, k)| k).collect()
            })
            .collect()
    }

    /// Stars per pc^3 a node of `level` holds at density 1 (the `expected` count of
    /// [`generate`](Self::generate) is this times the node's mean density times its volume).
    pub fn level_rate(&self, level: u8) -> f64 {
        let (m_bright, m_faint) = self.mag_band(level);
        if m_faint <= m_bright { 0.0 } else { self.n0 / 0.1 * lf_integral(m_bright, m_faint) }
    }

    /// True when a node whose expected star count is at most `max_expected` surely holds no
    /// stars: its first random draw alone already gives a Poisson count of 0. Lets a search skip
    /// the millions of tiny, nearly always empty nodes of the deep levels without sampling the
    /// density in each.
    pub fn surely_empty(&self, key: NodeKey, max_expected: f64) -> bool {
        key.level != self.remnant_level() && max_expected < 30.0 && Rng(self.seed(key)).f64() <= (-max_expected).exp()
    }

    pub fn seed(&self, key: NodeKey) -> u64 {
        let mut h = 0x9E37_79B9_7F4A_7C15u64 ^ self.galaxy_id as u64;
        for v in [key.level as i64, key.cell[0] as i64, key.cell[1] as i64, key.cell[2] as i64] {
            h = splitmix(h ^ v as u64);
        }
        h
    }

    /// Generate the stars of one node. `exclude` returns true for stars that
    /// must be skipped (e.g. ones already present in a real catalog).
    pub fn generate(
        &self,
        key: NodeKey,
        density: &dyn DensityModel,
        exclude: &dyn Fn(DVec3, f64) -> bool,
    ) -> Vec<ProcStar> {
        self.generate_where(key, density, exclude, &|_| true)
    }

    /// [`generate`](Self::generate), keeping only the stars at positions that pass `keep`; the
    /// others still take their random draws, so the kept ones are the same stars, but skip the
    /// costly type and colour (a search over a small region of a big node).
    pub fn generate_where(
        &self,
        key: NodeKey,
        density: &dyn DensityModel,
        exclude: &dyn Fn(DVec3, f64) -> bool,
        keep: &dyn Fn(DVec3) -> bool,
    ) -> Vec<ProcStar> {
        let s = self.node_size(key.level);
        let c = self.node_center(key);
        let (m_bright, m_faint) = self.mag_band(key.level);
        if m_faint <= m_bright {
            return Vec::new();
        }
        // Density samples for rejection sampling.
        let mut rho_max: f64 = 0.0;
        let mut rho_sum = 0.0;
        for dx in [-0.5, 0.0, 0.5] {
            for dy in [-0.5, 0.0, 0.5] {
                for dz in [-0.5, 0.0, 0.5] {
                    let r = density.density(c + DVec3::new(dx, dy, dz) * s);
                    rho_max = rho_max.max(r);
                    rho_sum += r;
                }
            }
        }
        let mut rho_mean = rho_sum / 27.0;
        // Where the density varies steeply across the node (a big node around a galaxy's centre),
        // the centre sample can sit on the peak and inflate the count: average a finer grid of
        // cell midpoints instead.
        if rho_max > 0.0 && rho_max > 8.0 * rho_mean {
            let n = 6;
            let mut sum = 0.0;
            for i in 0..n {
                for j in 0..n {
                    for k in 0..n {
                        let f = |a: i32| (a as f64 + 0.5) / n as f64 - 0.5;
                        sum += density.density(c + DVec3::new(f(i), f(j), f(k)) * s);
                    }
                }
            }
            rho_mean = sum / (n * n * n) as f64;
        }
        let per_pc3 = self.n0 / 0.1 * lf_integral(m_bright, m_faint);
        let expected = rho_mean * per_pc3 * s * s * s;
        if expected < 1e-6 {
            return Vec::new();
        }
        let mut rng = Rng(self.seed(key));
        let pmax = lf_max(m_bright, m_faint);
        let n = rng.poisson(expected).min(20_000);
        let mut out = Vec::with_capacity(n as usize);
        let mut attempts = 0;
        let mut index = 0u32;
        while (out.len() as u64) < n && attempts < n * 20 + 64 {
            attempts += 1;
            let p = c + DVec3::new(rng.f64() - 0.5, rng.f64() - 0.5, rng.f64() - 0.5) * s;
            if rho_max > 0.0 && rng.f64() * rho_max > density.density(p) {
                continue;
            }
            let m = sample_mag(&mut rng, m_bright, m_faint, pmax);
            // Supergiants and OB stars (brighter than the red-giant tip) belong to the young disk only; without
            // this the dense bulge produced a few saturated white "stars" towards the galactic centre.
            if m < OLD_POPULATION_MAG_LIMIT && rng.f64() > density.young_fraction(p) {
                continue;
            }
            index += 1;
            if exclude(p, m) {
                continue;
            }
            let draw = draw_star(&mut rng, m);
            if !keep(p) {
                continue;
            }
            let (kind, teff, class) = star_for_mag(m, &draw);
            out.push(ProcStar {
                pos: p,
                abs_mag: m as f32,
                teff: teff as f32,
                rgb: blackbody_rgb(teff),
                kind,
                class,
                index,
            });
        }
        // Dead stars (SE's fields have neutron stars and black holes): a sparse population of their
        // own on the level of ~50 pc nodes, with its own random stream. They give no light, so no
        // visibility band places them, and a search for them visits only that level.
        if key.level == self.remnant_level() {
            let mut rr = Rng(self.seed(key) ^ 0xDEAD_57A2_0000_0001);
            let k = rr.poisson(rho_mean * (BLACK_HOLES_PER_PC3 + NEUTRON_STARS_PER_PC3) * s * s * s).min(2_000);
            for _ in 0..k {
                let p = c + DVec3::new(rr.f64() - 0.5, rr.f64() - 0.5, rr.f64() - 0.5) * s;
                let hole = rr.f64() * (BLACK_HOLES_PER_PC3 + NEUTRON_STARS_PER_PC3) < BLACK_HOLES_PER_PC3;
                index += 1;
                if !keep(p) {
                    continue;
                }
                let (kind, abs_mag, teff, rgb, class) = if hole {
                    (StarKind::BlackHole, 30.0, 3000.0, [0.0; 3], "X")
                } else {
                    (StarKind::NeutronStar, 25.0, 600_000.0, [0.75, 0.85, 1.0], "Q")
                };
                out.push(ProcStar { pos: p, abs_mag, teff, rgb, kind, class: class.to_string(), index });
            }
        }
        out
    }

    /// The level holding the dead stars: nodes of about 50 pc (at least 40).
    pub fn remnant_level(&self) -> u8 {
        let mut l = 0;
        while l + 1 < self.levels && self.node_size(l + 1) >= 40.0 {
            l += 1;
        }
        l
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

/// Peak of the luminosity function over a band (sampled), for [`sample_mag`].
fn lf_max(m0: f64, m1: f64) -> f64 {
    let n = 16;
    (0..=n).map(|i| lf(m0 + (m1 - m0) * i as f64 / n as f64)).fold(0.0, f64::max)
}

fn sample_mag(rng: &mut Rng, m0: f64, m1: f64, pmax: f64) -> f64 {
    // Rejection sample the luminosity function within the band.
    for _ in 0..64 {
        let m = m0 + (m1 - m0) * rng.f64();
        if rng.f64() * pmax <= lf(m) {
            return m;
        }
    }
    m0 + (m1 - m0) * rng.f64()
}

/// Pick a stellar type consistent with an absolute magnitude.
/// The random draws behind a star's type ([`star_for_mag`]): the type pick, then one value
/// (temperature or its jitter), and a giant's subclass.
struct StarDraw {
    u: f64,
    a: f64,
    b: f64,
}

fn draw_star(rng: &mut Rng, m: f64) -> StarDraw {
    let u = rng.f64();
    let a = rng.f64();
    let b = if m < 1.0 && u < giant_fraction(m) { rng.f64() } else { 0.0 };
    StarDraw { u, a, b }
}

fn star_for_mag(m: f64, d: &StarDraw) -> (StarKind, f64, String) {
    let u = d.u;
    // Luminous stars are a mix of hot dwarfs and cool giants/supergiants.
    if m < 1.0 && u < giant_fraction(m) {
        let t = 3500.0 + d.a * 1700.0;
        let lc = if m < -4.0 { "Ib" } else if m < -1.5 { "II" } else { "III" };
        let letter = if t < 3900.0 { "M" } else if t < 5200.0 { "K" } else { "G" };
        return (StarKind::Normal, t, format!("{letter}{} {lc}", (d.b * 9.0).round()));
    }
    // White dwarfs among faint stars.
    if m > 10.0 && u < 0.08 {
        let t = 5000.0 + d.a.powi(2) * 25000.0;
        return (StarKind::WhiteDwarf, t, format!("DA{:.0}", (50400.0 / t).clamp(1.0, 9.0)));
    }
    // Main sequence: invert the Mv(T) relation by bisection.
    let (mut lo, mut hi) = (2300.0f64, 50000.0f64);
    for _ in 0..40 {
        let mid = (lo * hi).sqrt();
        if main_sequence_abs_mag(mid) > m {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t = (lo * hi).sqrt() * (0.97 + 0.06 * d.a);
    (StarKind::Normal, t, format!("{}V", ms_class_name(t)))
}

fn giant_fraction(m: f64) -> f64 {
    if m < -3.0 { 0.6 } else if m < 0.0 { 0.7 } else { 0.5 }
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

/// Small deterministic RNG (SplitMix64).
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
        // Normal approximation for large lambda.
        let u1 = self.f64().max(1e-300);
        let u2 = self.f64();
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        (lambda + z * lambda.sqrt()).round().max(0.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminosity_function_total() {
        let total = lf_integral(-10.0, 16.0);
        assert!(total > 0.07 && total < 0.15, "{total}");
    }

    #[test]
    fn bands_are_contiguous() {
        let t = StarOctree::new(0, 19_000.0, 7.5);
        assert!(t.levels > 10 && t.levels < 24, "{}", t.levels);
        for l in 1..t.levels {
            assert!((t.mag_band(l).0 - t.mag_band(l - 1).1).abs() < 1e-9);
        }
    }

    #[test]
    fn deterministic_and_dense_near_sun() {
        let sun = DVec3::new(8200.0, 0.0, 20.0);
        let d = MilkyWayDensity::new(19_000.0, sun);
        assert!((d.density(sun) - 1.0).abs() < 1e-9);
        assert!(d.density(DVec3::ZERO) > 20.0);
        assert!(d.density(DVec3::new(8200.0, 0.0, 3000.0)) < 0.05);
        let t = StarOctree::new(0, 19_000.0, 7.5);
        let lvl = t.levels - 1;
        let keys = t.nodes_around(lvl, sun);
        assert!(!keys.is_empty());
        let none = |_: DVec3, _: f64| false;
        let a = t.generate(keys[0], &d, &none);
        let b = t.generate(keys[0], &d, &none);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| x.pos == y.pos && x.abs_mag == y.abs_mag));
        // Visible star count around the Sun across all levels: thousands, not millions.
        let mut visible = 0usize;
        for l in 0..t.levels {
            for k in t.nodes_around(l, sun) {
                for s in t.generate(k, &d, &none) {
                    let dist = (s.pos - sun).length().max(0.01);
                    if s.abs_mag as f64 + 5.0 * (dist / 10.0).log10() < 7.5 {
                        visible += 1;
                    }
                }
            }
        }
        assert!(visible > 2_000 && visible < 80_000, "{visible}");
    }

    #[test]
    fn galaxy_density_scales_from_the_milky_way() {
        let sun = DVec3::new(SUN_GALACTOCENTRIC_PC, 0.0, 0.0);
        // A bulgeless Milky Way twin puts all of its light in the disk: the solar neighbourhood
        // density times 1 / (the Milky Way's disk share of the light).
        let twin = GalaxyStarDensity::new(GalaxyForm::Disk { bulge: 0.0 }, MILKY_WAY_RADIUS_PC, None);
        let d = twin.density(sun);
        assert!((d * MILKY_WAY_DISK_LIGHT - 1.0).abs() < 0.1, "{d}");
        // A galaxy a quarter the size is as dense at the same relative place.
        let small = GalaxyStarDensity::new(GalaxyForm::Disk { bulge: 0.0 }, MILKY_WAY_RADIUS_PC / 4.0, None);
        assert!((small.density(DVec3::new(SUN_GALACTOCENTRIC_PC / 4.0, 0.0, 0.0)) / d - 1.0).abs() < 0.05);
        // Ellipticals have SE's small share of luminous stars and are densest at the centre.
        let e = GalaxyStarDensity::new(GalaxyForm::from_type("E3"), 5_000.0, None);
        assert_eq!(e.young_fraction(DVec3::new(500.0, 0.0, 0.0)), ELLIPTICAL_YOUNG);
        assert!(e.density(DVec3::new(100.0, 0.0, 0.0)) > e.density(DVec3::new(2_000.0, 0.0, 0.0)));
        // A dwarf irregular still has a sensible density (stars per solar-neighbourhood volume).
        let irr = GalaxyStarDensity::new(GalaxyForm::from_type("Irr"), 2_000.0, None);
        let c = irr.density(DVec3::new(200.0, 0.0, 0.0));
        assert!(c > 0.01 && c < 100.0, "{c}");
        assert_eq!(GalaxyForm::from_type("SBa"), GalaxyForm::Disk { bulge: 1.6 });
    }

    #[test]
    fn no_star_is_brighter_than_a_hypergiant() {
        // A giant spiral: its top levels span magnitudes far brighter than any star.
        let d = GalaxyStarDensity::new(GalaxyForm::Disk { bulge: 1.0 }, 40_000.0, None);
        let t = StarOctree::new(7, 40_000.0, 7.5);
        let none = |_: DVec3, _: f64| false;
        let cam = DVec3::new(15_000.0, 0.0, 0.0);
        let mut n = 0;
        for l in 0..4 {
            for k in t.nodes_around(l, cam) {
                for s in t.generate(k, &d, &none) {
                    assert!(s.abs_mag as f64 >= BRIGHTEST_MAG, "level {l}: Mv {}", s.abs_mag);
                    n += 1;
                }
            }
        }
        assert!(n > 0);
    }

    #[test]
    fn wanted_nodes_reach_further_inside_the_cone() {
        let t = StarOctree::new(0, 19_000.0, 7.5);
        let cam = DVec3::new(8200.0, 0.0, 20.0);
        // Without a raised limit: exactly the nodes around the camera.
        let base = t.wanted_nodes(cam, 1.0, None, 0);
        for l in 0..t.levels {
            let mut a = base[l as usize].clone();
            let mut b = t.nodes_around(l, cam);
            a.sort();
            b.sort();
            assert_eq!(a, b, "level {l}");
        }
        // Raised by 5 magnitudes (10x the reach) inside a narrow cone: the base set plus nodes
        // far along the axis only.
        let axis = DVec3::X;
        let zoomed = t.wanted_nodes(cam, 10.0, Some((axis, 2f64.to_radians())), usize::MAX);
        let capped = t.wanted_nodes(cam, 10.0, Some((axis, 2f64.to_radians())), 3);
        assert!(capped.iter().zip(&base).all(|(c, b)| c.len() <= b.len() + 3));
        let deep = t.levels - 1;
        let s = t.node_size(deep);
        let far: Vec<_> = zoomed[deep as usize].iter().filter(|k| !base[deep as usize].contains(k)).collect();
        assert!(!far.is_empty());
        for k in &far {
            let v = t.node_center(**k) - cam;
            assert!(v.x > -s && v.length() <= VIEW_FACTOR * s * 10.0 + s, "{v}");
            assert!((v.y * v.y + v.z * v.z).sqrt() < v.x.max(0.0) * 0.05 + s, "{v}");
        }
        // Nearest first within a level.
        let d = |k: &NodeKey| (t.node_center(*k) - cam).length();
        assert!(d(&zoomed[deep as usize][0]) <= d(zoomed[deep as usize].last().unwrap()));
    }
}

#[cfg(test)]
mod remnant_tests {
    use super::*;

    struct Flat;
    impl DensityModel for Flat {
        fn density(&self, _p: DVec3) -> f64 {
            1.0
        }
    }

    #[test]
    fn dead_stars_live_on_one_level() {
        let t = StarOctree::new(0, 19_300.0, 7.5);
        let l = t.remnant_level();
        assert!((40.0..80.0).contains(&t.node_size(l)), "{}", t.node_size(l));
        let c = t.cell_of(l, DVec3::new(8_000.0, 0.0, 0.0));
        let stars = t.generate(NodeKey { level: l, cell: c }, &Flat, &|_, _| false);
        let holes = stars.iter().filter(|s| s.kind == StarKind::BlackHole).count();
        let pulsars = stars.iter().filter(|s| s.kind == StarKind::NeutronStar).count();
        assert!(holes > 3 && pulsars > holes, "{holes} {pulsars}");
        assert!(stars.iter().filter(|s| s.kind == StarKind::BlackHole).all(|s| s.class == "X" && s.rgb == [0.0; 3]));
        // Indices stay unique, so names stay unique.
        let mut ix: Vec<u32> = stars.iter().map(|s| s.index).collect();
        ix.sort();
        ix.dedup();
        assert_eq!(ix.len(), stars.len());
    }
}
