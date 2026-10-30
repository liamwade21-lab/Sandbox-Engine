//! SE's star cluster members: `seStarNodeCache::CreateClusterBlock` (FUN_14053a760), the node star
//! counts (FUN_140543c70) and the two star generators it calls, decompiled.
//!
//! - **Open** clusters (type 3) fill their root node only: points of a blob whose squared radius is
//!   exponential with mean 1/CenPow (FUN_140441860), kept inside the radius, each a star of
//!   [`open_member`] (FUN_1401d6c60) no brighter than half the cluster's light, until the members'
//!   light reaches the cluster's or there are 16384 of them.
//! - **Globular** (2) and **nucleus** (`Kern`, 1) clusters: every node of the octree tries
//!   [`node_tries`] uniform points in its cube and keeps those inside the radius with probability
//!   `exp(-c r/R)`, `c` = [`concentration`]; each is a star of [`dense_member`] (FUN_1401d63b0) in
//!   the level's magnitude band. The root's first member is a black hole ([`central_black_hole`]).
//!
//! The star physics is SE's where it is decoded: the class from the magnitude (FUN_1401da080), the
//! mass from the bolometric light (FUN_1401dd9f0), the lifetime (FUN_1401dd7e0, `climate`) against
//! the age deciding main sequence, giant or remnant, and the remnant from the mass (FUN_1401d5fe0).
//! The port's: a class's temperature (`SpectralClass::temperature`, SE's FUN_1401de8b0 table is not
//! read) and bolometric correction (SE's FUN_1401deb90), and the random streams (SE seeds each
//! node from its ids and the cluster's position; here a SplitMix stream per node).

use crate::star::{LumClass, SpectralClass, StarKind};
use crate::star_octree::Rng;

/// SE's most stars in one node (`0x4000`; the loops stop past `0x3fff`).
pub const NODE_CAP: usize = 16_384;
/// SE's oldest star (`DAT_1406d5544`, Gyr) and youngest (`DAT_1406d3ea8`).
const AGE_MAX: f64 = 12.87;
const AGE_MIN: f64 = 1e-4;
/// Luminosity class of remnants and its neutron star / black hole types (class word fields).
const LUM_REMNANT: u16 = 11;
/// Luminosity classes of SE's carbon / zirconium stars and Wolf-Rayet stars.
const LUM_CARBON: u16 = 9;
const LUM_WOLF_RAYET: u16 = 10;
/// SE's subtype names (FUN_1401dc7c0).
const WD_TYPES: [&str; 12] = ["DA", "DB", "DO", "DQ", "DZ", "DC", "DX", "DAB", "DAO", "DAZ", "DBZ", "WD"];
const CARBON_TYPES: [&str; 8] = ["MS", "S", "SC", "C-R", "C-N", "C-J", "C-H", "C-Hd"];
const WR_TYPES: [&str; 4] = ["WN", "WN/C", "WC", "WO"];
/// main-def.cfg `ProbWolfRayetStar` (0x14075f824) and `ProbCarbonStar` (0x14075f820).
const PROB_WOLF_RAYET: f64 = 0.05;
const PROB_CARBON: f64 = 0.08;
const TYPE_NS: u16 = 12;
const TYPE_BH: u16 = 13;

/// SE's normal draw (FUN_140443d50): twelve uniforms minus 6.
pub fn normal(rng: &mut Rng, mean: f64, sd: f64) -> f64 {
    let s: f64 = (0..12).map(|_| rng.f64()).sum();
    (s - 6.0) * sd + mean
}

/// SE's truncated normal (FUN_140443fd0): the inverse normal CDF of a uniform share of `[lo, hi]`.
pub fn trunc_normal(rng: &mut Rng, mean: f64, sd: f64, lo: f64, hi: f64) -> f64 {
    let cdf = |x: f64| 0.5 * (1.0 + crate::surface_gen::erf((x - mean) / (sd * std::f64::consts::SQRT_2)));
    let (a, b) = (cdf(lo), cdf(hi));
    let u = rng.f64();
    if b - a <= 0.0 {
        return lo + (hi - lo) * u;
    }
    (mean + sd * crate::surface_gen::inv_norm_cdf((a + (b - a) * u).clamp(1e-12, 1.0 - 1e-12))).clamp(lo, hi)
}

/// Count factor of a cluster's nodes (FUN_140543c70): `StarDensityInCluster x L x 1e-4` in [0.01, 5].
pub fn count_factor(lum: f64, density: f64) -> f64 {
    (density * lum * 1e-4).clamp(0.01, 5.0)
}

