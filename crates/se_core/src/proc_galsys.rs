//! Procedural star clusters and nebulae inside a galaxy (`seGalSysModelCache::CreateModel`,
//! `FUN_14049def0` nebulae and `FUN_14049cf40` clusters, decompiled).
//!
//! Every galaxy owns one deterministic list of clusters (`RC`) and one of nebulae (`RN`). Both are
//! generated from two LCG streams seeded by a hash of the galaxy's position. **Decoded** from the
//! exe: the seeds, the group order, the object counts and their caps, the parameter ranges
//! (`ClusterGenerator` / `NebulaGenerator` settings of `config/main-def.cfg`), the exponential
//! `min - ln(u) * 0.0724 * (max - min)` draws for radius and luminosity, the Gaussian blob used for
//! positions, and the exclusion of objects the catalogs already cover when seen from the Sun.
//! **Approximated**: the disk groups (diffuse and supernova-remnant nebulae, open clusters) are
//! accepted against the galaxy model's emission map in the exe; here the caller supplies an
//! [`EmissionMap`] (the Milky Way's voxel map, or an analytic spiral), and the azimuth of the blob
//! direction is guessed. So counts, classes and statistics follow SpaceEngine, but individual
//! `RC`/`RN` indices will not land on the same objects as in the original program.

use crate::proc_galaxy::{GalNodeKey, truncated_normal};

const INV32: f32 = 1.0 / 4_294_967_296.0;
const LCG_A: u32 = 1_664_525;
const LCG_C: u32 = 1_013_904_223;
/// `DAT_1406d40d8`: scale of the exponential draws.
const EXP_K: f32 = 0.072_382_41;
/// Cluster/nebula indices of nebulae are offset by this in the exe's shared object id.
pub const NEBULA_ID_OFFSET: u32 = 0x400_0000;
/// SpaceEngine's catalog index of the Milky Way (from `RN 8514-9602`, `RS 8514-...` in saved places).
pub const MILKY_WAY_SE_INDEX: u32 = 8514;

#[inline]
fn lcg(s: &mut u32) -> u32 {
    *s = s.wrapping_mul(LCG_A).wrapping_add(LCG_C);
    *s
}

/// `min - ln(u) * 0.0724 * (max - min)` with `u` in `(1e-6, 1]` (`FUN_140444350`).
fn exp_draw(s: &mut u32, min: f32, max: f32) -> f32 {
    let u = lcg(s) as f32 * 0.999_998_99 * INV32 + 1.0e-6;
    min - u.ln() * EXP_K * (max - min)
}

/// Sum of 12 uniform draws, centred: approximately N(mean, sigma) (`FUN_140443d50`, stream S20).
fn irwin_hall(s: &mut u32, mean: f32, sigma: f32) -> f32 {
    let mut sum = 0f32;
    for _ in 0..12 {
        sum += lcg(s) as f32;
    }
    (sum * INV32 - 6.0) * sigma + mean
}

/// Random point of the 3D blob used for every position (`FUN_140441860(k)`): the radius follows
/// `r^2 = -k ln u` (so most points lie within `sqrt(k)`), the direction is isotropic.
fn blob(s: &mut u32, k: f32) -> [f32; 3] {
    let d1 = lcg(s);
    let d2 = lcg(s);
    let r = (d2 as f32 * 0.999_998_99 * INV32 + 1.0e-6).ln() * -k;
    let r = r.max(0.0).sqrt();
    let d3 = lcg(s);
    let z = 1.0 - (d3 as f32 + d3 as f32) * INV32;
    let st = (1.0 - (z * z) as f64).max(0.0).sqrt() as f32;
    // The exe passes a hidden register to cos/sin here; the azimuth is taken from the first draw.
    let phi = d1 as f32 * INV32 * std::f32::consts::TAU;
    [phi.cos() * st * r, phi.sin() * st * r, z * r]
}

/// Random orientation, 4 draws (`FUN_1404415a0`).
fn random_quat(s: &mut u32) -> [f32; 4] {
    let mut q = [0f32; 4];
    for v in q.iter_mut() {
        let u = lcg(s) as f32;
        *v = 1.0 - (u + u) * INV32;
    }
    let mut n = q[1] * q[1] + q[0] * q[0] + q[2] * q[2] + q[3] * q[3];
    if n == 0.0 {
        q[0] = 1.0;
        n = 1.0;
    }
    let k = 1.0 / n.sqrt();
    [q[0] * k, q[1] * k, q[2] * k, q[3] * k]
}

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