/// Points a globular or nucleus node at `level` tries (FUN_140543c70): the factor times the band
/// table's `1e5 / (level + 1)^3` (FUN_1401d7300), at the deepest level (`leaf` = Some(nucleus))
/// times `1/(1 - 0.45 - 3e-5)` globular, `1/(1 - 0.6 - 0.01)` nucleus (`0x140807fdc`/`0x140807f5c`),
/// the room for [`leaf_member`]'s luminosity-0 objects.
pub fn node_tries(level: u8, factor: f64, leaf: Option<bool>) -> usize {
    let extra = match leaf {
        Some(true) => 1.0 / (1.0 - 0.6 - 0.01),
        Some(false) => 1.0 / (1.0 - 0.45 - 3e-5),
        None => 1.0,
    };
    (factor * 1e5 / ((level as f64 + 1.0).powi(3)) * extra) as usize
}

/// Concentration of a globular or nucleus cluster's `exp(-c r/R)` profile (FUN_14053a760):
/// `7.3 sqrt(CenPow)` in [5, 30].
pub fn concentration(cen_pow: f64) -> f64 {
    (7.3 * cen_pow.max(0.0).sqrt()).clamp(5.0, 30.0)
}

/// SE's default `CenPow` of a cluster (class init FUN_1404826a0): `tn(1.5, 0.3, 0.5, 2.5)`, x1.5
/// for a nucleus.
pub fn default_cen_pow(rng: &mut Rng, nucleus: bool) -> f64 {
    trunc_normal(rng, 1.5, 0.3, 0.5, 2.5) * if nucleus { 1.5 } else { 1.0 }
}

/// A point of an open cluster's blob (FUN_140441860 with `k` = 1/CenPow): a uniform direction at a
/// radius whose square is exponential with mean `k` (cluster radii).
pub fn blob_point(rng: &mut Rng, k: f64) -> [f64; 3] {
    let u = rng.f64() * 0.999_998_986_721 + 1e-6;
    let r = (-u.ln() * k).sqrt();
    let z = 1.0 - 2.0 * rng.f64();
    let phi = std::f64::consts::TAU * rng.f64();
    let s = (1.0 - z * z).max(0.0).sqrt();
    [phi.cos() * s * r, phi.sin() * s * r, z * r]
}

/// SE's star class word (`star+0x24`): luminosity class << 11 | spectral type << 7 | subclass x10.
/// Luminosity classes 0 hypergiant .. 7 V, 9 carbon and zirconium stars (types MS, S, SC, C-R, C-N,
/// C-J, C-H, C-Hd), 10 Wolf-Rayet (WN, WN/C, WC, WO; subclass to 11.9), 11 remnant; types 0..9 O..Y,
/// remnants' 0..11 white dwarfs (DA, DB, DO, DQ, DZ, DC, DX, DAB, DAO, DAZ, DBZ, WD), 12 neutron star,
/// 13 black hole (SE's class text, FUN_1401dc7c0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ClassWord(pub u16);

impl ClassWord {
    pub fn new(lum: u16, ty: u16, sub: u16) -> Self {
        Self((lum & 0xf) << 11 | (ty & 0xf) << 7 | sub.min(99))
    }
    pub fn lum(self) -> u16 {
        (self.0 >> 11) & 0xf
    }
    pub fn ty(self) -> u16 {
        (self.0 >> 7) & 0xf
    }
    /// Subclass x10 (0..99).
    pub fn sub(self) -> u16 {
        self.0 & 0x7f
    }
    pub fn kind(self) -> StarKind {
        match (self.lum(), self.ty()) {
            (LUM_CARBON, _) => StarKind::Carbon,
            (LUM_WOLF_RAYET, _) => StarKind::WolfRayet,
            (LUM_REMNANT, TYPE_NS) => StarKind::NeutronStar,
            (LUM_REMNANT, TYPE_BH) => StarKind::BlackHole,
            (LUM_REMNANT, _) => StarKind::WhiteDwarf,
            (_, t) if t >= 7 => StarKind::BrownDwarf,
            _ => StarKind::Normal,
        }
    }

    /// The class as the port's [`SpectralClass`] (no allocation).
    pub fn spectral(self) -> SpectralClass {
        let kind = self.kind();
        let letter = match kind {
            StarKind::WhiteDwarf => 'D',
            StarKind::NeutronStar => 'Q',
            StarKind::BlackHole => 'X',
            StarKind::WolfRayet => 'W',
            StarKind::Carbon if self.ty() < 3 => 'S',
            StarKind::Carbon => 'C',
            _ => b"OBAFGKMLTY"[self.ty().min(9) as usize] as char,
        };
        let lum_class = match (kind, self.lum()) {
            (StarKind::WhiteDwarf, _) => LumClass::VII,
            (StarKind::Carbon, _) => LumClass::III,
            (_, 0) => LumClass::Hypergiant,
            (_, 1) => LumClass::Ia,
            (_, 2 | 3) => LumClass::Ib,
            (_, 4) => LumClass::II,
            (_, 5) => LumClass::III,
            (_, 6) => LumClass::IV,
            (_, 8) => LumClass::VI,
            _ => LumClass::V,
        };
        SpectralClass { kind, letter, subclass: self.sub() as f64 / 10.0, lum_class, raw: String::new() }
    }