/// The `ClusterGenerator` / `NebulaGenerator` keys of `config/main-def.cfg` (defaults = shipped values).
#[derive(Debug, Clone, PartialEq)]
pub struct DeepSkyConfig {
    pub show_galaxies: bool,
    pub show_clusters: bool,
    pub show_nebulae: bool,
    /// Objects brighter than this from the Sun are left to the catalogs (home galaxy only).
    pub cluster_max_app_magn: f32,
    pub nebula_max_app_magn: f32,
    pub glob_density: f32,
    pub open_density: f32,
    pub glob_radius: [f32; 2],
    pub open_radius: [f32; 2],
    pub kern_radius: [f32; 2],
    pub glob_lum: [f32; 2],
    pub open_lum: [f32; 2],
    pub kern_lum: [f32; 2],
    /// Globular absolute magnitude: mean, sigma, min, max.
    pub glob_abs_magn: [f32; 4],
    pub diff_density: f32,
    pub plan_density: f32,
    pub snr_density: f32,
    pub diff_radius: [f32; 2],
    pub plan_radius: [f32; 2],
    pub snr_radius: [f32; 2],
    pub diff_lum: [f32; 2],
    pub plan_lum: [f32; 2],
    pub snr_lum: [f32; 2],
}

impl Default for DeepSkyConfig {
    fn default() -> Self {
        Self {
            show_galaxies: true,
            show_clusters: true,
            show_nebulae: true,
            cluster_max_app_magn: 8.1,
            nebula_max_app_magn: 6.1,
            glob_density: 1.0,
            open_density: 1.0,
            glob_radius: [5.0, 70.0],
            open_radius: [0.5, 15.0],
            kern_radius: [5.0, 40.0],
            glob_lum: [1000.0, 1.0e6],
            open_lum: [500.0, 25000.0],
            kern_lum: [500.0, 20000.0],
            glob_abs_magn: [-7.45, 0.95, -1.0, -11.0],
            diff_density: 1.0,
            plan_density: 5.0,
            snr_density: 0.1,
            diff_radius: [1.0, 70.0],
            plan_radius: [0.01, 15.0],
            snr_radius: [0.1, 50.0],
            diff_lum: [1000.0, 1.0e5],
            plan_lum: [0.1, 100.0],
            snr_lum: [0.1, 100.0],
        }
    }
}

impl DeepSkyConfig {
    /// Reads the relevant keys from the text of `config/main-def.cfg`; missing keys keep defaults.
    pub fn from_cfg(text: &str) -> Self {
        let root = crate::sc::parse(text);
        let mut c = Self::default();
        let find = |name: &str| root.items.iter().find(|i| i.key == name).and_then(|i| i.block.as_ref());
        let f = |b: &crate::sc::Block, k: &str| b.f64(k).map(|v| v as f32);
        let set = |b: Option<&crate::sc::Block>, apply: &mut dyn FnMut(&crate::sc::Block)| {
            if let Some(b) = b {
                apply(b);
            }
        };
        set(find("ClusterGenerator"), &mut |b| {
            let pair = |lo: &str, hi: &str, d: [f32; 2]| [f(b, lo).unwrap_or(d[0]), f(b, hi).unwrap_or(d[1])];
            c.cluster_max_app_magn = f(b, "ClusterMaxAppMagn").unwrap_or(c.cluster_max_app_magn);
            c.glob_density = f(b, "ClusterGlobDensity").unwrap_or(c.glob_density);
            c.open_density = f(b, "ClusterOpenDensity").unwrap_or(c.open_density);
            c.glob_radius = pair("ClusterGlobRadiusMin", "ClusterGlobRadiusMax", c.glob_radius);
            c.open_radius = pair("ClusterOpenRadiusMin", "ClusterOpenRadiusMax", c.open_radius);
            c.kern_radius = pair("ClusterKernRadiusMin", "ClusterKernRadiusMax", c.kern_radius);
            c.glob_lum = pair("ClusterGlobLumMin", "ClusterGlobLumMax", c.glob_lum);
            c.open_lum = pair("ClusterOpenLumMin", "ClusterOpenLumMax", c.open_lum);
            c.kern_lum = pair("ClusterKernLumMin", "ClusterKernLumMax", c.kern_lum);
            c.glob_abs_magn = [
                f(b, "ClusterGlobAbsMagn").unwrap_or(c.glob_abs_magn[0]),
                f(b, "ClusterGlobAbsMagnDisp").unwrap_or(c.glob_abs_magn[1]),
                f(b, "ClusterGlobAbsMagnMin").unwrap_or(c.glob_abs_magn[2]),
                f(b, "ClusterGlobAbsMagnMax").unwrap_or(c.glob_abs_magn[3]),
            ];
        });
        set(find("NebulaGenerator"), &mut |b| {
            let pair = |lo: &str, hi: &str, d: [f32; 2]| [f(b, lo).unwrap_or(d[0]), f(b, hi).unwrap_or(d[1])];
            c.nebula_max_app_magn = f(b, "NebulaMaxAppMagn").unwrap_or(c.nebula_max_app_magn);
            c.diff_density = f(b, "NebulaDiffDensity").unwrap_or(c.diff_density);
            c.plan_density = f(b, "NebulaPlanDensity").unwrap_or(c.plan_density);
            c.snr_density = f(b, "NebulaSNRDensity").unwrap_or(c.snr_density);
            c.diff_radius = pair("NebulaDiffRadiusMin", "NebulaDiffRadiusMax", c.diff_radius);
            c.plan_radius = pair("NebulaPlanRadiusMin", "NebulaPlanRadiusMax", c.plan_radius);
            c.snr_radius = pair("NebulaSNRRadiusMin", "NebulaSNRRadiusMax", c.snr_radius);
            c.diff_lum = pair("NebulaDiffLumMin", "NebulaDiffLumMax", c.diff_lum);
            c.plan_lum = pair("NebulaPlanLumMin", "NebulaPlanLumMax", c.plan_lum);
            c.snr_lum = pair("NebulaSNRLumMin", "NebulaSNRLumMax", c.snr_lum);
        });
        // `Show*` flags sit at the top level of main-def.cfg or inside a block; accept both.
        let flag = |k: &str| -> Option<bool> {
            root.items
                .iter()
                .find(|i| i.key == k)
                .and_then(|i| i.args.first())
                .and_then(|a| a.as_bool())
                .or_else(|| root.items.iter().filter_map(|i| i.block.as_ref()).find_map(|b| b.bool(k)))
        };
        c.show_galaxies = flag("ShowProcGalaxies").unwrap_or(c.show_galaxies);
        c.show_clusters = flag("ShowProcClusters").unwrap_or(c.show_clusters);
        c.show_nebulae = flag("ShowProcNebulae").unwrap_or(c.show_nebulae);
        c
    }