    /// The class text, e.g. "G5.5V", "M2III", "B3Iab", "DA9.5", "Q", "X".
    pub fn name(self) -> String {
        let sub = self.sub() as f64 / 10.0;
        match self.kind() {
            StarKind::NeutronStar => "Q".into(),
            StarKind::BlackHole => "X".into(),
            StarKind::WhiteDwarf => format!("{}{sub}", WD_TYPES[self.ty().min(11) as usize]),
            StarKind::Carbon => format!("{}{sub}", CARBON_TYPES[self.ty().min(7) as usize]),
            StarKind::WolfRayet => format!("{}{sub}", WR_TYPES[self.ty().min(3) as usize]),
            _ => {
                let lum = ["Ia+", "Ia", "Iab", "Ib", "II", "III", "IV", "V", "VI"][self.lum().min(8) as usize];
                format!("{}{sub}{lum}", b"OBAFGKMLTY"[self.ty().min(9) as usize] as char)
            }
        }
    }
}

/// Upper Mv of the main-sequence types O..Y (`0x140676350`).
const MS_MV: [f64; 11] = [-10.0, -2.0, 0.5, 2.0, 4.0, 5.5, 8.0, 17.0, 20.0, 25.0, 30.0];
/// Mv of the giant types F..M (`0x140676330`).
const GIANT_MV: [f64; 5] = [2.0, 1.5, 0.0, -1.0, -3.0];
/// White dwarf Mv and temperature by subclass / 10 (`0x1406762b8`, `0x1406763c0`).
const WD_MV: [f64; 12] = [10.8, 11.9, 12.9, 13.6, 14.2, 14.6, 15.0, 15.3, 15.6, 15.7, 15.8, 16.0];
const WD_TEFF: [f64; 11] = [100_000.0, 50_400.0, 25_200.0, 16_800.0, 12_600.0, 10_080.0, 8_400.0, 7_200.0, 6_300.0, 5_600.0, 5_000.0];

fn rand_mod(rng: &mut Rng, n: u64) -> u16 {
    (rng.next_u64() % n) as u16
}

/// A main-sequence class from the absolute magnitude (FUN_1401da080 mode 0): the type whose Mv range
/// holds it, the subclass by its place in the range; O..G stars are subgiants (IV) one time in 20.
pub fn ms_class(rng: &mut Rng, mv: f64) -> ClassWord {
    let i = MS_MV.iter().position(|&m| mv < m).unwrap_or(MS_MV.len());
    let (ty, sub) = match i {
        0 => (0, 50),
        11 => return ClassWord::new(7, 9, 99),
        _ => (i as u16 - 1, (((mv - MS_MV[i - 1]) / (MS_MV[i] - MS_MV[i - 1])).clamp(0.0, 1.0) * 100.0) as u16),
    };
    let lum = if ty < 5 && rand_mod(rng, 100) < 5 { 6 } else { 7 };
    ClassWord::new(lum, ty, sub)
}

/// A giant's class from the absolute magnitude (FUN_1401da080 mode 1): brighter than Mv -3 a
/// supergiant of random type (hypergiant 5 %, Ia 25 %, Iab 32 %, Ib 38 %), else a class III of type
/// F..M by Mv (brighter is later); fainter than Mv 2 a main-sequence class.
pub fn giant_class(rng: &mut Rng, mv: f64) -> ClassWord {
    if mv < -3.0 {
        let u = rng.f64();
        let lum = if u < 0.05 {
            0
        } else if u < 0.3 {
            1
        } else if u >= 0.62 {
            3
        } else {
            2
        };
        let (ty, sub) = (rand_mod(rng, 7), rand_mod(rng, 100));
        return ClassWord::new(lum, ty, sub);
    }
    match GIANT_MV.iter().position(|&m| m < mv) {
        Some(0) => ms_class(rng, mv),
        None => ClassWord::new(5, 6, rand_mod(rng, 100)),
        Some(i) => {
            let f = ((mv - GIANT_MV[i]) / (GIANT_MV[i - 1] - GIANT_MV[i])).clamp(0.0, 1.0);
            ClassWord::new(5, i as u16 + 2, (f * 100.0) as u16)
        }
    }
}

/// SE's mass (solar) of a star of class `c` and bolometric light `lum_bol` (FUN_1401dd9f0).
pub fn mass_of(c: ClassWord, lum_bol: f64) -> f64 {
    let l = lum_bol;
    let m = if l > 36_109.844 {
        l / 1800.0
    } else if l > 16.0 {
        l.powf(0.285_714_3) - 0.208_271_31 + l * 5.767_715e-6
    } else if l > 0.031_490_8 {
        l.powf(0.25)
    } else if l > 0.001_152_730_7 {
        (l / 0.23).powf(0.434_782_6)
    } else {
        (l / 1237.6327).powf(0.165_813_73)
    };
    if c.lum() >= 9 {
        return m;
    }
    match c.ty() {
        0..=6 => m.clamp(0.0767, 180.0),
        7 | 8 => m.clamp(0.012, 0.0767),
        _ => m.clamp(0.003, 0.03),
    }
}

/// SE's lifetime (Gyr) of a star (FUN_1401dd7e0).
fn lifetime(mass: f64, c: ClassWord) -> f64 {
    crate::climate::star_lifetime_gyr(mass as f32, c.lum() as u32) as f64
}

/// One generated member star.
#[derive(Clone, Copy, Debug)]
pub struct Member {
    /// Absolute V magnitude.
    pub abs_mag: f32,
    pub teff: f32,
    pub mass: f32,
    pub age_gyr: f32,
    /// [Fe/H].
    pub feh: f32,
    pub class: ClassWord,
}

/// Temperature and bolometric light of a normal star of class `c` and magnitude `mv`.
fn light(c: ClassWord, mv: f64) -> (f64, f64) {
    let teff = c.spectral().temperature();
    let lum_bol = crate::star::lum_from_abs_mag(mv) * 10f64.powf(-0.4 * crate::star::bolometric_correction(teff));
    (teff, lum_bol)
}

/// A remnant of `mass` (FUN_1401d5fe0): a white dwarf below 1.44 solar masses (subclass by mass, a
/// random type of six), a neutron star below 3, else a black hole. With `own_mag` its magnitude is
/// the remnant's (the white dwarf table; `tn(18, 3, 12, 30)` neutron stars, `tn(20, 3, 14, 32)`
/// black holes), else `None` (the caller keeps its own).
fn remnant(rng: &mut Rng, mass: f64, own_mag: bool) -> (ClassWord, Option<f64>, f64) {
    if mass >= 3.0 {
        return (ClassWord::new(LUM_REMNANT, TYPE_BH, 0), own_mag.then(|| trunc_normal(rng, 20.0, 3.0, 14.0, 32.0)), 0.0);
    }
    if mass >= 1.44 {
        return (ClassWord::new(LUM_REMNANT, TYPE_NS, 0), own_mag.then(|| trunc_normal(rng, 18.0, 3.0, 12.0, 30.0)), 300_000.0);
    }
    let sub = (((mass - 1.44) / -0.88).clamp(0.0, 1.0) * 100.0) as usize;
    let c = ClassWord::new(LUM_REMNANT, rand_mod(rng, 6), sub as u16);
    let (i, f) = (sub.min(99) / 10, (sub.min(99) % 10) as f64 * 0.1);
    let mv = WD_MV[i] + (WD_MV[i + 1] - WD_MV[i]) * f;
    let teff = WD_TEFF[i] + (WD_TEFF[(i + 1).min(10)] - WD_TEFF[i]) * f;
    (c, own_mag.then_some(mv), teff)
}

/// SE's Wolf-Rayet conversion (FUN_1401d7190): a main-sequence or brighter O star no older than
/// 1 Myr is, with `ProbWolfRayetStar`, a WN, WN/C, WC or WO star (type `(u % 305) / 100`, subclass
/// `u % 120`).
pub fn wolf_rayet(rng: &mut Rng, c: ClassWord, age: f64) -> ClassWord {
    if age <= 0.001 && c.lum() <= 7 && c.ty() == 0 && rng.f64() <= PROB_WOLF_RAYET {
        let t = (rng.next_u64() % 305 / 100) as u16;
        let sub = (rng.next_u64() % 120) as u16;
        return ClassWord((0xa0 | t) << 7 | sub);
    }
    c
}

/// SE's carbon / zirconium conversion (FUN_1401d70c0): a giant (class III or brighter) of type G or
/// later whose `x6` (SE's star+0x18: `N(1, 0.17)` in dense clusters, `N(0.66, 0.17)` in open ones)
/// is at least 0.8 is, with `ProbCarbonStar`, one of SE's eight carbon and S-type classes.
pub fn carbon(rng: &mut Rng, c: ClassWord, x6: f64) -> ClassWord {
    if x6 >= 0.8 && c.lum() <= 5 && c.ty() >= 4 && rng.f64() <= PROB_CARBON {
        let t = (rng.next_u64() & 7) as u16;
        let sub = (rng.next_u64() % 100) as u16;
        return ClassWord((0x90 | t) << 7 | sub);
    }
    c
}

/// A member whose class may have been converted: its light and mass for the class (SE recomputes
/// them, FUN_1401de170).
fn converted(mv: f64, teff: f64, mass: f64, age: f64, feh: f64, old: ClassWord, c: ClassWord) -> Member {
    if c == old {
        return member(mv, teff, mass, age, feh, c);
    }
    let (teff, lb) = light(c, mv);
    member(mv, teff, mass_of(c, lb), age, feh, c)
}