    /// `lum <= d^2 * K` is "fainter than the limit as seen from the Sun" (`d` in parsecs).
    fn limit_k(max_app_magn: f32) -> f32 {
        10f32.powf(0.4 * (-0.17 - max_app_magn))
    }
}

// ---------------------------------------------------------------------------------------------
// Inputs and outputs
// ---------------------------------------------------------------------------------------------

/// Probability that a point of the galaxy holds an object of the emission-following groups. `u` is the
/// position in units of the galaxy's x extent (`x`, `y` in [-1, 1] over the disc, `z` likewise, so it is
/// tiny for thin discs). `at` must return a value in `[0, 1]`.
pub trait EmissionMap {
    fn at(&self, unit: [f32; 3]) -> f32;
}

/// Fallback emission: exponential disc, a bulge and two logarithmic spiral arms.
pub struct AnalyticSpiral;

impl EmissionMap for AnalyticSpiral {
    fn at(&self, u: [f32; 3]) -> f32 {
        let r = (u[0] * u[0] + u[1] * u[1]).sqrt();
        let th = u[1].atan2(u[0]);
        let arms = 0.55 + 0.45 * (2.0 * (th - 2.4 * (r + 0.05).ln())).cos();
        let disc = (-3.0 * r).exp() * arms;
        let bulge = 0.8 * (-r * r / 0.012).exp();
        (disc + bulge).clamp(0.0, 1.0)
    }
}

/// What the generator needs to know about the parent galaxy.
#[derive(Debug, Clone)]
pub struct GalaxyShape {
    /// Position floats hashed into the seed: the in-node offset for a procedural galaxy, the
    /// catalog position for a catalog galaxy (parsecs).
    pub seed_pos: [f32; 3],
    /// Hubble type index 1..=16.
    pub hubble: u8,
    pub radius_pc: f32,
    pub lum: f32,
    pub compact: bool,
    /// Extents along the model x, y, z axes (`+0x40..+0x48` of the exe's record), parsecs.
    pub dims: [f32; 3],
    /// Type 9..=16 with a sprite model: enables the emission-following groups.
    pub has_model: bool,
    /// Galaxy-model position of the Sun when this is the home galaxy: objects brighter than the
    /// catalog limits seen from there are dropped.
    pub home: Option<[f32; 3]>,
    /// Number of catalog clusters / nebulae of the galaxy (they take the first indices).
    pub catalog_clusters: u32,
    pub catalog_nebulae: u32,
    /// The catalog galaxy's `GlobClusters`: its globular count instead of the luminosity formula.
    pub glob_clusters: Option<i32>,
}

impl GalaxyShape {
    /// Shape of a procedural galaxy; `model_scale` is the sprite model's `(R, R, R*SpriteOblZ)`.
    pub fn from_proc(g: &crate::proc_galaxy::ProcGalaxy, model_scale: Option<[f32; 3]>) -> Self {
        let off = g.pos_pc - g.node.center();
        let r = g.radius_pc;
        let dims = if g.hubble < 9 {
            let e = r * 2.4;
            let f = 1.0 - (g.hubble as f32 - 1.0) / 10.0;
            [f * e, e, f * e]
        } else if let Some(s) = model_scale {
            [r * s[0], r * s[1], r * s[2]]
        } else {
            [r, r, r]
        };
        GalaxyShape {
            seed_pos: [off.x as f32, off.y as f32, off.z as f32],
            hubble: g.hubble,
            radius_pc: r,
            lum: g.lum,
            compact: g.compact,
            dims,
            has_model: g.hubble >= 9 && model_scale.is_some(),
            home: None,
            catalog_clusters: 0,
            catalog_nebulae: 0,
            glob_clusters: None,
        }
    }