fn member(mv: f64, teff: f64, mass: f64, age: f64, feh: f64, class: ClassWord) -> Member {
    Member { abs_mag: mv as f32, teff: teff as f32, mass: mass as f32, age_gyr: age as f32, feh: feh as f32, class }
}

/// A member of an open cluster of age `cluster_age` (FUN_1401d6c60): [Fe/H] N(-0.125, 0.07), the
/// cluster's age +- N(0, 0.01) Gyr, Mv N(3, 4) on the main sequence; a star whose lifetime is over is
/// instead a giant of Mv N(1, 1), or, when the giant's is over too, a remnant of Mv N(14, 0.83) and
/// the giant's mass.
pub fn open_member(rng: &mut Rng, cluster_age: f64) -> Member {
    let feh = normal(rng, -0.125_165_79, 0.07);
    let x6 = normal(rng, 0.662_251_3, 0.169_864_37);
    let age = (cluster_age + normal(rng, 0.0, 0.01)).clamp(AGE_MIN, AGE_MAX);
    let mv = normal(rng, 3.0, 4.0);
    let c = ms_class(rng, mv);
    let (teff, lb) = light(c, mv);
    let mass = mass_of(c, lb);
    if lifetime(mass, c) > age {
        let w = wolf_rayet(rng, c, age);
        return converted(mv, teff, mass, age, feh, c, w);
    }
    let mv_g = normal(rng, 1.0, 1.0);
    let g = giant_class(rng, mv_g);
    let (teff_g, lb_g) = light(g, mv_g);
    let mass_g = mass_of(g, lb_g);
    if lifetime(mass_g, g) > age {
        let k = carbon(rng, g, x6);
        return converted(mv_g, teff_g, mass_g, age, feh, g, k);
    }
    let mv_r = normal(rng, 14.0, 0.83);
    let (r, _, teff_r) = remnant(rng, mass_g, false);
    member(mv_r, teff_r, mass_g, age, feh, r)
}

/// A member of a globular (`nucleus` false) or nucleus cluster in the magnitude band `[lo, hi]`
/// (FUN_1401d63b0): [Fe/H] N(-1, 0.25); a main-sequence star of Mv `tn(0, 6, lo, hi)` (the mean is
/// `0x140808174`, which nothing writes; the band edges use 3); the age is the
/// cluster's +- N(0, 0.01) Gyr (nucleus: 70 % a uniform share of the star's lifetime, else a young
/// N(0, 0.01)). A star whose lifetime is over is a giant of Mv `tn(1, 1, lo, hi)`, unless the band
/// reaches past Mv 11.5 and the star is older than 1.1 lifetimes: then a remnant of its mass.
pub fn dense_member(rng: &mut Rng, nucleus: bool, cluster_age: f64, lo: f64, hi: f64) -> Member {
    let feh = normal(rng, -1.0, 0.25);
    // SE's star+0x18 for cluster stars (FUN_1401d6fe0 of 0), read by the carbon test.
    let x6 = normal(rng, 1.0, 0.169_864_37);
    let mv = trunc_normal(rng, 0.0, 6.0, lo, hi);
    let c = ms_class(rng, mv);
    let (teff, lb) = light(c, mv);
    let mass = mass_of(c, lb);
    let t_ms = lifetime(mass, c);
    let mv_g = trunc_normal(rng, 1.0, 1.0, lo, hi);
    let g = giant_class(rng, mv_g);
    let t_end = t_ms * 1.1;
    let age = if !nucleus {
        cluster_age + normal(rng, 0.0, 0.01)
    } else if rng.f64() >= 0.3 {
        rng.f64() * t_ms.min(AGE_MAX)
    } else {
        normal(rng, 0.0, 0.01)
    }
    .clamp(AGE_MIN, AGE_MAX);
    if t_ms > age {
        let w = wolf_rayet(rng, c, age);
        return converted(mv, teff, mass, age, feh, c, w);
    }
    if age < t_end || hi < 11.5 {
        let (teff_g, lb_g) = light(g, mv_g);
        let mass_g = mass_of(g, lb_g);
        // SE then sets the age to the giant's lifetime or 1.1 MS lifetimes, the shorter.
        let age = lifetime(mass_g, g).min(t_end);
        let k = carbon(rng, g, x6);
        return converted(mv_g, teff_g, mass_g, age, feh, g, k);
    }
    let (r, mv_r, teff_r) = remnant(rng, mass, true);
    member(mv_r.unwrap_or(mv), teff_r, mass, age, feh, r)
}

/// A member of the deepest octree level (FUN_1401d63b0 with its leaf flag): 45 % (globular) / 60 %
/// (nucleus) of the draws are objects of luminosity 0 (class 0x6800, radius 1 AU), which draw nothing
/// (`None`); then a share 0.0333 / 0.14 of the rest (`0x1408300f8` / `0x1408300fc`, summed at start-up)
/// are remnants of a mass `tn(8, 3, 3, 100)` (black holes: 3e-5 / 0.01), `tn(2.22, 0.39, 1.44, 3)`
/// (neutron stars: 2.5e-4 / 0.03) or `tn(1, 0.5, 0.56, 1.44)` (white dwarfs: 0.033 / 0.1)
/// (FUN_1401d8060), magnitudes their own (FUN_1401d5fe0); the others are [`dense_member`]s.
pub fn leaf_member(rng: &mut Rng, nucleus: bool, cluster_age: f64, lo: f64, hi: f64) -> Option<Member> {
    let share = if nucleus { 0.6 } else { 0.45 };
    let u = rng.f64();
    if u < share {
        return None;
    }
    let x = (u - share) / (1.0 - share);
    let (bh, ns, wd) = if nucleus { (0.01, 0.03, 0.1) } else { (3e-5, 2.5e-4, 0.033) };
    if x > bh + ns + wd {
        return Some(dense_member(rng, nucleus, cluster_age, lo, hi));
    }
    let mass = if x < bh {
        trunc_normal(rng, 8.0, 3.0, 3.0, 100.0)
    } else if x < bh + ns {
        trunc_normal(rng, 2.22, 0.39, 1.44, 3.0)
    } else {
        trunc_normal(rng, 1.0, 0.5, 0.56, 1.44)
    };
    let (c, mv, teff) = remnant(rng, mass, true);
    let feh = normal(rng, -1.0, 0.25);
    Some(member(mv.unwrap_or(15.0), teff, mass, cluster_age.clamp(AGE_MIN, AGE_MAX), feh, c))
}

/// The black hole SE makes the first member of a globular cluster older than 10 Myr, or of a
/// nucleus (FUN_14053a760 root branch): its mass, solar. Globular: `u sqrt(age/13) 1e4` kept in
/// `[u 100, u 1e5]`; nucleus: `u (R_galaxy/13700)^3 1e6` in `[u 1e5, u 1e10]` (three draws `u`).
/// SE gives it luminosity 1000, radius `M x 4.245e-6` solar (Schwarzschild) and the cluster's age.
pub fn central_black_hole(rng: &mut Rng, nucleus: bool, age_gyr: f64, galaxy_radius_pc: f64) -> Option<f64> {
    if !nucleus && age_gyr <= 0.01 {
        return None;
    }
    let (a, b, c) = (rng.f64(), rng.f64(), rng.f64());
    let (v, lo, hi) = if nucleus {
        (c * (galaxy_radius_pc / 13_700.0).powi(3) * 1e6, a * 1e5, b * 1e10)
    } else {
        (c * (age_gyr / 13.0).sqrt() * 1e4, a * 100.0, b * 1e5)
    };
    Some(if lo <= v { v.min(hi) } else { lo })
}

/// The class word of [`central_black_hole`] (`0x5e80`).
pub fn black_hole_class() -> ClassWord {
    ClassWord::new(LUM_REMNANT, TYPE_BH, 0)
}

/// SE's number of the cluster octree node at `level` with cell `cell` (0..2^level per axis, the upper
/// half 1): a child's number is its parent's x 8 + its octant, whose bits 4, 2, 1 mark the lower x,
/// y, z half (FUN_140543c70 with the octant table at 0x1407f4be0).
pub fn node_number(level: u8, cell: [u16; 3]) -> u64 {
    (0..level).rev().fold(0u64, |n, j| {
        let bit = |c: u16| 1 - ((c >> j) & 1) as u64;
        n * 8 + (bit(cell[0]) << 2 | bit(cell[1]) << 1 | bit(cell[2]))
    })
}

/// The cell of node `number` at `level` (inverse of [`node_number`]).
pub fn node_cell(level: u8, number: u64) -> Option<[u16; 3]> {
    if level > 15 || number >= 8u64.pow(level as u32) {
        return None;
    }
    let mut cell = [0u16; 3];
    for j in (0..level).rev() {
        let k = (number >> (3 * j as u32)) & 7;
        cell[0] = cell[0] * 2 + (1 - ((k >> 2) & 1)) as u16;
        cell[1] = cell[1] * 2 + (1 - ((k >> 1) & 1)) as u16;
        cell[2] = cell[2] * 2 + (1 - (k & 1)) as u16;
    }
    Some(cell)
}