    /// Hash of the position that seeds both streams (`rec+0x60`, `FUN_14049b390`).
    pub fn seed(&self) -> i32 {
        let wrap = |v: f32| -> i32 {
            let v = v * 0.001;
            let m = 2_147_483_648.0f32;
            let w = v - ((v / (2.0 * m) + 0.5).floor() * 2.0) * m;
            w as i32
        };
        wrap(self.seed_pos[0])
            .wrapping_mul(0xb7823)
            .wrapping_add(wrap(self.seed_pos[2]).wrapping_mul(-0xe6490))
            .wrapping_add(wrap(self.seed_pos[1]).wrapping_mul(0xcd7d4))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterClass {
    /// The galactic nucleus cluster (`ClusterKern*`).
    Kern = 1,
    Globular = 2,
    Open = 3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcCluster {
    /// Index in the galaxy's cluster list (the last number of the RC name).
    pub index: u32,
    pub class: ClusterClass,
    /// Galaxy-model position, parsecs.
    pub pos: [f32; 3],
    pub radius_pc: f32,
    /// Luminosity, solar units.
    pub lum: f32,
    pub age_gyr: f32,
    /// Concentration `CenPow` (`tn(1.5, 0.3, 0.5, 2.5)`).
    pub cen_pow: f32,
    pub color: [f32; 3],
    /// Index (in the nebula list) of the diffuse nebula the cluster is embedded in.
    pub nebula: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NebulaClass {
    Diffuse = 1,
    Planetary = 2,
    Snr = 3,
}

impl NebulaClass {
    /// The catalog `Type` string (the key of the sprite-model pools).
    pub fn kind(self) -> &'static str {
        match self {
            NebulaClass::Diffuse => "Diffuse",
            NebulaClass::Planetary => "Planetary",
            NebulaClass::Snr => "SNR",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcNebula {
    pub index: u32,
    pub class: NebulaClass,
    pub pos: [f32; 3],
    pub radius_pc: f32,
    pub lum: f32,
    /// Random orientation `(w, x, y, z)` of the nebula model.
    pub quat: [f32; 4],
    /// Raw draw of the sprite-model pool pick (`pool[class][roll % len]`), if the pool is not empty.
    pub model_roll: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct GalaxySystem {
    pub clusters: Vec<ProcCluster>,
    pub nebulae: Vec<ProcNebula>,
}

/// Absolute magnitude of a luminosity in solar units.
pub fn abs_mag_of_lum(lum: f32) -> f32 {
    4.83 - 2.5 * lum.max(1e-30).log10()
}

// ---------------------------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------------------------

/// Builds the nebulae and clusters of a galaxy. `nebula_pools[c]` is the number of sprite models of
/// nebula class `c` (1 diffuse, 2 planetary, 3 SNR; index 0 unused), which sets how many draws the
/// model pick consumes.
pub fn generate_system(shape: &GalaxyShape, cfg: &DeepSkyConfig, emission: &dyn EmissionMap, nebula_pools: [u32; 4]) -> GalaxySystem {
    let seed = shape.seed();
    let nebulae = generate_nebulae(shape, cfg, emission, nebula_pools, seed);
    let clusters = generate_clusters(shape, cfg, emission, &nebulae, seed);
    GalaxySystem { clusters, nebulae }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    y * y + x * x + z * z
}

fn generate_nebulae(shape: &GalaxyShape, cfg: &DeepSkyConfig, emission: &dyn EmissionMap, pools: [u32; 4], seed: i32) -> Vec<ProcNebula> {
    let mut s24 = (seed as u32).wrapping_mul(0x17d63);
    let [dx, dy, dz] = shape.dims;
    let vol = dx * dy * dz * 1.0e-13;
    let k_home = DeepSkyConfig::limit_k(cfg.nebula_max_app_magn);
    // Counts and caps (an unsigned wrap of `(2 - u/4000) * 4000 + u` is `8000 + u % 4000`).
    let (mut n_diff, mut n_snr) = (0i32, 0i32);
    if shape.has_model {
        let d1 = s24.wrapping_mul(LCG_A).wrapping_add(LCG_C);
        n_diff = ((vol * cfg.diff_density * 10000.0) as i32).min(8000 + (d1 % 4000) as i32);
        let d2 = d1.wrapping_mul(LCG_A).wrapping_add(LCG_C);
        s24 = d2;
        n_snr = ((vol * cfg.snr_density * 10000.0) as i32).min(8000 + (d2 % 4000) as i32);
    }
    let d3 = lcg(&mut s24);
    let n_plan = ((vol * cfg.plan_density * 10000.0) as i32).min(8000 + (d3 % 4000) as i32);
    let mut out: Vec<ProcNebula> = Vec::new();
    let base = shape.catalog_nebulae;

    let run = |s24: &mut u32, class: NebulaClass, count: i32, out: &mut Vec<ProcNebula>| {
        let (lum_r, rad_r, thick) = match class {
            NebulaClass::Diffuse => (cfg.diff_lum, cfg.diff_radius, dx * 0.02),
            NebulaClass::Snr => (cfg.snr_lum, cfg.snr_radius, dx * 0.08),
            NebulaClass::Planetary => (cfg.plan_lum, cfg.plan_radius, 0.0),
        };
        for _ in 0..count.max(0) {
            let v = blob(s24, 0.25);
            let pos;
            if class == NebulaClass::Planetary {
                let ext = if shape.hubble < 9 { [dx, dy, dz] } else { [dx, dx, dx * 0.3] };
                pos = [ext[0] * v[0], ext[1] * v[1], ext[2] * v[2]];
            } else {
                let em = emission.at([v[0], v[1], v[2] * thick / dx]);
                let u = lcg(s24) as f32 * INV32;
                if u > em {
                    continue;
                }
                pos = [dx * v[0], dx * v[1], thick * v[2]];
            }
            let lum = exp_draw(s24, lum_r[0], lum_r[1]);
            if let Some(h) = shape.home {
                if lum > dist2(pos, h) * k_home {
                    continue;
                }
            }
            let radius = exp_draw(s24, rad_r[0], rad_r[1]);
            let quat = random_quat(s24);
            let count_pool = pools[class as usize];
            // The pool pick consumes two steps of the stream; the second one is the roll.
            let model_roll = if count_pool > 0 {
                lcg(s24);
                Some(lcg(s24))
            } else {
                None
            };
            out.push(ProcNebula { index: base + out.len() as u32, class, pos, radius_pc: radius, lum, quat, model_roll });
        }
    };
    if shape.has_model {
        run(&mut s24, NebulaClass::Diffuse, n_diff, &mut out);
        run(&mut s24, NebulaClass::Snr, n_snr, &mut out);
    }
    run(&mut s24, NebulaClass::Planetary, n_plan, &mut out);
    out
}

/// A nebula's young embedded cluster (FUN_14049cf40): radius `exp_draw(ClusterOpenRadius)` but at
/// most `N(0.4, 0.02)` of the nebula's, luminosity `exp_draw(ClusterOpenLum)` times the cube of that
/// cut, age `tn(0.02, 0.005, 0.001, 0.1)` Gyr, `CenPow` `tn(1.5, 0.3, 0.5, 2.5)`.
pub struct Embedded {
    pub radius_pc: f32,
    pub lum: f32,
    pub age_gyr: f32,
    pub cen_pow: f32,
}

fn embedded_draw(s24: &mut u32, s20: &mut u32, cfg: &DeepSkyConfig, nebula_radius_pc: f32) -> Embedded {
    let _q = random_quat(s24);
    let mut radius = exp_draw(s24, cfg.open_radius[0], cfg.open_radius[1]);
    let limit = irwin_hall(s20, 0.4, 0.02) * nebula_radius_pc;
    let mut frac = 1.0f32;
    if limit < radius {
        frac = limit / radius;
        radius = limit;
    }
    let mut lum = exp_draw(s24, cfg.open_lum[0], cfg.open_lum[1]);
    if frac < 1.0 {
        lum *= frac * frac * frac;
    }
    let age_gyr = truncated_normal(s20, 0.02, 0.005, 0.001, 0.1);
    let cen_pow = truncated_normal(s20, 1.5, 0.3, 0.5, 2.5);
    Embedded { radius_pc: radius, lum, age_gyr, cen_pow }
}

/// The young cluster of a catalog nebula that makes stars (its `GenerateStars`), drawn as SE draws a
/// procedural nebula's ([`embedded_draw`]) on streams seeded by the nebula's position hash (the
/// galaxies' rule, [`GalaxyShape::seed`]).
pub fn catalog_embedded(cfg: &DeepSkyConfig, pos_pc: [f32; 3], nebula_radius_pc: f32) -> Embedded {
    let seed = GalaxyShape { seed_pos: pos_pc, hubble: 0, radius_pc: 0.0, lum: 0.0, compact: false, dims: [0.0; 3], has_model: false, home: None, catalog_clusters: 0, catalog_nebulae: 0, glob_clusters: None }.seed();
    let mut s24 = (seed as u32).wrapping_mul(0x34cb);
    let mut s20 = (seed as u32).wrapping_mul(0xb5e1);
    embedded_draw(&mut s24, &mut s20, cfg, nebula_radius_pc)
}

fn generate_clusters(shape: &GalaxyShape, cfg: &DeepSkyConfig, emission: &dyn EmissionMap, nebulae: &[ProcNebula], seed: i32) -> Vec<ProcCluster> {
    let mut s24 = (seed as u32).wrapping_mul(0x34cb);
    let mut s20 = (seed as u32).wrapping_mul(0xb5e1);
    let [dx, dy, dz] = shape.dims;
    let k_home = DeepSkyConfig::limit_k(cfg.cluster_max_app_magn);
    let mut out: Vec<ProcCluster> = Vec::new();
    let base = shape.catalog_clusters;
    let push = |out: &mut Vec<ProcCluster>, class, pos, radius_pc, lum, (age_gyr, cen_pow), color, nebula| {
        let index = base + out.len() as u32;
        out.push(ProcCluster { index, class, pos, radius_pc, lum, age_gyr, cen_pow, color, nebula });
    };

    // Counts.
    let mut n_open = 0i32;
    if shape.has_model {
        let req = (dx * dy * dz * 1.389e-13 * cfg.open_density * 100_000.0) as i32;
        let u = lcg(&mut s24);
        n_open = req.min(350_000 + (u % 100_000) as i32);
    }
    // The type-16 galaxy without any flag has no globular clusters.
    let has_glob = !(shape.hubble == 16 && !shape.compact);
    let mut n_glob = 0i32;
    if has_glob {
        let lum = if shape.compact { shape.lum * 0.001 } else { shape.lum };
        let m = abs_mag_of_lum(lum);
        let req = if let Some(n) = shape.glob_clusters.filter(|n| *n >= 0) {
            n as f32
        } else if shape.hubble < 9 {
            let a = (m + 18.5).powf(2.0);
            let f = if m < -18.5 { 0.6 } else { 0.45 };
            10f32.powf((m + 15.0) * -0.4) * (a * f + 1.0)
        } else if shape.hubble == 9 {
            10f32.powf((m + 15.0) * -0.4) * 3.0
        } else {
            dx * dx * 1500.0 * 1.786e-12 * cfg.glob_density * 200.0
        };
        let u = lcg(&mut s24);
        n_glob = (req as i32).min(80_000 + (u % 40_000) as i32);
    }

    // 1. the galactic nucleus.
    let scale = (shape.radius_pc * 1.6 / 10000.0 + 0.2).clamp(0.0, 1.0);
    {
        let _q = random_quat(&mut s24);
        let radius = exp_draw(&mut s24, cfg.kern_radius[0], cfg.kern_radius[1]) * 0.3 * scale;
        let lum = exp_draw(&mut s24, cfg.kern_lum[0], cfg.kern_lum[1]) * scale * scale * scale;
        let age = truncated_normal(&mut s20, 0.1, 0.03, 0.01, 13.0);
        let cen_pow = truncated_normal(&mut s20, 1.5, 0.3, 0.5, 2.5);
        push(&mut out, ClusterClass::Kern, [0.0; 3], radius, lum, (age, cen_pow), [1.0, 0.9, 0.7], None);
    }

    // 2. one young cluster inside every diffuse nebula.
    for n in nebulae.iter().filter(|n| n.class == NebulaClass::Diffuse) {
        let e = embedded_draw(&mut s24, &mut s20, cfg, n.radius_pc);
        push(&mut out, ClusterClass::Open, n.pos, e.radius_pc, e.lum, (e.age_gyr, e.cen_pow), [0.7, 0.8, 1.0], Some(n.index));
    }

    // 3. globular clusters: a halo blob.
    if has_glob {
        let ext = if shape.hubble < 9 { [dx, dy, dz] } else { [dx, dx, dx * 0.7] };
        for _ in 0..n_glob {
            let v = blob(&mut s24, 0.25);
            let pos = [ext[0] * v[0], ext[1] * v[1], ext[2] * v[2]];
            // FUN_1404829f0(2): an exponential luminosity draw that the Gaussian magnitude replaces.
            let _ = exp_draw(&mut s24, cfg.glob_lum[0], cfg.glob_lum[1]);
            let g = cfg.glob_abs_magn;
            let m = truncated_normal(&mut s20, g[0], g[1], g[2], g[3]);
            let lum = crate::proc_galaxy::lum_from_mag(m);
            if let Some(h) = shape.home {
                if lum > dist2(pos, h) * k_home {
                    continue;
                }
            }
            let _q = random_quat(&mut s24);
            let radius = exp_draw(&mut s24, cfg.glob_radius[0], cfg.glob_radius[1]);
            let age = truncated_normal(&mut s20, 11.0, 3.0, 0.0, 13.0);
            let cen_pow = truncated_normal(&mut s20, 1.5, 0.3, 0.5, 2.5);
            push(&mut out, ClusterClass::Globular, pos, radius, lum, (age, cen_pow), [1.0, 0.9, 0.7], None);
        }
    }

    // 4. open clusters: the disc, following the emission map.
    if shape.has_model {
        for _ in 0..n_open {
            let v = blob(&mut s24, 0.25);
            let em = emission.at([v[0], v[1], v[2] * 0.05]);
            let u = lcg(&mut s24) as f32 * INV32;
            if u > em {
                continue;
            }
            let pos = [dx * v[0], dx * v[1], dx * 0.05 * v[2]];
            let lum = exp_draw(&mut s24, cfg.open_lum[0], cfg.open_lum[1]);
            if let Some(h) = shape.home {
                if lum > dist2(pos, h) * k_home {
                    continue;
                }
            }
            let _q = random_quat(&mut s24);
            let radius = exp_draw(&mut s24, cfg.open_radius[0], cfg.open_radius[1]);
            let age = truncated_normal(&mut s20, 0.15, 0.1, 0.001, 3.5);
            let cen_pow = truncated_normal(&mut s20, 1.5, 0.3, 0.5, 2.5);
            push(&mut out, ClusterClass::Open, pos, radius, lum, (age, cen_pow), [0.7, 0.8, 1.0], None);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------------------------

/// The galaxy a cluster or nebula belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GalaxyRef {
    /// A catalog galaxy by its SpaceEngine catalog index (the Milky Way is [`MILKY_WAY_SE_INDEX`]).
    Catalog(u32),
    /// A procedural galaxy.
    Procedural(GalNodeKey, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemKind {
    Cluster,
    Nebula,
}

/// `RC 8514-123`, `RC 0-3-116-2668-52089`, `RN 8514-9602`, `RN 0-9-110030168-905-7`.
pub fn system_name(kind: SystemKind, g: GalaxyRef, index: u32) -> String {
    let p = match kind {
        SystemKind::Cluster => "RC",
        SystemKind::Nebula => "RN",
    };
    match g {
        GalaxyRef::Catalog(i) => format!("{p} {i}-{index}"),
        GalaxyRef::Procedural(k, idx) => format!("{p} 0-{}-{}-{}-{}", k.level, k.id, idx, index),
    }
}

/// Parses an RC/RN name (case-insensitive). The nebula index is the plain index (without the
/// exe's 0x4000000 offset).
pub fn parse_system_name(s: &str) -> Option<(SystemKind, GalaxyRef, u32)> {
    let s = s.trim();
    let head = s.get(..3)?;
    let kind = if head.eq_ignore_ascii_case("RC ") {
        SystemKind::Cluster
    } else if head.eq_ignore_ascii_case("RN ") {
        SystemKind::Nebula
    } else {
        return None;
    };
    let nums: Vec<i64> = s[3..].trim().split('-').map(|p| p.trim().parse::<i64>()).collect::<Result<_, _>>().ok()?;
    match nums.as_slice() {
        [g, i] if *g >= 0 && *i >= 0 => Some((kind, GalaxyRef::Catalog(*g as u32), *i as u32)),
        [0, lvl, id, gi, i] if *lvl >= 0 && *lvl < crate::proc_galaxy::LEVELS as i64 && *id >= 0 && *gi >= 0 && *i >= 0 => {
            let key = GalNodeKey { level: *lvl as u8, id: *id as u64 };
            (key.id < 8u64.pow(key.level as u32)).then_some((kind, GalaxyRef::Procedural(key, *gi as u32), *i as u32))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milky_way_like() -> GalaxyShape {
        GalaxyShape {
            seed_pos: [0.0, 0.0, 0.0],
            hubble: 15,
            radius_pc: 30_000.0,
            lum: 1.5e10,
            compact: false,
            dims: [33_000.0, 33_000.0, 1_300.0],
            has_model: true,
            home: Some([-8_000.0, 0.0, 20.0]),
            glob_clusters: None,
            catalog_clusters: 20,
            catalog_nebulae: 190,
        }
    }

    #[test]
    fn names_round_trip() {
        let k = GalNodeKey { level: 3, id: 116 };
        let n = system_name(SystemKind::Cluster, GalaxyRef::Procedural(k, 2668), 52089);
        assert_eq!(n, "RC 0-3-116-2668-52089");
        assert_eq!(parse_system_name(&n), Some((SystemKind::Cluster, GalaxyRef::Procedural(k, 2668), 52089)));
        let n = system_name(SystemKind::Nebula, GalaxyRef::Catalog(MILKY_WAY_SE_INDEX), 9602);
        assert_eq!(n, "RN 8514-9602");
        assert_eq!(parse_system_name("rn 8514-9602"), Some((SystemKind::Nebula, GalaxyRef::Catalog(8514), 9602)));
        assert_eq!(parse_system_name("RC 0-3-9999-1-1"), None);
        assert_eq!(parse_system_name("RS 8514-1"), None);
        assert_eq!(parse_system_name("RC 12"), None);
    }

    #[test]
    fn deterministic_and_indexed() {
        let s = milky_way_like();
        let cfg = DeepSkyConfig::default();
        let a = generate_system(&s, &cfg, &AnalyticSpiral, [0, 3, 3, 4]);
        let b = generate_system(&s, &cfg, &AnalyticSpiral, [0, 3, 3, 4]);
        assert_eq!(a.clusters, b.clusters);
        assert_eq!(a.nebulae, b.nebulae);
        assert!(!a.clusters.is_empty() && !a.nebulae.is_empty());
        for (i, c) in a.clusters.iter().enumerate() {
            assert_eq!(c.index, 20 + i as u32);
        }
        for (i, n) in a.nebulae.iter().enumerate() {
            assert_eq!(n.index, 190 + i as u32);
            assert_eq!(n.model_roll.is_some(), true);
        }
        // Classes come in the exe's order: kernel first, then embedded, globular, open.
        assert_eq!(a.clusters[0].class, ClusterClass::Kern);
        let first_glob = a.clusters.iter().position(|c| c.class == ClusterClass::Globular).unwrap();
        assert!(a.clusters[1..first_glob].iter().all(|c| c.nebula.is_some()));
    }

    #[test]
    fn counts_match_the_galaxy_size() {
        let s = milky_way_like();
        let a = generate_system(&s, &DeepSkyConfig::default(), &AnalyticSpiral, [0, 3, 3, 4]);
        let glob = a.clusters.iter().filter(|c| c.class == ClusterClass::Globular).count();
        let open = a.clusters.iter().filter(|c| c.class == ClusterClass::Open && c.nebula.is_none()).count();
        let pn = a.nebulae.iter().filter(|n| n.class == NebulaClass::Planetary).count();
        let diff = a.nebulae.iter().filter(|n| n.class == NebulaClass::Diffuse).count();
        // ~33000^2 * 5.4e-7 = 590 globulars requested, 8000-12000 cap on planetaries.
        assert!(glob > 100 && glob < 700, "globulars {glob}");
        assert!(open > 100, "open {open}");
        assert!(pn > 500 && pn <= 12000, "planetary {pn}");
        assert!(diff > 0 && diff < 8000, "diffuse {diff}");
    }

    #[test]
    fn home_galaxy_drops_what_the_catalogs_cover() {
        let mut s = milky_way_like();
        let cfg = DeepSkyConfig::default();
        let with_home = generate_system(&s, &cfg, &AnalyticSpiral, [0, 3, 3, 4]);
        let home = s.home.unwrap();
        let k = DeepSkyConfig::limit_k(cfg.nebula_max_app_magn);
        for n in &with_home.nebulae {
            assert!(n.lum <= dist2(n.pos, home) * k * 1.0001);
        }
        s.home = None;
        let no_home = generate_system(&s, &cfg, &AnalyticSpiral, [0, 3, 3, 4]);
        let too_bright = no_home.nebulae.iter().filter(|n| n.lum > dist2(n.pos, home) * k).count();
        assert!(too_bright > 0, "without the home cut some nebulae are brighter than the limit");
    }

    #[test]
    fn elliptical_galaxies_only_have_planetaries_and_globulars() {
        let s = GalaxyShape {
            seed_pos: [123.0, -456.0, 789.0],
            hubble: 4,
            radius_pc: 20_000.0,
            lum: 5.0e10,
            compact: false,
            dims: [40_000.0 * 0.7, 40_000.0, 40_000.0 * 0.7],
            has_model: false,
            home: None,
            catalog_clusters: 0,
            catalog_nebulae: 0,
            glob_clusters: None,
        };
        let a = generate_system(&s, &DeepSkyConfig::default(), &AnalyticSpiral, [0, 3, 3, 4]);
        assert!(a.nebulae.iter().all(|n| n.class == NebulaClass::Planetary));
        assert!(a.clusters.iter().all(|c| matches!(c.class, ClusterClass::Kern | ClusterClass::Globular)));
        assert!(a.clusters.len() > 500, "{}", a.clusters.len());
        assert!(a.clusters.iter().all(|c| (0.5..=2.5).contains(&c.cen_pow)));
        // A catalog `GlobClusters` replaces the luminosity count (the nucleus comes on top).
        let b = generate_system(&GalaxyShape { glob_clusters: Some(14), ..s.clone() }, &DeepSkyConfig::default(), &AnalyticSpiral, [0, 3, 3, 4]);
        assert_eq!(b.clusters.iter().filter(|c| c.class == ClusterClass::Globular).count(), 14);
        let z = generate_system(&GalaxyShape { glob_clusters: Some(0), ..s }, &DeepSkyConfig::default(), &AnalyticSpiral, [0, 3, 3, 4]);
        assert_eq!(z.clusters.len(), 1);
    }

    #[test]
    fn config_parses_shipped_values() {
        let text = "ClusterGenerator\n{\n ClusterMaxAppMagn 9.5\n ClusterGlobRadiusMin 7\n}\nNebulaGenerator\n{\n NebulaPlanDensity 2\n}\nFoo\n{\n ShowProcNebulae false\n}\n";
        let c = DeepSkyConfig::from_cfg(text);
        assert_eq!(c.cluster_max_app_magn, 9.5);
        assert_eq!(c.glob_radius, [7.0, 70.0]);
        assert_eq!(c.plan_density, 2.0);
        assert!(!c.show_nebulae && c.show_clusters);
    }

    #[test]
    fn seed_hash_is_position_based() {
        let mut s = milky_way_like();
        let a = s.seed();
        s.seed_pos = [5000.0, 1.0, 2.0];
        assert_ne!(a, s.seed());
        assert_eq!(s.seed(), 5 * 0xb7823 + 0 * 0xcd7d4 + 0 * -0xe6490);
    }
}