/// Splits a member name `RSC <cluster numbers>-<level>-<node>-<index>` (FUN_1401c0180: two cluster
/// numbers in catalog galaxies, five in procedural ones) into its cluster's `RC <cluster numbers>`
/// name, level, node and index. Any case.
pub fn parse_member_name(s: &str) -> Option<(String, u8, u64, u32)> {
    let s = s.trim();
    if !s.get(..4)?.eq_ignore_ascii_case("RSC ") {
        return None;
    }
    let parts: Vec<&str> = s[4..].trim().split('-').map(str::trim).collect();
    if parts.len() != 5 && parts.len() != 8 {
        return None;
    }
    let n = parts.len();
    let level: u8 = parts[n - 3].parse().ok()?;
    let node: u64 = parts[n - 2].parse().ok()?;
    let index: u32 = parts[n - 1].parse().ok()?;
    if parts[..n - 3].iter().any(|p| p.parse::<u64>().is_err()) {
        return None;
    }
    node_cell(level, node)?;
    Some((format!("RC {}", parts[..n - 3].join("-")), level, node, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn se_member_names() {
        // The root is node 0; its upper (+x, +y, +z) octant is child 0, the lower one child 7.
        assert_eq!(node_number(0, [0; 3]), 0);
        assert_eq!(node_number(1, [1, 1, 1]), 0);
        assert_eq!(node_number(1, [0, 0, 0]), 7);
        assert_eq!(node_number(1, [0, 1, 1]), 4);
        for (l, c) in [(3u8, [5u16, 0, 7]), (4, [15, 3, 9]), (2, [1, 2, 3])] {
            assert_eq!(node_cell(l, node_number(l, c)), Some(c));
        }
        assert_eq!(node_cell(1, 8), None);
        assert_eq!(parse_member_name("RSC 8514-12-3-140-7"), Some(("RC 8514-12".into(), 3, 140, 7)));
        assert_eq!(parse_member_name("rsc 0-3-116-2668-52089-0-0-0"), Some(("RC 0-3-116-2668-52089".into(), 0, 0, 0)));
        assert_eq!(parse_member_name("RSC 8514-12-1-9-0"), None);
        assert_eq!(parse_member_name("RS 8514-12-1-0-0"), None);
    }

    #[test]
    fn se_node_counts() {
        // Omega Cen: L 1.1e6 -> the factor's cap 5; root 5e5 tries, level 4 4000 per node.
        let f = count_factor(1.1e6, 1.0);
        assert_eq!(f, 5.0);
        assert_eq!(node_tries(0, f, None), 500_000);
        assert_eq!(node_tries(4, f, None), 4000);
        assert_eq!(node_tries(4, f, Some(false)), 7273);
        assert_eq!(node_tries(4, f, Some(true)), 10_256);
        assert_eq!(count_factor(10.0, 1.0), 0.01);
        assert!((concentration(1.61) - 9.263).abs() < 0.01);
        assert_eq!(concentration(0.1), 5.0);
        assert_eq!(concentration(30.0), 30.0);
    }

    #[test]
    fn se_classes() {
        let mut rng = Rng(1);
        let c = ms_class(&mut rng, 6.0);
        assert_eq!((c.lum(), c.ty(), c.sub()), (7, 5, 20));
        assert_eq!(c.name(), "K2V");
        // A K giant at Mv -0.5, an M giant at -2, a supergiant above -3.
        let g = giant_class(&mut rng, -0.5);
        assert_eq!((g.lum(), g.ty(), g.sub()), (5, 5, 50));
        assert_eq!(g.name(), "K5III");
        assert_eq!(giant_class(&mut rng, -2.0).ty(), 6);
        assert!(giant_class(&mut rng, -5.0).lum() <= 3);
        assert_eq!(ms_class(&mut rng, 31.0).name(), "Y9.9V");
        assert_eq!(SpectralClass::parse(&g.name()).lum_class, LumClass::III);
        assert_eq!(black_hole_class().0, 0x5e80);
    }

    #[test]
    fn se_masses_and_remnants() {
        let v = ClassWord::new(7, 4, 50);
        assert!((mass_of(v, 1.0) - 1.0).abs() < 1e-6);
        assert!((mass_of(v, 100.0) - (100f64.powf(0.2857143) - 0.2082713 + 100.0 * 5.7677e-6)).abs() < 1e-5);
        let mut rng = Rng(3);
        let (wd, mv, t) = remnant(&mut rng, 0.6, true);
        assert_eq!(wd.kind(), StarKind::WhiteDwarf);
        assert_eq!(wd.sub(), 95);
        assert!((mv.unwrap() - 15.75).abs() < 1e-6 && (t - 5300.0).abs() < 1e-6);
        assert_eq!(remnant(&mut rng, 2.0, false).0.kind(), StarKind::NeutronStar);
        assert_eq!(remnant(&mut rng, 5.0, false).0.kind(), StarKind::BlackHole);
    }

    #[test]
    fn wolf_rayet_and_carbon_conversions_follow_se() {
        let mut rng = Rng(11);
        let o_star = ClassWord::new(7, 0, 50);
        let wr: Vec<ClassWord> = (0..20_000).map(|_| wolf_rayet(&mut rng, o_star, 0.0005)).filter(|c| *c != o_star).collect();
        assert!((800..1200).contains(&wr.len()), "{}", wr.len());
        assert!(wr.iter().all(|c| c.kind() == StarKind::WolfRayet && c.ty() <= 3 && c.sub() < 120));
        // Too old, or not an O star: never.
        assert_eq!(wolf_rayet(&mut rng, o_star, 0.002), o_star);
        assert_eq!(wolf_rayet(&mut rng, ClassWord::new(7, 1, 0), 0.0005), ClassWord::new(7, 1, 0));
        // Carbon: G-M giants with star+0x18 >= 0.8; not dwarfs, not K giants below 0.8.
        let k_giant = ClassWord::new(5, 5, 20);
        let n = (0..20_000).filter(|_| carbon(&mut rng, k_giant, 0.9) != k_giant).count();
        assert!((1300..1900).contains(&n), "{n}");
        assert_eq!(carbon(&mut rng, k_giant, 0.7), k_giant);
        assert_eq!(carbon(&mut rng, ClassWord::new(7, 5, 20), 0.9), ClassWord::new(7, 5, 20));
        // SE's names.
        assert_eq!(ClassWord((0xa0 | 2) << 7 | 55).name(), "WC5.5");
        assert_eq!(ClassWord((0x90 | 4) << 7 | 30).name(), "C-N3");
        assert_eq!(ClassWord::new(11, 3, 70).name(), "DQ7");
    }

    #[test]
    fn old_globular_bright_members_are_giants() {
        // In an 11 Gyr globular every member of the two brightest bands has left the main sequence.
        // A few are SE's carbon and S-type stars (ProbCarbonStar of the G-M giants).
        let mut rng = Rng(7);
        let mut carbon_stars = 0;
        for _ in 0..1000 {
            let m = dense_member(&mut rng, false, 11.0, -1.49, 1.79);
            carbon_stars += (m.class.kind() == StarKind::Carbon) as usize;
            assert!(m.class.lum() <= 6 || m.class.kind() == StarKind::Carbon, "{:?}", m.class.name());
            assert!(m.abs_mag >= -1.49 && m.abs_mag <= 1.79);
        }
        assert!((20..120).contains(&carbon_stars), "{carbon_stars}");
        // The faint band keeps its dwarfs.
        let dwarfs = (0..200).filter(|_| dense_member(&mut rng, false, 11.0, 8.19, 25.3).class.lum() == 7).count();
        assert!(dwarfs > 150, "{dwarfs}");
        // The leaf level: 45 % nothing, 3.3 % of the rest remnants (mostly white dwarfs).
        let leaf: Vec<Option<Member>> = (0..20_000).map(|_| leaf_member(&mut rng, false, 11.0, 8.19, 25.3)).collect();
        let none = leaf.iter().filter(|m| m.is_none()).count() as f64 / 20_000.0;
        let wd = leaf.iter().flatten().filter(|m| m.class.kind() == StarKind::WhiteDwarf).count() as f64 / 20_000.0;
        assert!((none - 0.45).abs() < 0.02 && (wd - 0.55 * 0.033).abs() < 0.006, "{none} {wd}");
        // Each band's main-sequence draws centre on SE's mean 0, not 3: the leaf band starts at its edge.
        let mv: f64 = (0..2000).map(|_| trunc_normal(&mut rng, 0.0, 6.0, 8.19, 25.3)).sum::<f64>() / 2000.0;
        assert!((mv - 10.95).abs() < 0.2, "{mv}");
    }

    #[test]
    fn open_members_and_black_holes() {
        let mut rng = Rng(11);
        // Mv N(3, 4) puts a fifth of the draws above B-star masses, which are giants at 50 Myr.
        let young: Vec<Member> = (0..2000).map(|_| open_member(&mut rng, 0.05)).collect();
        let dwarfs = young.iter().filter(|m| m.class.lum() == 7).count();
        assert!(dwarfs > 1200 && dwarfs < 1800, "{dwarfs}");
        let feh = young.iter().map(|m| m.feh as f64).sum::<f64>() / 2000.0;
        assert!((feh + 0.125).abs() < 0.01, "{feh}");
        // An old open cluster has evolved members.
        assert!((0..2000).any(|_| open_member(&mut rng, 3.0).class.lum() != 7));
        assert!(central_black_hole(&mut rng, false, 0.005, 0.0).is_none());
        for _ in 0..100 {
            let m = central_black_hole(&mut rng, false, 12.0, 0.0).unwrap();
            assert!(m > 0.0 && m <= 1e5);
        }
        let p = blob_point(&mut rng, 0.5);
        assert!(p.iter().all(|x| x.is_finite()));
    }
}
