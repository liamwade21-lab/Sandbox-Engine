//! SpaceEngine's per-body surface parameters, decoded from SpaceEngine.exe 0.991:
//! the body classes SE derives first (`seSpaceBody::InitSurfaceClass` and helpers) and the
//! generator that fills every `Surface` key the catalog leaves out (FUN_1402f37c0, run for each
//! body when a planetary system is initialised). The biome preset is chosen with the generated
//! `SurfStyle`. Layout, RNG and formulas: docs/formats/surface_params.md.

use crate::sc::Block;

/// "Not given" marker SE stores in every surface slot before reading the catalog (-1e38).
pub const UNSET: f32 = -1.0e38;

/// Surface slots, in SE's record order (byte offset 0x50 + 4 * index in the body record).
pub const N_PARAMS: usize = 91;
pub const RANDOMIZE: usize = 0;
pub const MAIN_FREQ: usize = 3;
pub const VENUS_MAGN: usize = 4;
pub const VENUS_FREQ: usize = 5;
pub const SEA_LEVEL: usize = 6;
pub const SNOW_LEVEL: usize = 7;
pub const COLOR_DIST_MAGN: usize = 8;
pub const COLOR_DIST_FREQ: usize = 9;
pub const DRIVEN_DARKENING: usize = 10;
pub const CLIMATE_POLE: usize = 11;
pub const CLIMATE_TROPIC: usize = 12;
pub const CLIMATE_EQUATOR: usize = 13;
pub const TROPIC_WIDTH: usize = 14;
pub const TROPIC_LATITUDE: usize = 15;
pub const CLIMATE_STEPPE_MIN: usize = 16;
pub const CLIMATE_STEPPE_MAX: usize = 17;
pub const CLIMATE_GRASS_MIN: usize = 18;
pub const CLIMATE_GRASS_MAX: usize = 19;
pub const CLIMATE_FOREST_MIN: usize = 20;
pub const CLIMATE_FOREST_MAX: usize = 21;
pub const HUMIDITY: usize = 22;
pub const BEACH_WIDTH: usize = 23;
pub const HEIGHT_TEMP_GRAD: usize = 24;
pub const ICECAP_LATITUDE: usize = 25;
pub const ICECAP_HEIGHT: usize = 26;
pub const TERRACE_PROB: usize = 27;
pub const EROSION: usize = 28;
pub const MARE_FREQ: usize = 29;
pub const MARE_DENSITY: usize = 30;
pub const MONTES_MAGN: usize = 31;
pub const MONTES_FREQ: usize = 32;
pub const MONTES_FRACTION: usize = 33;
pub const MONTES_SPIKY: usize = 34;
pub const DUNES_MAGN: usize = 35;
pub const DUNES_FREQ: usize = 36;
pub const DUNES_FRACTION: usize = 37;
pub const HILLS_MAGN: usize = 38;
pub const HILLS_FREQ: usize = 39;
pub const HILLS_FRACTION: usize = 40;
pub const HILLS2_FRACTION: usize = 41;
pub const RIVERS_MAGN: usize = 42;
pub const RIVERS_FREQ: usize = 43;
pub const RIVERS_SIN: usize = 44;
pub const RIFTS_MAGN: usize = 45;
pub const RIFTS_FREQ: usize = 46;
pub const RIFTS_SIN: usize = 47;
pub const EQRIDGE_MAGN: usize = 48;
pub const EQRIDGE_WIDTH: usize = 49;
pub const EQRIDGE_MOD_MAGN: usize = 50;
pub const EQRIDGE_MOD_FREQ: usize = 51;
pub const CANYONS_MAGN: usize = 52;
pub const CANYONS_FREQ: usize = 53;
pub const CANYONS_FRACTION: usize = 54;
pub const CRACKS_MAGN: usize = 55;
pub const CRACKS_FREQ: usize = 56;
pub const CRACKS_OCTAVES: usize = 57;
pub const CRATER_MAGN: usize = 58;
pub const CRATER_FREQ: usize = 59;
pub const CRATER_DENSITY: usize = 60;
pub const CRATER_OCTAVES: usize = 61;
pub const CRATER_RAYED_FACTOR: usize = 62;
pub const VOLCANO_MAGN: usize = 63;
pub const VOLCANO_FREQ: usize = 64;
pub const VOLCANO_DENSITY: usize = 65;
pub const VOLCANO_OCTAVES: usize = 66;
pub const VOLCANO_ACTIVITY: usize = 67;
pub const VOLCANO_FLOWS: usize = 68;
pub const VOLCANO_RADIUS: usize = 69;
pub const VOLCANO_TEMP: usize = 70;
/// Unnamed slot (0x16c) read as a lava/ice cover fraction; never generated.
pub const LAVA_LEVEL: usize = 71;
pub const LAVA_COVER_TIDAL: usize = 72;
pub const LAVA_COVER_SUN: usize = 73;
pub const LAVA_COVER_YOUNG: usize = 74;
pub const STRIPE_ZONES: usize = 75;
pub const STRIPE_FLUCT: usize = 76;
pub const STRIPE_TWIST: usize = 77;
/// The `Clouds` block's `mainFreq`, `mainOctaves` and `Coverage` live in the same record.
pub const CLOUDS_MAIN_FREQ: usize = 78;
pub const CLOUDS_MAIN_OCTAVES: usize = 79;
pub const CLOUDS_COVERAGE: usize = 80;
pub const CYCLONE_MAGN: usize = 81;
pub const CYCLONE_FREQ: usize = 82;
pub const CYCLONE_DENSITY: usize = 83;
pub const CYCLONE_OCTAVES: usize = 84;
pub const CYCLONE_MAGN2: usize = 85;
pub const CYCLONE_FREQ2: usize = 86;
pub const CYCLONE_LATITUDE2: usize = 87;
pub const CYCLONE_OCTAVES2: usize = 88;
pub const DETAIL_SCALE: usize = 89;
pub const SURF_STYLE: usize = 90;

/// `Surface` block keys of the slots (the reader FUN_14046f840); "" for slots without one.
pub const KEYS: [&str; N_PARAMS] = [
    "", "", "", "mainFreq", "venusMagn", "venusFreq", "seaLevel", "snowLevel", "colorDistMagn",
    "colorDistFreq", "drivenDarkening", "climatePole", "climateTropic", "climateEquator", "tropicWidth",
    "tropicLatitude", "climateSteppeMin", "climateSteppeMax", "climateGrassMin", "climateGrassMax",
    "climateForestMin", "climateForestMax", "humidity", "beachWidth", "heightTempGrad", "icecapLatitude",
    "icecapHeight", "terraceProb", "erosion", "mareFreq", "mareDensity", "montesMagn", "montesFreq",
    "montesFraction", "montesSpiky", "dunesMagn", "dunesFreq", "dunesFraction", "hillsMagn", "hillsFreq",
    "hillsFraction", "hills2Fraction", "riversMagn", "riversFreq", "riversSin", "riftsMagn", "riftsFreq",
    "riftsSin", "eqridgeMagn", "eqridgeWidth", "eqridgeModMagn", "eqridgeModFreq", "canyonsMagn",
    "canyonsFreq", "canyonsFraction", "cracksMagn", "cracksFreq", "cracksOctaves", "craterMagn",
    "craterFreq", "craterDensity", "craterOctaves", "craterRayedFactor", "volcanoMagn", "volcanoFreq",
    "volcanoDensity", "volcanoOctaves", "volcanoActivity", "volcanoFlows", "volcanoRadius", "volcanoTemp",
    "", "lavaCoverTidal", "lavaCoverSun", "lavaCoverYoung", "stripeZones", "stripeFluct", "stripeTwist",
    "", "", "", "cycloneMagn", "cycloneFreq", "cycloneDensity", "cycloneOctaves", "cycloneMagn2",
    "cycloneFreq2", "cycloneLatitude2", "cycloneOctaves2", "detailScale", "SurfStyle",
];

// ---------------------------------------------------------------------------------------
// Random numbers
// ---------------------------------------------------------------------------------------

const INV_2_32_F: f32 = 2.328_306_4e-10;
const INV_2_32_D: f64 = 2.328_306_436_538_696_3e-10;

/// SE's two thread-local LCG streams (multiplier 1664525, increment 1013904223). Stream `a`
/// (TLS+0x24) feeds uniform and integer draws, stream `b` (TLS+0x20) Gaussian and truncated
/// normal draws, so the two sequences advance independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeRng {
    pub a: u32,
    pub b: u32,
}

impl SeRng {
    pub fn new(a: u32, b: u32) -> Self {
        Self { a, b }
    }

    fn lcg(s: &mut u32) -> u32 {
        *s = s.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        *s
    }

    /// Uniform in [0, scale) (FUN_140444850), in f32 like the exe.
    pub fn uniform(&mut self, scale: f32) -> f32 {
        Self::lcg(&mut self.a) as f32 * scale * INV_2_32_F
    }

    /// `rand % n` (FUN_140443910); 0 without a draw when n is 0.
    pub fn int(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        Self::lcg(&mut self.a) % n
    }

    /// Sum of 12 uniforms (FUN_140443d50).
    pub fn gauss(&mut self, mean: f32, sigma: f32) -> f32 {
        let mut s = 0.0f32;
        for _ in 0..12 {
            s += Self::lcg(&mut self.b) as f32;
        }
        (s * INV_2_32_F - 6.0) * sigma + mean
    }

    /// Normal(mean, sigma) truncated to [lo, hi] by inverting the CDF (FUN_140443fd0).
    pub fn tnormal(&mut self, mean: f32, sigma: f32, lo: f32, hi: f32) -> f32 {
        let u = Self::lcg(&mut self.b) as f64 * INV_2_32_D;
        let k = std::f32::consts::SQRT_2 * sigma;
        let a = erf(((lo - mean) / k) as f64) as f32 as f64;
        let d = erf(((hi - mean) / k) as f64) as f32 as f64 - a;
        if d == 0.0 {
            return (((hi - lo) as f64) * u + lo as f64) as f32;
        }
        if u == 0.0 {
            return hi;
        }
        let x = inv_norm_cdf((d * u + a + 1.0) * 0.5) / std::f64::consts::SQRT_2;
        (x as f32) * k + mean
    }
}

fn erfc_cf(x: f64) -> f64 {
    // erfc(x) = exp(-x^2) / sqrt(pi) / (x + (1/2) / (x + 1 / (x + (3/2) / (x + ...)))), x >= 2.5.
    let mut t = x;
    for n in (1..=80).rev() {
        t = x + (n as f64 * 0.5) / t;
    }
    (-x * x).exp() / (std::f64::consts::PI.sqrt() * t)
}

/// Error function to about 1e-13.
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let ax = x.abs();
    let r = if ax < 2.5 {
        let x2 = ax * ax;
        let (mut term, mut sum, mut n) = (ax, ax, 0.0);
        loop {
            n += 1.0;
            term *= -x2 / n;
            let t = term / (2.0 * n + 1.0);
            sum += t;
            if t.abs() <= 1e-17 * sum.abs() || n > 200.0 {
                break;
            }
        }
        sum * 2.0 / std::f64::consts::PI.sqrt()
    } else {
        1.0 - erfc_cf(ax)
    };
    r.copysign(x)
}

fn erfc(x: f64) -> f64 {
    if x >= 2.5 {
        erfc_cf(x)
    } else {
        1.0 - erf(x)
    }
}

/// Standard normal quantile (Acklam's approximation with one Halley step).
pub fn inv_norm_cdf(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [-3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2, 1.383577518672690e2, -3.066479806614716e1, 2.506628277459239];
    const B: [f64; 5] = [-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2, 6.680131188771972e1, -1.328068155288572e1];
    const C: [f64; 6] = [-7.784894002430293e-3, -3.223964580411365e-1, -2.400758277161838, -2.549732539343734, 4.374664141464968, 2.938163982698783];
    const D: [f64; 4] = [7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996, 3.754408661907416];
    let tail = |q: f64| (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    let x = if p < 0.02425 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p <= 1.0 - 0.02425 {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    };
    let e = 0.5 * erfc(-x / std::f64::consts::SQRT_2) - p;
    let u = e * (2.0 * std::f64::consts::PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

/// Seeds of a catalog body (FUN_1404652f0): a x131 hash of `"<names>|<parent>"` (the header
/// name string as written, then `ParentBody`), bytes as signed chars.
/// A catalog body's record seeds: SE's index seeds a generated body carries (`SeSeedA`/`SeSeedB`,
/// written by `procgen`), else the name hash of [`catalog_seeds`].
pub fn body_seeds(b: &crate::catalog::BodyRecord) -> (u32, u32) {
    match (b.block.f64("SeSeedA"), b.block.f64("SeSeedB")) {
        (Some(a), Some(s)) => (a as u32, s as u32),
        _ => catalog_seeds(&b.names.join("/"), &b.parent),
    }
}

pub fn catalog_seeds(names: &str, parent: &str) -> (u32, u32) {
    let mut h: i32 = 0;
    for c in names.bytes().chain(std::iter::once(b'|')).chain(parent.bytes()) {
        h = h.wrapping_mul(0x83).wrapping_add(c as i8 as i32);
    }
    let h = h as u32;
    (h.wrapping_mul(0x0014_ab8d).wrapping_add(0x047f_7e9f), h.wrapping_mul(0x0056_a18d).wrapping_add(0x0044_796b))
}

// ---------------------------------------------------------------------------------------
// Body classes
// ---------------------------------------------------------------------------------------

/// SE object types (`Type` table at 0x140687370).
pub mod obj {
    pub const STAR: i32 = 3;
    pub const PLANET: i32 = 4;
    pub const MOON: i32 = 5;
    pub const DWARF_PLANET: i32 = 6;
    pub const DWARF_MOON: i32 = 7;
    pub const ASTEROID: i32 = 8;
    pub const COMET: i32 = 9;
}

/// Bulk classes (table at 0x140687290); `NONE` doubles as "any" in presets.
pub mod bulk {
    pub const ASTEROID: i32 = 0;
    pub const FERRIA: i32 = 1;
    pub const TERRA: i32 = 2;
    pub const CARBONIA: i32 = 3;
    pub const AQUARIA: i32 = 4;
    pub const NEPTUNE: i32 = 5;
    pub const JUPITER: i32 = 6;
    pub const CHTHONIA: i32 = 7;
    pub const BROWN_DWARF: i32 = 8;
    pub const SUN: i32 = 9;
    pub const NONE: i32 = 10;
}

pub const TEMP_NAMES: [&str; 7] = ["Torrid", "Hot", "Warm", "Temperate", "Cool", "Cold", "Frigid"];
pub const ATMO_NAMES: [&str; 8] = ["Airless", "Infrabaric", "Hypobaric", "Mesobaric", "Hyperbaric", "Ultrabaric", "NonAirless", "Any"];
pub const VOLQ_NAMES: [&str; 10] = ["Airless", "Arid", "Lacustrine", "Marine", "Oceanic", "Superoceanic", "Glacial", "Superglacial", "NonArid", "Any"];
pub const BULK_NAMES: [&str; 11] = ["Asteroid", "Ferria", "Terra", "Carbonia", "Aquaria", "Neptune", "Jupiter", "Chthonia", "BrownDwarf", "Sun", "None"];

/// Bulk class of a catalog `Class` string. SE derives it from the composition; catalogs and the
/// port's generator name it, and the older names map onto SE's set.
pub fn bulk_of(class: &str) -> i32 {
    let c: String = class.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    match c.as_str() {
        "asteroid" | "comet" | "small" | "minor" | "minorbody" => bulk::ASTEROID,
        "ferria" | "iron" => bulk::FERRIA,
        "carbonia" => bulk::CARBONIA,
        "aquaria" | "glacia" | "oceania" => bulk::AQUARIA,
        "neptune" | "icegiant" | "uranus" => bulk::NEPTUNE,
        "jupiter" | "gasgiant" | "saturn" | "gas" => bulk::JUPITER,
        "chthonia" => bulk::CHTHONIA,
        "browndwarf" => bulk::BROWN_DWARF,
        _ => bulk::TERRA,
    }
}

/// Temperature class: > 1000 Torrid, > 500 Hot, > 330 Warm, > 250 Temperate, > 170 Cool, > 90 Cold,
/// else Frigid. FUN_1404d6850 first applies it to the surface temperature; once the system is
/// built FUN_140510f40 reapplies it to the temperature model's average, which the class string and
/// the generator use.
pub fn temp_class(t: f32) -> i32 {
    if t > 1000.0 {
        0
    } else if t > 500.0 {
        1
    } else if t > 330.0 {
        2
    } else if t > 250.0 {
        3
    } else if t > 170.0 {
        4
    } else if t > 90.0 {
        5
    } else {
        6
    }
}

/// Atmosphere class from surface pressure in atm (FUN_1404caaa0); giants and stars are 7.
pub fn atmo_class(bulk: i32, pressure_atm: f32) -> i32 {
    if bulk >= 5 {
        return 7;
    }
    let p = pressure_atm;
    if p > 1000.0 {
        5
    } else if p > 10.0 {
        4
    } else if p > 0.1 {
        3
    } else if p > 0.001 {
        2
    } else if p > 1e-6 {
        1
    } else {
        0
    }
}

/// An ocean as the volatiles class sees it: depth (km), the body's `BumpHeight` (km), and the
/// global-ocean flags (no land; Oceanic rather than Superoceanic).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OceanInfo {
    pub depth_km: f32,
    pub bump_height_km: f32,
    pub global: bool,
    pub global_shallow: bool,
}

/// Volatiles class (FUN_140505680): 9 for asteroids and giants; Airless/Arid by atmosphere; with an
/// ocean Lacustrine (depth <= 0.1 bump), Marine (<= 0.9 bump), Oceanic, or for global oceans
/// Superoceanic (Oceanic when flagged shallow).
pub fn volq_class(bulk: i32, atmo: i32, ocean: Option<OceanInfo>) -> i32 {
    if bulk == bulk::ASTEROID || bulk > 4 {
        return 9;
    }
    let mut v = i32::from(atmo != 0);
    if let Some(o) = ocean {
        v = if o.global {
            if o.global_shallow { 4 } else { 5 }
        } else if o.depth_km > o.bump_height_km * 0.1 {
            if o.depth_km <= o.bump_height_km * 0.9 { 3 } else { 4 }
        } else {
            2
        };
    }
    v
}

/// Size class (FUN_1404cabf0): 0 micro .. 5 mega by mass (Earth masses).
pub fn size_class(bulk: i32, mass_earth: f32) -> i32 {
    let t = match bulk {
        bulk::JUPITER => [0.0, 63.56568, 158.9142, 794.571, 3972.855],
        bulk::NEPTUNE => [0.0, 4.0, 10.0, 25.0, 62.5],
        _ => [0.002, 0.02, 0.2, 2.0, 10.0],
    };
    t.iter().rposition(|&x| mass_earth > x).map_or(0, |i| i as i32 + 1)
}

/// Surface class (`InitSurfaceClass`): picks the generator branch. 1 oceanic, 2 lacustrine/marine
/// terra, 3 lacustrine/marine aquaria, 4 arid, 5 airless or thin, 6 aquaria arid/airless, 7 asteroid,
/// 8 gas giant or chthonia, 9 ice giant, 11 none.
pub fn surface_class(obj_type: i32, bulk: i32, atmo: i32, volq: i32) -> i32 {
    if obj_type > obj::COMET {
        return 11;
    }
    match bulk {
        bulk::JUPITER | bulk::CHTHONIA => return 8,
        bulk::NEPTUNE => return 9,
        bulk::ASTEROID => return 7,
        _ => {}
    }
    let aq = bulk == bulk::AQUARIA;
    if atmo < 2 {
        return 5 + i32::from(aq);
    }
    match volq {
        0 => 5 + i32::from(aq),
        1 => if aq { 6 } else { 4 },
        2 | 3 => 2 + i32::from(aq),
        4 | 5 => 1,
        _ => 11,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Classes {
    pub bulk: i32,
    pub temp: i32,
    pub atmo: i32,
    pub volq: i32,
    pub size: i32,
    pub surface: i32,
}

impl Classes {
    /// `t_class` is the temperature model's average (`climate::class_temperature`).
    pub fn new(obj_type: i32, bulk: i32, t_class: f32, pressure_atm: f32, mass_earth: f32, ocean: Option<OceanInfo>) -> Self {
        let atmo = atmo_class(bulk, pressure_atm);
        let volq = volq_class(bulk, atmo, ocean);
        Self {
            bulk,
            temp: temp_class(t_class),
            atmo,
            volq,
            size: size_class(bulk, mass_earth),
            surface: surface_class(obj_type, bulk, atmo, volq),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------------------

/// What the generator reads from the body record. Units as SE: km, Earth masses, K, atm.
#[derive(Clone, Debug, Default)]
pub struct GenInput {
    pub seeds: (u32, u32),
    pub obj_type: i32,
    pub classes: Classes,
    pub radius_km: f32,
    pub mass_earth: f32,
    /// Radiative equilibrium temperature (record +0x1248).
    pub t_eq: f32,
    /// Mean surface temperature with greenhouse (+0x1250).
    pub t_surf: f32,
    /// Temperature model average behind the class word (+0x1860).
    pub t_class: f32,
    pub pressure_atm: f32,
    /// Atmosphere scale height and height (km), for the snow line.
    pub atm_scale_height_km: f32,
    pub atm_height_km: f32,
    pub has_clouds: bool,
    /// Ocean level as a fraction of `BumpHeight` above the lowest terrain, if there is an ocean.
    pub ocean_level: Option<f32>,
    pub ocean_depth_km: f32,
    pub bump_height_km: f32,
    /// Any life (record flags 0x1010).
    pub life: bool,
    /// Ice in the composition (volcano temperature 273 K instead of 1100 K).
    pub icy: bool,
    /// Angle between the spin axis and the orbit normal of the body's planet (radians).
    pub obliquity_rad: f32,
    /// `Color` given in the catalog (otherwise generated).
    pub color_given: bool,
    pub rotation_days: f32,
    /// Moons: semi-major axis and the parent's radius (km), parent's ring outer radius (km),
    /// whether the parent is a giant, and whether the moon orbits faster than its parent spins.
    pub moon: Option<MoonInfo>,
    /// Planets with rings of their own (the equatorial-ridge chance).
    pub has_rings: bool,
    /// Stars: SE's class word fields (record +0x1620).
    pub star: Option<StarCode>,
}

/// The fields of SE's star class word the surface generator reads: the luminosity code (bits 11-14:
/// 0 hypergiant, 1 Ia, 2 Ib, 3 II, 4 III, 5 IV, 6 V, 7 VI, 9 carbon, 10 Wolf-Rayet, 11 white dwarf,
/// 12 neutron star), the letter index (bits 7-10: O 0, B 1, A 2, F 3, G 4, K 5, M 6, L 7, T 8,
/// Y 9) and the subclass in tenths (bits 0-6).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StarCode {
    pub lum: u32,
    pub letter: u32,
    pub sub10: u32,
}

impl StarCode {
    pub fn of(class: &crate::star::SpectralClass) -> Self {
        use crate::star::{LumClass, StarKind};
        let lum = match class.kind {
            StarKind::WhiteDwarf => 11,
            StarKind::NeutronStar | StarKind::BlackHole => 12,
            StarKind::WolfRayet => 10,
            StarKind::Carbon => 9,
            _ => match class.lum_class {
                LumClass::Hypergiant => 0,
                LumClass::Ia => 1,
                LumClass::Ib => 2,
                LumClass::II => 3,
                LumClass::III => 4,
                LumClass::IV => 5,
                LumClass::V => 6,
                LumClass::VI => 7,
                LumClass::VII => 11,
            },
        };
        let letter = match class.letter {
            'O' | 'W' => 0,
            'B' => 1,
            'A' => 2,
            'F' => 3,
            'G' => 4,
            'K' => 5,
            'M' | 'C' | 'S' => 6,
            'L' => 7,
            'T' => 8,
            'Y' => 9,
            _ => 4,
        };
        StarCode { lum, letter, sub10: (class.subclass * 10.0).round().clamp(0.0, 99.0) as u32 }
    }
    /// SE's "giant" surfaces (class word & 0x7800 <= 0x2800, or carbon stars): big convection cells
    /// with flows, no spots.
    pub fn giant(&self) -> bool {
        self.lum <= 5 || self.lum == 9
    }
}

/// The generator input of a catalog star: its Teff, radius and mass (Earth masses; one solar mass
/// when the catalog has none), seeded like a body by its names and parent.
pub fn star_input(s: &crate::catalog::StarRecord) -> GenInput {
    use crate::units::{EARTH_MASS_KG, SOLAR_MASS_KG};
    let mass_kg = if s.mass_kg > 0.0 { s.mass_kg } else { SOLAR_MASS_KG };
    GenInput {
        seeds: catalog_seeds(&s.names.join("/"), s.parent.as_deref().unwrap_or("")),
        obj_type: obj::STAR,
        classes: Classes { bulk: -1, ..Default::default() },
        radius_km: s.radius_km.max(1.0) as f32,
        mass_earth: (mass_kg / EARTH_MASS_KG) as f32,
        t_eq: s.teff as f32,
        t_surf: s.teff as f32,
        t_class: s.teff as f32,
        star: Some(StarCode::of(&s.class)),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoonInfo {
    pub a_km: f32,
    pub parent_radius_km: f32,
    pub parent_ring_outer_km: Option<f32>,
    pub parent_giant: bool,
    pub inside_sync_orbit: bool,
}

/// Generated parameters (`p` in slot order; [`UNSET`] where SE leaves a slot unset).
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceGen {
    pub p: [f32; N_PARAMS],
    /// Generated `Color` (linear 0..1) when the catalog has none.
    pub color: Option<[f32; 3]>,
    pub classes: Classes,
}

impl SurfaceGen {
    pub fn get(&self, i: usize) -> Option<f32> {
        let v = self.p[i];
        (v != UNSET).then_some(v)
    }
    /// The value, 0 where unset.
    pub fn v(&self, i: usize) -> f32 {
        self.get(i).unwrap_or(0.0)
    }
    pub fn surf_style(&self) -> f32 {
        self.v(SURF_STYLE)
    }
}

/// Slots the catalog's `Surface` (and `Clouds`) blocks give.
pub fn given_from(surface: Option<&Block>, clouds: Option<&Block>) -> [f32; N_PARAMS] {
    let mut p = [UNSET; N_PARAMS];
    p[0] = 0.0;
    p[1] = 0.0;
    p[2] = 0.0;
    p[LAVA_LEVEL] = UNSET;
    if let Some(s) = surface {
        for (i, k) in KEYS.iter().enumerate() {
            if !k.is_empty() {
                if let Some(v) = s.f64(k) {
                    p[i] = v as f32;
                }
            }
        }
        if let Some(v) = s.f64("canyonFraction") {
            p[CANYONS_FRACTION] = v as f32;
        }
        if let Some(v) = s.f64("Style") {
            p[SURF_STYLE] = v as f32;
        }
        if let Some(r) = s.vec3("Randomize") {
            p[0] = r[0] as f32;
            p[1] = r[1] as f32;
            p[2] = r[2] as f32;
        }
    }
    if let Some(c) = clouds {
        for (i, k) in [(CLOUDS_MAIN_FREQ, "mainFreq"), (CLOUDS_MAIN_OCTAVES, "mainOctaves"), (CLOUDS_COVERAGE, "Coverage")] {
            if let Some(v) = c.f64(k) {
                p[i] = v as f32;
            }
        }
    }
    p
}

fn sat(x: f32) -> f32 {
    // SE's pattern `r = 0; if (0 <= x) { r = 1; if (x <= 1) r = x; }` (NaN -> 0).
    if x >= 0.0 {
        if x <= 1.0 { x } else { 1.0 }
    } else {
        0.0
    }
}

/// HSL to RGB (FUN_140442b00).
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let q = if l > 0.5 { s + l - s * l } else { (s + 1.0) * l };
    if q <= 0.0 {
        return [l, l, 0.0];
    }
    let p = 2.0 * l - q;
    let ch = |t: f32| {
        let t = t - t.floor();
        if t * 6.0 < 1.0 {
            p + (q - p) * 6.0 * t
        } else if t * 2.0 < 1.0 {
            q
        } else if t * 3.0 < 2.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [ch(h + 1.0 / 3.0), ch(h), ch(h - 1.0 / 3.0)]
}

/// Default `craterOctaves`: ceil(log_1.8(R / 0.03 km)) (0.015 km for asteroids), 1..30.
fn crater_octaves_default(bulk: i32, radius_km: f32) -> f32 {
    let base = if bulk == bulk::ASTEROID { 0.015 } else { 0.03f32 };
    let v = ((radius_km / base).ln() / 1.8f32.ln()).ceil();
    if v >= 1.0 {
        v.min(30.0)
    } else {
        1.0
    }
}

/// Fill the unset slots of `given` like SE does for a planet, moon, dwarf body or asteroid
/// (object types 4..9). Giants (bulk > 4) get only their `SurfStyle` and `Randomize`; their
/// stripe and cyclone parameters are not ported.
pub fn generate(inp: &GenInput, given: &[f32; N_PARAMS]) -> SurfaceGen {
    let mut p = *given;
    let orig = *given;
    let un = |v: f32| v == UNSET;
    let mut r = SeRng::new(inp.seeds.0, inp.seeds.1);
    let c = inp.classes;
    let bulk = c.bulk;
    let rad = inp.radius_km;
    let ts = inp.t_surf;

    let crater_oct = if un(p[CRATER_OCTAVES]) { crater_octaves_default(bulk, rad) } else { p[CRATER_OCTAVES] };
    if un(p[DETAIL_SCALE]) {
        p[DETAIL_SCALE] = 4.0;
    }
    // Body colour: random HSL, mostly desaturated.
    let mut color = None;
    if !inp.color_given && bulk <= bulk::AQUARIA && inp.obj_type != obj::STAR {
        let (h, mut s, l, keep_sat) = if bulk == bulk::AQUARIA {
            let h = (r.uniform(30.0) + 12.0) / 360.0;
            let s = (r.uniform(30.0) + 10.0) / 100.0;
            let l = (r.uniform(40.0) + 40.0) / 100.0;
            (h, s, l, if ts < 100.0 { 0.6 } else { 0.35 })
        } else {
            let h = (r.uniform(25.0) + 15.0) / 360.0;
            let s = (r.uniform(20.0) + 10.0) / 100.0;
            let l = (r.uniform(40.0) + 40.0) / 100.0;
            (h, s, l, 0.35)
        };
        if keep_sat <= r.uniform(1.0) {
            s *= 0.05;
        }
        color = Some(hsl_to_rgb(h, s, l));
    }
    if un(p[SURF_STYLE]) {
        p[SURF_STYLE] = r.uniform(1.0);
    }
    if p[0] == 0.0 && p[1] == 0.0 && p[2] == 0.0 {
        for i in 0..3 {
            p[i] = r.uniform(2.0) - 1.0;
        }
    }
    // Both clamps also turn an unset slot into 0, as in SE.
    let done = |mut p: [f32; N_PARAMS]| {
        p[CRATER_OCTAVES] = if p[CRATER_OCTAVES] >= 0.0 { p[CRATER_OCTAVES].min(30.0) } else { 0.0 };
        p[CLOUDS_MAIN_OCTAVES] = if p[CLOUDS_MAIN_OCTAVES] >= 0.0 { p[CLOUDS_MAIN_OCTAVES].min(15.0) } else { 0.0 };
        SurfaceGen { p, color, classes: c }
    };
    if inp.obj_type == obj::STAR {
        star_surface(&mut p, &mut r, inp);
        return done(p);
    }
    if (bulk::NEPTUNE..=bulk::BROWN_DWARF).contains(&bulk) {
        giant_clouds(&mut p, &mut r, inp);
    }
    if !(obj::PLANET..=obj::COMET).contains(&inp.obj_type) || bulk > bulk::AQUARIA {
        return done(p);
    }

    let tropic_lat = sat(inp.obliquity_rad.sin());
    let has_ocean = inp.ocean_level.is_some();
    // Climate bands of vegetation (dry / wet worlds).
    let (forest_min, forest_max, steppe_min, steppe_max, grass_min) =
        if has_ocean { (0.5, 0.6875, 0.3125, 0.5625, 0.625) } else { (0.25, 0.5625, 0.0, 0.3125, 0.5) };
    // Height of the -15 C (or methane-freezing) level, km, for the snow line.
    let freeze = if bulk == bulk::AQUARIA { 90.65 } else { 258.15 };
    let snow_km = if inp.pressure_atm < 0.2 {
        0.0
    } else {
        ((1.0 - freeze / ts) * inp.atm_scale_height_km / 0.2857).min(inp.atm_height_km)
    };
    // Cold-world factor (1 below 213 K, 0 above 333 K; 50..90 K for aquaria).
    let cold = if bulk == bulk::AQUARIA {
        if ts < 50.0 { 1.0 } else if ts <= 90.0 { 1.0 - (ts - 50.0) / 40.0 } else { 0.0 }
    } else if ts < 213.0 {
        1.0
    } else if ts <= 333.0 {
        1.0 - (ts - 213.0) / 120.0
    } else {
        0.0
    };
    let pole_cold = if ts < if bulk == bulk::AQUARIA { 100.0 } else { 353.0 } { 0.9375 } else { 0.4375 };
    // Greenhouse "warmth": log10 pressure, plus heat above 350 K.
    let mut warm = 0.0f32;
    if inp.pressure_atm > 0.0 {
        let l = inp.pressure_atm.log10();
        warm = if ts >= 180.0 { l + l } else { l * 6.0 };
        warm += (ts - 350.0).max(0.0) * inp.pressure_atm * 10.0;
        warm = warm.max(0.0);
    }
    let pole_warm = 0.5625f32;
    if un(p[TROPIC_LATITUDE]) {
        p[TROPIC_LATITUDE] = tropic_lat;
    }
    let lerp_w = |a: f32, b: f32, t: f32| if t < 0.0 { a } else if t > 1.0 { b } else { (b - a) * t + a };
    if un(p[CLIMATE_POLE]) {
        p[CLIMATE_POLE] = lerp_w(pole_cold, pole_warm, warm - 1.0);
    }
    if un(p[CLIMATE_TROPIC]) {
        p[CLIMATE_TROPIC] = lerp_w(0.3125, pole_warm, warm);
    }
    if un(p[CLIMATE_EQUATOR]) {
        p[CLIMATE_EQUATOR] = lerp_w(0.6875, pole_warm, warm);
    }
    if un(p[TROPIC_WIDTH]) {
        let t = warm * warm;
        p[TROPIC_WIDTH] = if t < 0.0 { 0.07 } else if t > 1.0 { 0.5 } else { t * 0.43 + 0.07 };
    }
    let no_life = -1.0;
    let lv = |v: f32| if inp.life { v } else { no_life };
    for (i, v) in [
        (CLIMATE_STEPPE_MIN, steppe_min),
        (CLIMATE_STEPPE_MAX, steppe_max),
        (CLIMATE_FOREST_MIN, forest_min),
        (CLIMATE_FOREST_MAX, forest_max),
        (CLIMATE_GRASS_MIN, grass_min),
        (CLIMATE_GRASS_MAX, 0.8125),
    ] {
        if un(p[i]) {
            p[i] = lv(v);
        }
    }
    if un(p[HUMIDITY]) {
        p[HUMIDITY] = if !has_ocean {
            r.uniform(0.4) + 0.1
        } else {
            let h = sat(inp.ocean_depth_km / 5.0);
            sat(r.uniform(0.2) + h)
        };
    }
    if un(p[BEACH_WIDTH]) {
        let g = r.gauss(1.0, 0.3);
        p[BEACH_WIDTH] = (if g < 0.1 { 0.1 } else if g > 2.0 { 2.0 } else { g }) * 0.001;
    }
    // Equatorial ridge chance: airless moons close to their parent's rings, big moons, ringed planets.
    let mut ridge = false;
    let mut near_rings = false;
    if inp.pressure_atm < 0.005 {
        let mut chance = 0.0f32;
        if inp.obj_type == obj::MOON || inp.obj_type == obj::DWARF_MOON {
            if let Some(m) = inp.moon {
                if let Some(ring) = m.parent_ring_outer_km {
                    let x = sat((m.a_km / ring - 5.0) * -0.25);
                    chance = 0.6 * (3.0 - (x + x)) * x * x;
                    near_rings = inp.obj_type == obj::DWARF_MOON && chance > 0.0;
                }
            }
            if rad >= 500.0 {
                chance += 0.12;
            }
        } else if inp.has_rings {
            chance = 0.06;
        }
        ridge = r.uniform(1.0) < chance;
    }
    if inp.has_clouds {
        clouds(&mut p, &mut r, inp);
    }

    // Per surface class. Constants are the values SE's registers hold at its switch.
    let case_seal = |p: &mut [f32; N_PARAMS]| p[SEA_LEVEL] = inp.ocean_level.unwrap_or(-1.0);
    match c.surface {
        1 | 2 => {
            let oceanic = c.surface == 1;
            if un(p[MAIN_FREQ]) {
                p[MAIN_FREQ] = r.tnormal(0.9, 0.3, 0.2, 1.5);
            }
            if un(p[VENUS_FREQ]) {
                p[VENUS_FREQ] = r.tnormal(1.0, 0.4, 0.2, 1.8);
            }
            if un(p[VENUS_MAGN]) {
                let roll = r.uniform(1.0);
                p[VENUS_MAGN] = if (oceanic && roll < 0.66) || (!oceanic && roll < 0.2) {
                    let v = r.tnormal(0.4, 0.1, 0.2, 0.6);
                    v + v
                } else {
                    0.0
                };
            }
            case_seal(&mut p);
            if oceanic && un(p[ICECAP_HEIGHT]) {
                let v = r.tnormal(5.0, 2.0, 2.0, 8.0);
                p[ICECAP_HEIGHT] = v / inp.bump_height_km + p[SEA_LEVEL];
            }
            if un(p[COLOR_DIST_MAGN]) {
                p[COLOR_DIST_MAGN] = r.tnormal(0.06, 0.02, 0.04, 0.1);
            }
            if un(p[COLOR_DIST_FREQ]) {
                p[COLOR_DIST_FREQ] = r.tnormal(700.0, 100.0, 500.0, 900.0);
            }
            set0(&mut p, DRIVEN_DARKENING);
            if un(p[TERRACE_PROB]) {
                p[TERRACE_PROB] = r.tnormal(0.2, 0.2, 0.1, 1.0);
            }
            if oceanic {
                set0(&mut p, EROSION);
                set0(&mut p, MARE_FREQ);
                set0(&mut p, MARE_DENSITY);
            } else {
                if un(p[EROSION]) {
                    p[EROSION] = r.tnormal(0.12, 0.01, 0.1, 0.14);
                }
                if un(p[MARE_FREQ]) {
                    p[MARE_FREQ] = r.tnormal(1.8, 1.0, 0.8, 2.5) * 3.0;
                }
                if un(p[MARE_DENSITY]) {
                    p[MARE_DENSITY] = r.tnormal(0.18, 0.01, 0.2, 0.3) * 0.3;
                }
            }
            if un(p[MONTES_MAGN]) {
                p[MONTES_MAGN] = r.tnormal(0.25, 0.1, 0.2, 0.35) * 0.75;
            }
            if un(p[MONTES_FREQ]) {
                p[MONTES_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * 0.75;
            }
            if un(p[MONTES_FRACTION]) {
                p[MONTES_FRACTION] = r.uniform(1.0);
            }
            if un(p[MONTES_SPIKY]) {
                p[MONTES_SPIKY] = r.tnormal(1.0, 0.1, 0.5, 1.0);
            }
            if un(p[DUNES_MAGN]) {
                p[DUNES_MAGN] = r.tnormal(0.05, 0.01, 0.005, 0.1);
            }
            if un(p[DUNES_FREQ]) {
                p[DUNES_FREQ] = r.tnormal(50.0, 10.0, 30.0, 80.0);
            }
            if un(p[DUNES_FRACTION]) {
                p[DUNES_FRACTION] = r.uniform(1.0);
            }
            if un(p[HILLS_MAGN]) {
                p[HILLS_MAGN] = if oceanic { r.tnormal(0.25, 0.1, 0.01, 0.35) } else { r.tnormal(0.25, 0.1, 0.2, 0.3) } * 0.5;
            }
            if un(p[HILLS_FREQ]) {
                p[HILLS_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * if oceanic { 1.8 } else { 0.257_142_8 };
            }
            if un(p[HILLS_FRACTION]) {
                p[HILLS_FRACTION] = r.uniform(1.0);
            }
            if un(p[HILLS2_FRACTION]) {
                p[HILLS2_FRACTION] = r.uniform(1.0);
            }
            canyons_std(&mut p, &mut r);
            if un(p[CRACKS_MAGN]) {
                p[CRACKS_MAGN] = r.tnormal(0.1, 0.03, 0.06, 0.14);
            }
            if un(p[CRACKS_FREQ]) {
                p[CRACKS_FREQ] = r.tnormal(0.5, 0.1, 0.3, 0.8);
            }
            if oceanic {
                if un(p[CRACKS_OCTAVES]) {
                    let n = r.int(4) + 1;
                    p[CRACKS_OCTAVES] = ((inp.mass_earth.log10() + 3.0) * n as f32 * 0.25).floor().max(0.0);
                }
                if un(p[CRATER_MAGN]) {
                    p[CRATER_MAGN] = r.tnormal(0.2, 0.01, 0.18, 0.22);
                }
                if un(p[CRATER_FREQ]) {
                    p[CRATER_FREQ] = r.tnormal(50.0, 5.0, 40.0, 60.0);
                }
                set0(&mut p, CRATER_DENSITY);
                set0(&mut p, CRATER_OCTAVES);
                if un(p[VOLCANO_MAGN]) {
                    p[VOLCANO_MAGN] = r.tnormal(0.7, 0.05, 0.5, 0.9);
                }
                if un(p[VOLCANO_FREQ]) {
                    p[VOLCANO_FREQ] = r.tnormal(0.7, 0.1, 0.4, 2.0);
                }
                set0(&mut p, VOLCANO_DENSITY);
                set0(&mut p, VOLCANO_OCTAVES);
                set0(&mut p, VOLCANO_FLOWS);
                set0(&mut p, RIVERS_MAGN);
                if un(p[RIVERS_FREQ]) {
                    p[RIVERS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
                }
            } else {
                set0(&mut p, CRACKS_OCTAVES);
                if un(p[CRATER_MAGN]) {
                    p[CRATER_MAGN] = r.tnormal(1.2, 0.1, 1.0, 1.4);
                }
                if un(p[CRATER_FREQ]) {
                    p[CRATER_FREQ] = r.tnormal(16.0, 3.0, 10.0, 22.0);
                }
                if un(p[CRATER_DENSITY]) {
                    p[CRATER_DENSITY] = r.tnormal(0.9, 0.1, 0.7, 1.0);
                }
                if un(p[CRATER_OCTAVES]) {
                    p[CRATER_OCTAVES] = crater_oct;
                }
                if un(p[VOLCANO_MAGN]) {
                    p[VOLCANO_MAGN] = r.tnormal(0.7, 0.05, 0.5, 0.9);
                }
                if un(p[VOLCANO_FREQ]) {
                    p[VOLCANO_FREQ] = r.tnormal(0.7, 0.1, 0.4, 2.0);
                }
                if un(p[VOLCANO_DENSITY]) {
                    p[VOLCANO_DENSITY] = r.tnormal(0.2, 0.03, 0.05, 0.5);
                }
                if un(p[VOLCANO_OCTAVES]) {
                    p[VOLCANO_OCTAVES] = 3.0;
                }
                if un(p[VOLCANO_FLOWS]) {
                    p[VOLCANO_FLOWS] = r.tnormal(0.7, 0.2, 0.2, 1.0);
                }
                if un(p[RIVERS_MAGN]) {
                    p[RIVERS_MAGN] = r.tnormal(60.0, 5.0, 35.0, 75.0);
                }
                if un(p[RIVERS_FREQ]) {
                    p[RIVERS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
                }
            }
            rivers_rifts_tail(&mut p, &mut r, 0.0);
        }
        3 => {
            if un(p[MAIN_FREQ]) {
                p[MAIN_FREQ] = r.tnormal(1.8, 0.2, 1.5, 2.1);
            }
            if un(p[VENUS_FREQ]) {
                p[VENUS_FREQ] = r.tnormal(1.0, 0.4, 0.2, 1.8);
            }
            if un(p[VENUS_MAGN]) {
                p[VENUS_MAGN] = if 0.25 <= r.uniform(1.0) {
                    -1.0
                } else {
                    let v = r.tnormal(0.4, 0.1, 0.2, 0.6);
                    v + v
                };
            }
            case_seal(&mut p);
            if un(p[COLOR_DIST_MAGN]) {
                p[COLOR_DIST_MAGN] = r.tnormal(0.06, 0.02, 0.04, 0.1);
            }
            if un(p[COLOR_DIST_FREQ]) {
                p[COLOR_DIST_FREQ] = r.tnormal(700.0, 100.0, 500.0, 900.0);
            }
            set0(&mut p, DRIVEN_DARKENING);
            if un(p[TERRACE_PROB]) {
                p[TERRACE_PROB] = r.tnormal(0.2, 0.2, 0.1, 1.0);
            }
            if un(p[EROSION]) {
                p[EROSION] = r.tnormal(0.12, 0.01, 0.1, 0.14);
            }
            if un(p[MARE_FREQ]) {
                p[MARE_FREQ] = r.tnormal(1.8, 1.0, 0.8, 2.5);
            }
            if un(p[MARE_DENSITY]) {
                p[MARE_DENSITY] = r.tnormal(0.18, 0.01, 0.2, 0.3);
            }
            let mut sp = 0.1;
            if un(p[MONTES_MAGN]) {
                p[MONTES_MAGN] = r.tnormal(0.25, 0.1, 0.2, 0.35) * 0.75;
                p[MONTES_MAGN] *= sat((inp.mass_earth.log10() + 2.0) * 0.5);
                sp = 0.1;
            }
            if un(p[MONTES_FREQ]) {
                p[MONTES_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * 0.75;
            }
            if un(p[MONTES_FRACTION]) {
                p[MONTES_FRACTION] = r.uniform(1.0);
            }
            if un(p[MONTES_SPIKY]) {
                p[MONTES_SPIKY] = r.tnormal(1.0, sp, 0.5, 1.0);
            }
            if un(p[DUNES_MAGN]) {
                p[DUNES_MAGN] = r.tnormal(0.05, 0.01, 0.005, sp);
            }
            if un(p[DUNES_FREQ]) {
                p[DUNES_FREQ] = r.tnormal(50.0, 10.0, 30.0, 80.0);
            }
            if un(p[DUNES_FRACTION]) {
                p[DUNES_FRACTION] = r.uniform(1.0);
            }
            if un(p[HILLS_MAGN]) {
                p[HILLS_MAGN] = r.tnormal(0.25, sp, 0.2, 0.3) * 0.5;
            }
            if un(p[HILLS_FREQ]) {
                p[HILLS_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * 1.8;
            }
            if un(p[HILLS_FRACTION]) {
                p[HILLS_FRACTION] = r.uniform(1.0);
            }
            if un(p[HILLS2_FRACTION]) {
                p[HILLS2_FRACTION] = r.uniform(1.0);
            }
            canyons_std(&mut p, &mut r);
            if un(p[CRACKS_MAGN]) {
                p[CRACKS_MAGN] = r.tnormal(0.05, 0.02, 0.02, 0.08);
            }
            if un(p[CRACKS_FREQ]) {
                p[CRACKS_FREQ] = r.tnormal(2.5, 1.0, 1.0, 4.0);
            }
            if un(p[CRACKS_OCTAVES]) {
                let u = r.uniform(6.0);
                p[CRACKS_OCTAVES] = ((inp.mass_earth.log10() + 3.0) * (u + 4.0) * 0.25).floor().max(0.0);
            }
            craters_icy(&mut p, &mut r, crater_oct, sp, 3.0);
            if un(p[VOLCANO_MAGN]) {
                p[VOLCANO_MAGN] = r.tnormal(0.6, 0.05, 0.4, 0.9);
            }
            if un(p[VOLCANO_FREQ]) {
                p[VOLCANO_FREQ] = r.tnormal(0.7, 0.1, 0.4, 2.0);
            }
            if un(p[VOLCANO_DENSITY]) {
                p[VOLCANO_DENSITY] = r.tnormal(0.2, 0.03, 0.05, 0.5);
            }
            if un(p[VOLCANO_OCTAVES]) {
                p[VOLCANO_OCTAVES] = 3.0;
            }
            if un(p[VOLCANO_FLOWS]) {
                p[VOLCANO_FLOWS] = r.tnormal(0.7, 0.2, 0.2, 1.0);
            }
            if un(p[RIVERS_MAGN]) {
                p[RIVERS_MAGN] = r.tnormal(60.0, 5.0, 35.0, 75.0);
            }
            if un(p[RIVERS_FREQ]) {
                p[RIVERS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
            }
            rivers_rifts_tail(&mut p, &mut r, 0.0);
        }
        4 => {
            if un(p[MAIN_FREQ]) {
                p[MAIN_FREQ] = r.tnormal(0.9, 0.3, 0.2, 1.5);
            }
            if un(p[VENUS_FREQ]) {
                p[VENUS_FREQ] = r.tnormal(1.0, 0.4, 0.2, 1.8);
            }
            if un(p[VENUS_MAGN]) {
                p[VENUS_MAGN] = if r.uniform(1.0) < 0.33 {
                    let v = r.tnormal(0.4, 0.1, 0.2, 0.6);
                    v + v
                } else {
                    0.0
                };
            }
            if un(p[SEA_LEVEL]) {
                p[SEA_LEVEL] = r.tnormal(0.1, 0.05, 0.2, 0.0);
            }
            if un(p[COLOR_DIST_MAGN]) {
                p[COLOR_DIST_MAGN] = r.tnormal(0.06, 0.02, 0.04, 0.1);
            }
            if un(p[COLOR_DIST_FREQ]) {
                p[COLOR_DIST_FREQ] = r.tnormal(700.0, 100.0, 500.0, 900.0);
            }
            set0(&mut p, DRIVEN_DARKENING);
            if un(p[TERRACE_PROB]) {
                p[TERRACE_PROB] = r.tnormal(0.2, 0.2, 0.1, 1.0);
            }
            set0(&mut p, EROSION);
            if un(p[MARE_FREQ]) {
                p[MARE_FREQ] = r.tnormal(1.8, 1.0, 0.8, 2.5);
            }
            if un(p[MARE_DENSITY]) {
                p[MARE_DENSITY] = r.tnormal(0.18, 0.01, 0.2, 0.3);
            }
            if un(p[MONTES_MAGN]) {
                p[MONTES_MAGN] = r.tnormal(0.25, 0.1, 0.2, 0.35) * 0.75;
            }
            if un(p[MONTES_FREQ]) {
                p[MONTES_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * 0.75;
            }
            if un(p[MONTES_FRACTION]) {
                p[MONTES_FRACTION] = r.uniform(1.0);
            }
            if un(p[MONTES_SPIKY]) {
                p[MONTES_SPIKY] = r.tnormal(1.0, 0.1, 0.5, 1.0);
            }
            if un(p[DUNES_MAGN]) {
                p[DUNES_MAGN] = r.tnormal(0.06, 0.01, 0.02, 0.1);
            }
            if un(p[DUNES_FREQ]) {
                p[DUNES_FREQ] = r.tnormal(50.0, 10.0, 30.0, 80.0);
            }
            if un(p[DUNES_FRACTION]) {
                p[DUNES_FRACTION] = r.uniform(1.0);
            }
            if un(p[HILLS_MAGN]) {
                p[HILLS_MAGN] = r.tnormal(0.25, 0.1, 0.2, 0.35) * 0.5;
            }
            if un(p[HILLS_FREQ]) {
                p[HILLS_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * 1.8;
            }
            set0(&mut p, HILLS_FRACTION);
            set0(&mut p, HILLS2_FRACTION);
            if un(p[CANYONS_MAGN]) {
                p[CANYONS_MAGN] = r.tnormal(0.04, 0.02, 0.0, 0.07);
            }
            if un(p[CANYONS_FREQ]) {
                p[CANYONS_FREQ] = r.tnormal(50.0, 5.0, 40.0, 60.0) * 3.0;
            }
            if un(p[CANYONS_FRACTION]) {
                p[CANYONS_FRACTION] = r.int(1) as f32;
            }
            if un(p[CRACKS_MAGN]) {
                p[CRACKS_MAGN] = r.tnormal(0.1, 0.03, 0.06, 0.14);
            }
            if un(p[CRACKS_FREQ]) {
                p[CRACKS_FREQ] = r.tnormal(0.5, 0.1, 0.3, 0.8);
            }
            set0(&mut p, CRACKS_OCTAVES);
            if un(p[CRATER_MAGN]) {
                p[CRATER_MAGN] = r.tnormal(1.2, 0.1, 1.0, 1.4);
            }
            if un(p[CRATER_FREQ]) {
                p[CRATER_FREQ] = r.tnormal(16.0, 3.0, 10.0, 22.0);
            }
            if un(p[CRATER_DENSITY]) {
                p[CRATER_DENSITY] = r.tnormal(0.9, 0.1, 0.7, 1.0);
            }
            if un(p[CRATER_OCTAVES]) {
                p[CRATER_OCTAVES] = crater_oct;
            }
            if un(p[VOLCANO_MAGN]) {
                p[VOLCANO_MAGN] = r.tnormal(0.7, 0.05, 0.5, 0.9);
            }
            if un(p[VOLCANO_FREQ]) {
                p[VOLCANO_FREQ] = r.tnormal(0.7, 0.1, 0.4, 2.0);
            }
            if un(p[VOLCANO_DENSITY]) {
                p[VOLCANO_DENSITY] = r.tnormal(0.2, 0.03, 0.05, 0.5);
            }
            if un(p[VOLCANO_OCTAVES]) {
                p[VOLCANO_OCTAVES] = 3.0;
            }
            if un(p[VOLCANO_FLOWS]) {
                p[VOLCANO_FLOWS] = r.tnormal(0.7, 0.2, 0.2, 1.0);
            }
            set0(&mut p, RIVERS_MAGN);
            if un(p[RIVERS_FREQ]) {
                p[RIVERS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
            }
            if un(p[RIVERS_SIN]) {
                p[RIVERS_SIN] = r.tnormal(6.0, 1.0, 3.0, 9.0);
            }
            if un(p[RIFTS_MAGN]) {
                p[RIFTS_MAGN] = r.tnormal(60.0, 5.0, 35.0, 75.0);
            }
            if un(p[RIFTS_FREQ]) {
                p[RIFTS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
            }
            if un(p[RIFTS_SIN]) {
                p[RIFTS_SIN] = r.tnormal(6.0, 1.0, 3.0, 9.0);
            }
        }
        5 | 6 => {
            let icy = c.surface == 6;
            if un(p[MAIN_FREQ]) {
                p[MAIN_FREQ] = r.tnormal(1.8, 0.2, 1.5, 2.1);
            }
            if un(p[VENUS_FREQ]) {
                p[VENUS_FREQ] = r.tnormal(1.0, 0.4, 0.2, 1.8);
            }
            if un(p[VENUS_MAGN]) {
                p[VENUS_MAGN] = if !icy || 0.25 <= r.uniform(1.0) { 0.0 } else { r.tnormal(0.2, 0.05, 0.1, 0.3) };
            }
            if un(p[SEA_LEVEL]) {
                p[SEA_LEVEL] = r.tnormal(0.2, 0.05, 0.0, 0.5);
            }
            if un(p[COLOR_DIST_MAGN]) {
                p[COLOR_DIST_MAGN] = r.tnormal(0.06, 0.02, 0.04, 0.1);
            }
            if un(p[COLOR_DIST_FREQ]) {
                p[COLOR_DIST_FREQ] = r.tnormal(700.0, 100.0, 500.0, 900.0);
            }
            if un(p[DRIVEN_DARKENING]) {
                p[DRIVEN_DARKENING] = if icy { driven_darkening(inp) } else { 0.0 };
            }
            if un(p[TERRACE_PROB]) {
                p[TERRACE_PROB] = r.tnormal(0.2, 0.2, 0.1, 1.0);
            }
            set0(&mut p, EROSION);
            if un(p[MARE_FREQ]) {
                p[MARE_FREQ] = r.tnormal(1.8, 0.3, 1.3, 2.3);
            }
            if un(p[MARE_DENSITY]) {
                p[MARE_DENSITY] = r.tnormal(0.15, 0.03, 0.1, 0.2);
            }
            if un(p[MONTES_MAGN]) {
                p[MONTES_MAGN] = r.tnormal(0.25, 0.1, 0.2, 0.35) * 0.75;
            }
            if un(p[MONTES_FREQ]) {
                p[MONTES_FREQ] = r.tnormal(360.0, 120.0, 240.0, 480.0) * 0.75;
            }
            if un(p[MONTES_FRACTION]) {
                p[MONTES_FRACTION] = r.uniform(1.0);
            }
            if un(p[MONTES_SPIKY]) {
                p[MONTES_SPIKY] = r.tnormal(1.0, 0.1, 0.5, 1.0);
            }
            if un(p[DUNES_MAGN]) {
                p[DUNES_MAGN] = r.tnormal(0.04, 0.01, 0.02, 0.06);
            }
            if un(p[DUNES_FREQ]) {
                p[DUNES_FREQ] = r.tnormal(50.0, 10.0, 30.0, 80.0);
            }
            if un(p[DUNES_FRACTION]) {
                p[DUNES_FRACTION] = r.uniform(1.0);
            }
            if un(p[HILLS_MAGN]) {
                p[HILLS_MAGN] = r.tnormal(0.125, 0.02, 0.1, 0.15);
            }
            if un(p[HILLS_FREQ]) {
                p[HILLS_FREQ] = r.tnormal(650.0, 80.0, 500.0, 800.0) * if icy { 0.25 } else { 1.0 };
            }
            if un(p[HILLS_FRACTION]) {
                p[HILLS_FRACTION] = r.uniform(1.0);
            }
            if icy {
                if un(p[HILLS2_FRACTION]) {
                    p[HILLS2_FRACTION] = r.uniform(0.3);
                }
            } else {
                set0(&mut p, HILLS2_FRACTION);
            }
            if un(p[CANYONS_MAGN]) {
                p[CANYONS_MAGN] = r.tnormal(0.5, 0.1, 0.05, 0.95);
            }
            if un(p[CANYONS_FREQ]) {
                p[CANYONS_FREQ] = r.tnormal(2.0, 0.3, 0.7, 3.3);
            }
            if icy {
                set0(&mut p, CANYONS_FRACTION);
            } else if un(p[CANYONS_FRACTION]) {
                p[CANYONS_FRACTION] = r.uniform(1.0);
            }
            if un(p[CRACKS_MAGN]) {
                p[CRACKS_MAGN] = r.tnormal(0.05, 0.02, 0.02, 0.08);
            }
            if un(p[CRACKS_FREQ]) {
                p[CRACKS_FREQ] = r.tnormal(2.5, 1.0, 1.0, 4.0);
            }
            if icy {
                if un(p[CRACKS_OCTAVES]) {
                    let u = r.uniform(6.0);
                    p[CRACKS_OCTAVES] = ((inp.mass_earth.log10() + 3.0) * (u + 4.0) * 0.25).floor().max(0.0);
                }
            } else {
                set0(&mut p, CRACKS_OCTAVES);
            }
            craters_icy(&mut p, &mut r, crater_oct, 0.1, 3.0);
            if un(p[VOLCANO_MAGN]) {
                p[VOLCANO_MAGN] = r.tnormal(0.6, 0.05, 0.4, 0.9);
            }
            if un(p[VOLCANO_FREQ]) {
                p[VOLCANO_FREQ] = r.tnormal(0.7, 0.1, 0.4, 2.0);
            }
            if un(p[VOLCANO_DENSITY]) {
                p[VOLCANO_DENSITY] = r.tnormal(0.2, 0.03, 0.05, 0.5);
            }
            if un(p[VOLCANO_OCTAVES]) {
                p[VOLCANO_OCTAVES] = 3.0;
            }
            if un(p[VOLCANO_FLOWS]) {
                p[VOLCANO_FLOWS] = r.tnormal(0.25, 0.1, 0.0, 0.5);
            }
            if icy {
                set0(&mut p, RIVERS_MAGN);
                if un(p[RIVERS_FREQ]) {
                    p[RIVERS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
                }
                if un(p[RIVERS_SIN]) {
                    p[RIVERS_SIN] = r.tnormal(6.0, 1.0, 3.0, 9.0);
                }
                set0(&mut p, RIFTS_MAGN);
                if un(p[RIFTS_FREQ]) {
                    p[RIFTS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
                }
                if un(p[RIFTS_SIN]) {
                    p[RIFTS_SIN] = r.tnormal(6.0, 1.0, 3.0, 9.0);
                }
            } else {
                if un(p[RIVERS_MAGN]) {
                    p[RIVERS_MAGN] = r.tnormal(60.0, 5.0, 35.0, 75.0);
                }
                if un(p[RIVERS_FREQ]) {
                    p[RIVERS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
                }
                if un(p[RIVERS_SIN]) {
                    p[RIVERS_SIN] = r.tnormal(6.0, 1.0, 3.0, 9.0);
                }
                if un(p[RIFTS_MAGN]) {
                    p[RIFTS_MAGN] = r.tnormal(60.0, 5.0, 35.0, 75.0);
                }
                if un(p[RIFTS_FREQ]) {
                    p[RIFTS_FREQ] = r.tnormal(3.0, 0.7, 1.0, 5.0);
                }
                if un(p[RIFTS_SIN]) {
                    p[RIFTS_SIN] = r.tnormal(6.0, 1.0, 3.0, 9.0);
                }
            }
        }
        7 => {
            if un(p[MAIN_FREQ]) {
                p[MAIN_FREQ] = r.tnormal(0.0512, 0.01792, 0.0256, 0.0768) * 10.0;
            }
            if un(p[VENUS_FREQ]) {
                p[VENUS_FREQ] = r.tnormal(0.6, 0.05, 0.2, 1.0);
            }
            if un(p[VENUS_MAGN]) {
                p[VENUS_MAGN] = r.tnormal(0.66, 0.1, 0.33, 1.0);
                if near_rings {
                    p[VENUS_MAGN] *= sat((rad / 30.0) * (rad / 30.0));
                }
            }
            if un(p[COLOR_DIST_MAGN]) {
                p[COLOR_DIST_MAGN] = r.tnormal(0.7, 0.5, 0.0, 1.0);
            }
            if un(p[COLOR_DIST_FREQ]) {
                p[COLOR_DIST_FREQ] = r.tnormal(3.5, 1.6, 0.05, 5.7);
            }
            set0(&mut p, DRIVEN_DARKENING);
            if un(p[TERRACE_PROB]) {
                p[TERRACE_PROB] = r.tnormal(0.2, 0.2, 0.1, 1.0);
            }
            set0(&mut p, EROSION);
            set0(&mut p, MARE_DENSITY);
            if un(p[MONTES_MAGN]) {
                p[MONTES_MAGN] = r.tnormal(0.5, 0.1, 0.2, 0.7);
            }
            if un(p[MONTES_FREQ]) {
                p[MONTES_FREQ] = r.tnormal(3.0, 0.5, 1.0, 6.5);
            }
            if un(p[MONTES_FRACTION]) {
                p[MONTES_FRACTION] = r.tnormal(0.55, 0.2, 0.05, 0.95);
            }
            if un(p[MONTES_SPIKY]) {
                p[MONTES_SPIKY] = r.tnormal(1.0, 0.1, 0.5, 1.0);
            }
            if un(p[HILLS_MAGN]) {
                p[HILLS_MAGN] = if rad <= 250.0 || r.uniform(1.0) > 0.01 { 0.0 } else { r.tnormal(1.0, 0.15, 0.5, 1.5) };
            }
            if un(p[HILLS_FREQ]) {
                p[HILLS_FREQ] = r.tnormal(1.0, 0.2, 0.4, 1.6) * 15.0;
            }
            if un(p[HILLS_FRACTION]) {
                p[HILLS_FRACTION] = r.tnormal(1.0, 0.2, 0.5, 1.5) * 0.64;
            }
            if un(p[CRATER_MAGN]) {
                p[CRATER_MAGN] = r.tnormal(1.2, 0.1, 0.5, 1.5) * 0.2;
            }
            if un(p[CRATER_FREQ]) {
                p[CRATER_FREQ] = r.tnormal(16.0, 3.0, 10.0, 20.0) * 0.014;
            }
            if un(p[CRATER_DENSITY]) {
                p[CRATER_DENSITY] = r.tnormal(0.9, 0.1, 1.1, 0.5);
            }
            if un(p[CRATER_OCTAVES]) {
                p[CRATER_OCTAVES] = crater_oct;
            }
            set0(&mut p, VOLCANO_FLOWS);
        }
        _ => {}
    }

    // Equatorial ridge.
    if ridge {
        if bulk == bulk::ASTEROID {
            set_tn(&mut p, &mut r, EQRIDGE_MAGN, (0.4, 0.05, 0.1, 0.6));
            set_tn(&mut p, &mut r, EQRIDGE_WIDTH, (0.12, 0.02, 0.08, 0.2));
            set_tn(&mut p, &mut r, EQRIDGE_MOD_MAGN, (0.5, 0.05, 0.0, 1.0));
            set_tn(&mut p, &mut r, EQRIDGE_MOD_FREQ, (0.25, 0.07, 0.05, 1.0));
        } else {
            set_tn(&mut p, &mut r, EQRIDGE_MAGN, (0.4, 0.05, 0.1, 0.6));
            set_tn(&mut p, &mut r, EQRIDGE_WIDTH, (0.025, 0.005, 0.01, 0.04));
            set_tn(&mut p, &mut r, EQRIDGE_MOD_MAGN, (2.5, 0.5, 0.0, 5.0));
            set_tn(&mut p, &mut r, EQRIDGE_MOD_FREQ, (10.0, 1.5, 4.0, 20.0));
        }
    }

    // Snow line from the freezing height.
    if un(p[SNOW_LEVEL]) {
        p[SNOW_LEVEL] = 2.0;
        if snow_km != 0.0 {
            let v = snow_km.max(0.0) / inp.bump_height_km + p[SEA_LEVEL];
            let mut s = v;
            if v < 0.85 {
                let x = (p[SEA_LEVEL] - 0.3) / 0.3;
                s = if x < 0.0 { 0.85 } else if x > 1.0 { v } else { (v - 0.85) * x + 0.85 };
            }
            if s < 0.999 {
                p[SNOW_LEVEL] = s;
            }
        }
    }
    if un(p[HEIGHT_TEMP_GRAD]) {
        p[HEIGHT_TEMP_GRAD] = 0.9375 - p[CLIMATE_TROPIC];
    }
    // Polar caps.
    let mut cap = p[ICECAP_LATITUDE];
    if un(cap) {
        if c.atmo < 2 {
            let lat = (r.uniform(0.6) + 0.4 + p[TROPIC_LATITUDE]).clamp(0.25, 1.0);
            cap = (2.0 - lat) * sat((ts - 200.0) / 30.0) + lat;
        } else if p[CLIMATE_POLE] < 0.9375 {
            cap = 2.0;
        } else {
            let s = sat(p[SEA_LEVEL] + p[SEA_LEVEL]);
            cap = 1.0 - ((1.0 - p[TROPIC_LATITUDE]) - p[TROPIC_WIDTH]) * s * cold;
        }
        p[ICECAP_LATITUDE] = cap;
    }
    if un(p[ICECAP_HEIGHT]) {
        let s = sat(1.0 - p[SEA_LEVEL]);
        p[ICECAP_HEIGHT] = if cap > 1.0 {
            r.tnormal(3.0, 0.7, 1.5, 5.0) / 5.0 * s
        } else {
            let k = sat(1.0 - cap);
            r.tnormal(1.2, 0.5, 0.1, 2.0) * 0.5 * s * k
        };
    }
    // Frequencies scale with size.
    let (f_most, f_crater, f_montes) = if bulk == bulk::ASTEROID {
        let f = (rad / 80.0) * (rad / 80.0);
        (f, f.max(1.0), 1.0)
    } else {
        let f = rad.max(80.0) / 5700.0;
        (f, f, f)
    };
    let f_mare = sat((rad - 600.0) / 2400.0);
    let fresh = |i: usize| orig[i] == UNSET;
    for (i, f) in [
        (COLOR_DIST_FREQ, f_most),
        (DUNES_FREQ, f_most),
        (HILLS_FREQ, f_most),
        (MONTES_FREQ, f_montes),
        (CANYONS_FREQ, f_most),
        (CRATER_FREQ, f_crater),
        (CRACKS_FREQ, f_most),
        (MARE_FREQ, f_mare),
    ] {
        if !un(p[i]) && fresh(i) {
            p[i] *= f;
        }
    }
    // Maria only on low-gravity bodies, and not under the sea.
    if !un(p[MARE_DENSITY]) && fresh(MARE_DENSITY) {
        let g = inp.mass_earth / ((rad / 6371.0) * (rad / 6371.0));
        let x = sat((g - 0.7) / -0.2);
        p[MARE_DENSITY] *= (3.0 - (x + x)) * x * x;
    }
    if !un(p[CRATER_DENSITY]) && fresh(CRATER_DENSITY) && inp.ocean_level.is_some() {
        p[CRATER_DENSITY] *= sat((0.5 - p[SEA_LEVEL]) * 2.0);
        if !un(p[MARE_DENSITY]) {
            p[MARE_DENSITY] *= sat((0.2 - p[SEA_LEVEL]) / 0.2);
        }
    }
    // Atmosphere erodes craters and thickens clouds.
    let mut pf = 0.0;
    if inp.pressure_atm > 0.0 {
        pf = (inp.pressure_atm.max(0.001).log10() + 3.0) / 3.0;
        if !un(p[CLOUDS_COVERAGE]) && fresh(CLOUDS_COVERAGE) && inp.has_clouds {
            let mut cv = sat(pf) * sat(pf) * p[CLOUDS_COVERAGE];
            p[CLOUDS_COVERAGE] = cv;
            if (2..=5).contains(&c.volq) {
                cv *= (p[SEA_LEVEL] + p[SEA_LEVEL]).min(1.0);
                p[CLOUDS_COVERAGE] = cv;
            }
            let roll = r.uniform(1.0);
            if bulk == bulk::AQUARIA {
                if roll < 0.66 && inp.pressure_atm > 1.0 {
                    p[CLOUDS_COVERAGE] = 1.0;
                }
            } else if c.volq == 1 && roll < 0.66 && inp.pressure_atm > 40.0 && ts > 400.0 {
                p[CLOUDS_COVERAGE] = 1.0;
            }
            if p[CLOUDS_COVERAGE] < 0.1 {
                p[CLOUDS_COVERAGE] = 0.1;
            }
        }
    }
    let lava = sat(p[LAVA_LEVEL]);
    let k = (1.0 - lava * 0.9) * sat(1.0 - pf);
    if !un(p[CRATER_DENSITY]) && fresh(CRATER_DENSITY) {
        p[CRATER_DENSITY] *= k;
    }
    if !un(p[CRATER_OCTAVES]) && fresh(CRATER_OCTAVES) {
        p[CRATER_OCTAVES] *= k;
    }
    if !un(p[CRATER_RAYED_FACTOR]) && fresh(CRATER_RAYED_FACTOR) {
        p[CRATER_RAYED_FACTOR] *= k;
    }
    if bulk == bulk::ASTEROID {
        if !un(p[CRATER_OCTAVES]) && fresh(CRATER_OCTAVES) {
            p[CRATER_OCTAVES] *= sat(1.0 - p[LAVA_LEVEL]);
        }
    } else if !un(p[CRATER_MAGN]) && fresh(CRATER_MAGN) {
        p[CRATER_MAGN] *= 10.0 / inp.bump_height_km;
    }
    // Hot worlds lose their relief.
    if inp.t_eq >= 1000.0 {
        let k = 1.0 - sat((inp.t_eq - 1000.0) / 200.0);
        for i in [DUNES_MAGN, HILLS_MAGN, MONTES_MAGN, CANYONS_MAGN, CRATER_MAGN, CRACKS_MAGN, MARE_FREQ, VOLCANO_MAGN, RIVERS_MAGN, RIFTS_MAGN] {
            if !un(p[i]) && fresh(i) {
                p[i] *= k;
            }
        }
    }
    if !un(p[CRATER_DENSITY]) && fresh(CRATER_DENSITY) && p[CRATER_DENSITY] < 0.0025 {
        p[CRATER_DENSITY] = 0.0;
        p[CRATER_OCTAVES] = 0.0;
    }
    if !un(p[CRATER_OCTAVES]) && fresh(CRATER_OCTAVES) && p[CRATER_OCTAVES] < 1.0 {
        p[CRATER_DENSITY] = 0.0;
    }
    if !un(p[CRATER_RAYED_FACTOR]) && fresh(CRATER_RAYED_FACTOR) && p[CRATER_RAYED_FACTOR] < 0.05 {
        p[CRATER_RAYED_FACTOR] = 0.0;
    }
    if un(p[VOLCANO_RADIUS]) {
        p[VOLCANO_RADIUS] = r.tnormal(0.5, 0.05, 0.3, 0.8);
    }
    if bulk != bulk::ASTEROID {
        let x = (rad - 1500.0) / 1500.0;
        let k = if x < 0.0 { 0.3 } else if x > 1.0 { 1.0 } else { x * 0.7 + 0.3 };
        for i in [MONTES_MAGN, VOLCANO_MAGN, VOLCANO_RADIUS] {
            if !un(p[i]) && fresh(i) {
                p[i] *= k;
            }
        }
    }
    if un(p[VOLCANO_TEMP]) {
        p[VOLCANO_TEMP] = if inp.icy { r.gauss(273.0, 70.0) } else { r.gauss(1100.0, 100.0) };
    }
    // `volcanoActivity` is never generated, so an unset activity zeroes the density (as in SE).
    let mut dens = p[VOLCANO_DENSITY];
    if !un(dens) && fresh(VOLCANO_DENSITY) {
        let x = (p[VOLCANO_ACTIVITY] + 1.0) * dens;
        dens = if x < 0.0 { 0.0 } else { x.min(0.4) };
        p[VOLCANO_DENSITY] = dens;
    }
    if !un(p[VOLCANO_OCTAVES]) && fresh(VOLCANO_OCTAVES) && (p[VOLCANO_FREQ] == 0.0 || dens == 0.0) {
        p[VOLCANO_OCTAVES] = 0.0;
    }
    normalize_fractions(&mut p);
    done(p)
}

fn set0(p: &mut [f32; N_PARAMS], i: usize) {
    if p[i] == UNSET {
        p[i] = 0.0;
    }
}

fn set_tn(p: &mut [f32; N_PARAMS], r: &mut SeRng, i: usize, a: (f32, f32, f32, f32)) {
    if p[i] == UNSET {
        p[i] = r.tnormal(a.0, a.1, a.2, a.3);
    }
}

/// Canyons of surface classes 1..3.
fn canyons_std(p: &mut [f32; N_PARAMS], r: &mut SeRng) {
    set_tn(p, r, CANYONS_MAGN, (0.04, 0.02, 0.0, 0.07));
    if p[CANYONS_FREQ] == UNSET {
        p[CANYONS_FREQ] = r.tnormal(50.0, 5.0, 40.0, 60.0) * 3.0;
    }
    if p[CANYONS_FRACTION] == UNSET {
        p[CANYONS_FRACTION] = r.uniform(1.0);
    }
}

/// Craters of surface classes 3, 5, 6.
fn craters_icy(p: &mut [f32; N_PARAMS], r: &mut SeRng, crater_oct: f32, sigma: f32, freq_sigma: f32) {
    set_tn(p, r, CRATER_MAGN, (1.2, sigma, 1.0, 1.4));
    set_tn(p, r, CRATER_FREQ, (16.0, freq_sigma, 10.0, 22.0));
    set_tn(p, r, CRATER_DENSITY, (0.9, sigma, 0.7, 1.0));
    if p[CRATER_OCTAVES] == UNSET {
        p[CRATER_OCTAVES] = crater_oct;
    }
    if p[CRATER_RAYED_FACTOR] == UNSET {
        p[CRATER_RAYED_FACTOR] = r.tnormal(0.3, 0.2, 0.0, 0.5) * 0.5;
    }
}

/// `riversSin`, rifts (classes 1..3: no rift relief).
fn rivers_rifts_tail(p: &mut [f32; N_PARAMS], r: &mut SeRng, rifts_magn: f32) {
    set_tn(p, r, RIVERS_SIN, (6.0, 1.0, 3.0, 9.0));
    if p[RIFTS_MAGN] == UNSET {
        p[RIFTS_MAGN] = rifts_magn;
    }
    set_tn(p, r, RIFTS_FREQ, (3.0, 0.7, 1.0, 5.0));
    set_tn(p, r, RIFTS_SIN, (6.0, 1.0, 3.0, 9.0));
}

/// A star's photosphere (FUN_1402f37c0's branch for object type 3), in the slots SE's sun shaders
/// read under other names (tg_common.glh 245-259): granulation (hillsFreq/Magn, dunesMagn,
/// canyonsMagn, cracksOctaves), flows (colorDistFreq/Magn), granule temperatures (latTropic,
/// latIceCaps) and spots (mareFreq, mareDensity, montesFreq, climatePole/Tropic). The granulation
/// scale follows the surface gravity: giants (and carbon stars) `4 g / sqrt(Teff)`, with flows and
/// up to 15 octaves; dwarfs `sqrt(Teff / 6000) f^2 / sqrt(g / g_sun)`, `f` ramping from 1500 to
/// 4000 K, with spots that grow with the spectral type and keep to a latitude band above 2700 K.
fn star_surface(p: &mut [f32; N_PARAMS], r: &mut SeRng, inp: &GenInput) {
    let un = |v: f32| v == UNSET;
    let code = inp.star.unwrap_or_default();
    let giant = code.giant();
    let t = inp.t_eq;
    let f = if giant { 1.0 } else { ((t - 1500.0) / 2500.0).clamp(0.0, 1.0) };
    // Surface gravity in Earth g (mass in Earth masses over the radius in Earth radii squared).
    let g = inp.mass_earth / (inp.radius_km / 6371.0).powi(2);
    let k = if giant {
        (g * 4.0 / t.max(1.0).sqrt()).max(5.0e-5)
    } else {
        (t.min(30000.0) / 6000.0).sqrt() * f * f / (g / 28.0).max(1e-12).sqrt()
    };
    if un(p[HILLS_FREQ]) {
        p[HILLS_FREQ] = r.tnormal(5000.0, 1000.0, 2000.0, 7000.0) * k;
    }
    if un(p[HILLS_MAGN]) {
        p[HILLS_MAGN] = if giant { (7.5e-5 / k).min(1.0) } else { 0.03 };
    }
    if un(p[DUNES_MAGN]) {
        p[DUNES_MAGN] = if giant { (2.5e-4 / k).min(5.0) } else { 0.5 / f.sqrt().max(0.1) };
    }
    if un(p[CANYONS_MAGN]) {
        p[CANYONS_MAGN] = if giant { p[DUNES_MAGN] } else { r.tnormal(0.3, 0.02, 0.2, 0.4) };
    }
    if un(p[CRACKS_OCTAVES]) {
        p[CRACKS_OCTAVES] = if giant { 15.0 } else { 5.0 };
    }
    if un(p[EROSION]) {
        p[EROSION] = (k * 250.0).clamp(0.0, 1.0);
    }
    if un(p[COLOR_DIST_FREQ]) {
        p[COLOR_DIST_FREQ] = if giant { p[HILLS_FREQ].max(0.0).sqrt() * 0.015 } else { 0.0 };
    }
    if un(p[COLOR_DIST_MAGN]) {
        p[COLOR_DIST_MAGN] = if giant { p[HILLS_MAGN] } else { 0.0 };
    }
    if un(p[MONTES_MAGN]) {
        p[MONTES_MAGN] = if giant { p[HILLS_MAGN] * 0.1 } else { p[HILLS_MAGN] };
    }
    // Granule bottom/top temperatures (relative to Teff), set whatever the catalog says.
    (p[TROPIC_LATITUDE], p[ICECAP_LATITUDE]) = if giant { (0.9, 1.1) } else { (0.95, 1.05) };
    if !giant && code.lum != 11 {
        // Spots: denser and smaller for later types (G2 = 1).
        let y = ((code.letter * 100 + code.sub10) as f32 / 420.0).sqrt() * 0.75 + 0.25;
        let ff = f.max(0.2) as f64;
        if un(p[MARE_DENSITY]) {
            let d = r.tnormal(0.0025, 0.0002, 0.002, 0.003) as f64;
            p[MARE_DENSITY] = (d * (y as f64).powf(3.0) * ff * 4.0) as f32;
        }
        if un(p[MARE_FREQ]) {
            let fq = r.tnormal(50.0, 15.0, 25.0, 75.0) as f64;
            p[MARE_FREQ] = (fq * (y as f64).powf(-3.0) * ff) as f32;
        }
        if un(p[MONTES_FREQ]) {
            p[MONTES_FREQ] = p[HILLS_FREQ].min(537.0);
        }
        // The spot band: everywhere below 2700 K, between ~0.05..0.15 and ~0.3..0.6 above 3000 K.
        let s = ((t - 2700.0) / 300.0).clamp(0.0, 1.0);
        if un(p[CLIMATE_POLE]) {
            p[CLIMATE_POLE] = (r.uniform(0.3) + 0.3 - 1.0) * s + 1.0;
        }
        if un(p[CLIMATE_TROPIC]) {
            p[CLIMATE_TROPIC] = (r.uniform(0.1) + 0.05) * s;
        }
    } else {
        for i in [MARE_DENSITY, MARE_FREQ, MONTES_FREQ] {
            if un(p[i]) {
                p[i] = 0.0;
            }
        }
    }
}

/// A giant's cloud bands and cyclones (FUN_1402f37c0's giant branch, the bodies that are not
/// flagged at record +0x38): `hot` above 400 K (and brown dwarfs) has fewer cyclones the hotter it is,
/// doubled cloud frequency and stripe fluctuation and halved twist. The draws' distributions are SE's;
/// two inputs are not decoded and approximated: the polar cyclones' latitude factor (a 0..1 value
/// from FUN_140515e50) is drawn uniformly, and a non-hot Jupiter's equatorial cyclone octaves (an
/// uninitialised register in the decompile) are 1..3. The draws do not follow SE's order, so a
/// procedural giant's values differ from SE's.
fn giant_clouds(p: &mut [f32; N_PARAMS], r: &mut SeRng, inp: &GenInput) {
    let un = |v: f32| v == UNSET;
    let bulk = inp.classes.bulk;
    let over = (inp.t_surf - 400.0) / 100.0;
    let hot = bulk == bulk::BROWN_DWARF || over >= 0.0;
    let neptune = bulk == bulk::NEPTUNE;
    if un(p[CLOUDS_MAIN_FREQ]) {
        let f = r.tnormal(1.0, 0.3, 0.5, 1.5);
        p[CLOUDS_MAIN_FREQ] = if hot { f + f } else { f };
    }
    if un(p[CLOUDS_MAIN_OCTAVES]) {
        p[CLOUDS_MAIN_OCTAVES] = 12.0;
    }
    if un(p[STRIPE_ZONES]) {
        let z = r.tnormal(4.0, 1.0, 0.5, 6.0);
        p[STRIPE_ZONES] = z * (inp.radius_km * 2.55e-5).clamp(0.2, 1.5);
    }
    if un(p[STRIPE_TWIST]) {
        let t = r.tnormal(11.0, 1.5, 7.0, 15.0);
        p[STRIPE_TWIST] = if hot { t * 0.5 } else { t };
    }
    if un(p[CYCLONE_OCTAVES]) {
        p[CYCLONE_OCTAVES] = if hot {
            (5.0 - over).clamp(0.0, 5.0).trunc()
        } else if neptune {
            r.int(2) as f32
        } else {
            r.int(3) as f32 + 1.0
        };
    }
    if un(p[CYCLONE_MAGN]) {
        p[CYCLONE_MAGN] = r.tnormal(1.8, 0.5, 0.5, 3.5) * std::f32::consts::TAU;
    }
    if un(p[CYCLONE_FREQ]) {
        let f = r.tnormal(1.0, 0.3, 0.5, 1.5) * 0.8;
        p[CYCLONE_FREQ] = if hot { f * 0.5 } else { f };
    }
    if un(p[CYCLONE_DENSITY]) {
        p[CYCLONE_DENSITY] = r.tnormal(0.4, 0.1, 0.1, 1.0);
    }
    if un(p[CYCLONE_MAGN2]) {
        p[CYCLONE_MAGN2] = r.tnormal(1.0, 0.3, 0.1, 3.0) * 0.5;
    }
    if un(p[CYCLONE_FREQ2]) {
        let f = r.tnormal(1.0, 0.3, 0.5, 1.5) * 0.8;
        p[CYCLONE_FREQ2] = if hot { f * 0.5 } else { f };
    }
    if un(p[CYCLONE_LATITUDE2]) {
        p[CYCLONE_LATITUDE2] = 0.85 + 0.15 * r.uniform(1.0);
    }
    if un(p[CYCLONE_OCTAVES2]) {
        let o = if hot {
            (5.0 - over).clamp(0.0, 4.0).trunc()
        } else if neptune {
            0.0
        } else {
            r.int(3) as f32 + 2.0
        };
        p[CYCLONE_OCTAVES2] = if p[CYCLONE_LATITUDE2] >= 0.95 { 0.0 } else { o };
    }
    if un(p[STRIPE_FLUCT]) {
        let f = r.tnormal(0.2, 0.05, 0.15, 0.25) * 2.0;
        p[STRIPE_FLUCT] = if hot { f + f } else { f };
    }
}

/// A brown dwarf's surface parameters (SE's bulk class 8, drawn by tg_browndwarf_*, FUN_1402ef870
/// generator 6): the star branch's photosphere (its `erosion` mixes the granulation in), then the
/// giant branch's cloud bands, hot (`bulk == BROWN_DWARF`: doubled cloud frequency and fluctuation,
/// halved twist, fewer cyclones the hotter). Draws as in [`generate`] and `giant_clouds`, not in
/// SE's order.
pub fn brown_dwarf(inp: &GenInput, given: &[f32; N_PARAMS]) -> SurfaceGen {
    let mut g = generate(inp, given);
    let mut i = inp.clone();
    i.classes.bulk = bulk::BROWN_DWARF;
    let mut r = SeRng::new(inp.seeds.0 ^ 0x0b0d_0b0d, inp.seeds.1);
    // The star branch's final clamp turned the unset octaves into 0.
    g.p[CLOUDS_MAIN_OCTAVES] = given[CLOUDS_MAIN_OCTAVES];
    giant_clouds(&mut g.p, &mut r, &i);
    g.p[CLOUDS_MAIN_OCTAVES] = if g.p[CLOUDS_MAIN_OCTAVES] >= 0.0 { g.p[CLOUDS_MAIN_OCTAVES].min(15.0) } else { 0.0 };
    g
}

/// Leading/trailing darkening of airless moons of giants (class 6, `drivenDarkening`).
fn driven_darkening(inp: &GenInput) -> f32 {
    let Some(m) = inp.moon else { return 0.0 };
    if !(inp.obj_type == obj::MOON || inp.obj_type == obj::DWARF_MOON) || inp.pressure_atm != 0.0 || !m.parent_giant {
        return 0.0;
    }
    let x = sat(m.parent_radius_km * 20.0 / m.a_km) * 0.7;
    if m.inside_sync_orbit { -x } else { x }
}

/// Stripe, cyclone and cloud parameters of solid bodies with clouds.
fn clouds(p: &mut [f32; N_PARAMS], r: &mut SeRng, inp: &GenInput) {
    if p[STRIPE_ZONES] == UNSET {
        p[STRIPE_ZONES] = r.tnormal(2.0, 0.5, 0.5, 5.0).max(1.0);
    }
    if p[STRIPE_TWIST] == UNSET {
        let t = inp.rotation_days.abs().clamp(0.01, 243.0);
        p[STRIPE_TWIST] = (t.log10() * 0.15 + 0.15).abs();
    }
    if p[CYCLONE_MAGN] == UNSET {
        p[CYCLONE_MAGN] = (r.tnormal(1.0, 0.2, 0.5, 1.5) as f64 * 2.513_274_16) as f32;
    }
    if p[CYCLONE_FREQ] == UNSET {
        p[CYCLONE_FREQ] = r.tnormal(1.0, 0.3, 0.5, 1.5) * 0.8;
    }
    if p[CYCLONE_DENSITY] == UNSET {
        p[CYCLONE_DENSITY] = r.tnormal(1.0, 0.3, 0.5, 1.5) * 0.36;
    }
    if p[CYCLONE_OCTAVES] == UNSET {
        p[CYCLONE_OCTAVES] = (r.int(3) + 2) as f32;
    }
    if p[CLOUDS_MAIN_FREQ] == UNSET {
        let rr = inp.radius_km / 6378.14;
        let k = if rr < 0.5 { 0.5 } else if rr > 2.0 { 2.0 } else { rr };
        p[CLOUDS_MAIN_FREQ] = r.tnormal(0.75, 0.07, 0.5, 1.0) * (k * 0.25 + 0.75);
    }
    if p[CLOUDS_MAIN_OCTAVES] == UNSET {
        p[CLOUDS_MAIN_OCTAVES] = 10.0;
    }
    if p[CLOUDS_COVERAGE] == UNSET {
        p[CLOUDS_COVERAGE] = r.uniform(0.2) + 0.2;
    }
}

/// Landform shares (FUN_1402f9fa0): unset fractions become 0, and slots 0x5b..0x5f hold the
/// cumulative thresholds montes | dunes | hills | hills2 | canyons the shader picks with. Exposed
/// as [`landform_thresholds`].
fn normalize_fractions(p: &mut [f32; N_PARAMS]) {
    for i in [DUNES_FRACTION, HILLS_FRACTION, HILLS2_FRACTION, CANYONS_FRACTION, MONTES_FRACTION] {
        set0(p, i);
    }
}

/// Cumulative landform thresholds `[dunes, +hills, +hills2, +canyons, +montes]` over 0..1.
pub fn landform_thresholds(g: &SurfaceGen) -> Option<[f32; 5]> {
    let w = [g.v(DUNES_FRACTION), g.v(HILLS_FRACTION), g.v(HILLS2_FRACTION), g.v(CANYONS_FRACTION), g.v(MONTES_FRACTION)];
    let total: f32 = w.iter().sum();
    if total == 0.0 {
        return None;
    }
    let mut acc = 0.0;
    let mut out = [0.0; 5];
    for (i, x) in w.iter().enumerate() {
        acc += x / total;
        out[i] = acc;
    }
    Some(out)
}

/// SE's default `SpecBrightWater` by surface class (`InitSurfaceClass`): 0.65 for the wet classes
/// 1..3, 0.3 for arid worlds, none otherwise. `RoughnessWater` defaults to 0.35 for every class.
pub fn water_spec_default(surface_class: i32) -> f32 {
    match surface_class {
        1..=3 => 0.65,
        4 => 0.3,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------------------
// Catalog bodies
// ---------------------------------------------------------------------------------------

/// Default `BumpHeight` (km) of a body without one: the port generator's rule, 19.5 km scaled by
/// gravity (Earth 19.5), 1.5..60.
pub fn default_bump_km(mass_kg: f64, radius_km: f64) -> f64 {
    let g = crate::units::G * mass_kg / (radius_km * 1000.0).powi(2).max(1.0);
    if g <= 0.0 {
        return 10.0;
    }
    (19.5 * (9.81 / g).powf(0.8)).clamp(1.5, 60.0).min(radius_km * 0.2)
}

fn sub_unless<'a>(b: &'a Block, key: &str, no: &str) -> Option<&'a Block> {
    if b.bool(no).unwrap_or(false) {
        None
    } else {
        b.sub(key)
    }
}

/// `BumpHeight` (km) of catalog body `i`: the catalog's, else SE's default (see
/// `climate::se_bump_default_km`; a generated ocean deeper than that raises it).
pub fn catalog_bump_km(cat: &crate::catalog::Catalog, i: usize, completion: Option<&crate::climate::Completion>) -> f64 {
    let b = &cat.bodies[i];
    let given = b.block.sub("Surface").and_then(|s| s.f64("BumpHeight")).filter(|v| *v > 0.0);
    given.unwrap_or_else(|| match completion {
        Some(c) => c.bump_km as f64,
        None => {
            let small = matches!(b.tag, crate::catalog::BodyTag::Asteroid | crate::catalog::BodyTag::Comet);
            crate::climate::se_bump_default_km((b.mass_kg / crate::units::EARTH_MASS_KG) as f32, b.radius_km as f32, small) as f64
        }
    })
}

/// Generator input and given slots for catalog body `i`. `t_eq` is the radiative equilibrium
/// temperature and `greenhouse` the catalog warming (K); SE's surface temperature is their sum.
/// `flux` is the insolation (1 at Earth). `completion` is SE's generated atmosphere and ocean for a
/// body the catalog leaves without.
pub fn catalog_input(cat: &crate::catalog::Catalog, i: usize, t_eq: f64, greenhouse: f64, flux: f64, completion: Option<&crate::climate::Completion>) -> (GenInput, [f32; N_PARAMS]) {
    use crate::catalog::{BodyTag, ObjectRef};
    let b = &cat.bodies[i];
    let blk = &b.block;
    let obj_type = match b.tag {
        BodyTag::Planet => obj::PLANET,
        BodyTag::DwarfPlanet => obj::DWARF_PLANET,
        BodyTag::Moon => obj::MOON,
        BodyTag::DwarfMoon => obj::DWARF_MOON,
        BodyTag::Asteroid => obj::ASTEROID,
        BodyTag::Comet => obj::COMET,
        BodyTag::Spacecraft => 11,
    };
    let class = b.class.as_deref().unwrap_or("");
    let bulk = if let Some(c) = completion {
        c.bulk
    } else if matches!(b.tag, BodyTag::Asteroid | BodyTag::Comet) && !class.eq_ignore_ascii_case("aquaria") {
        bulk::ASTEROID
    } else {
        bulk_of(class)
    };
    let surface = blk.sub("Surface");
    let atm = sub_unless(blk, "Atmosphere", "NoAtmosphere");
    let ocean = sub_unless(blk, "Ocean", "NoOcean");
    let clouds = sub_unless(blk, "Clouds", "NoClouds");
    let mass_earth = (b.mass_kg / crate::units::EARTH_MASS_KG) as f32;
    let radius_km = b.radius_km as f32;
    let bump_km = catalog_bump_km(cat, i, completion);
    let (t_eq, greenhouse) = completion.map_or((t_eq, greenhouse), |c| (c.t_eq as f64, c.greenhouse_k as f64));
    let pressure = match completion {
        Some(c) => c.pressure_atm,
        None => atm.and_then(|a| a.f64("Pressure")).unwrap_or(if atm.is_some() { 1.0 } else { 0.0 }) as f32,
    };
    let t_surf = completion.map_or((t_eq + greenhouse) as f32, |c| c.t_surf);
    let ocean_info = match completion {
        Some(c) => c.ocean_info(),
        None => ocean.map(|o| {
            let h = o.f64("Height").or_else(|| o.f64("Depth")).or_else(|| surface.and_then(|s| s.f64("BumpOffset"))).unwrap_or(bump_km * 0.5);
            OceanInfo { depth_km: h as f32, bump_height_km: bump_km as f32, global: h >= bump_km, global_shallow: false }
        }),
    };
    let obliquity_rad = (blk.f64("Obliquity").unwrap_or(0.0) as f32).to_radians();
    let rotation_days = b.rotation.period_seconds().map_or(1.0, |s| (s / 86_400.0) as f32);
    // The class word follows SE's temperature model, not the surface temperature. Bodies the
    // catalog describes fully get Earth's internal heat (plus a moon's tidal heating, with SE's
    // mean tidal Q of 600) and air of molar mass 29.
    let t_class = completion.map_or_else(
        || crate::climate::class_temperature(&catalog_thermal(cat, i, flux, greenhouse, pressure, t_surf, bulk)),
        |c| c.t_class,
    );
    let classes = Classes::new(obj_type, bulk, t_class, pressure, mass_earth, ocean_info);
    let g = crate::units::G * b.mass_kg / (b.radius_km * 1000.0).powi(2).max(1.0);
    let scale_h_km = match completion {
        Some(c) if c.scale_height_km > 0.0 => c.scale_height_km as f64,
        _ if g > 0.0 => 8.314 * t_surf as f64 / (0.029 * g) / 1000.0,
        _ => 8.0,
    };
    let parent = cat.find(&b.parent).and_then(|r| match r {
        ObjectRef::Body(j) => Some(&cat.bodies[j as usize]),
        _ => None,
    });
    let moon = parent.filter(|_| obj_type == obj::MOON || obj_type == obj::DWARF_MOON).map(|p| {
        let a_km = b.orbit.as_ref().map_or(0.0, |o| o.semi_major_axis / 1000.0) as f32;
        let period = b.orbit.as_ref().map_or(0.0, |o| o.period);
        let spin = p.rotation.period_seconds().unwrap_or(f64::INFINITY);
        MoonInfo {
            a_km: a_km.max(1.0),
            parent_radius_km: p.radius_km as f32,
            parent_ring_outer_km: sub_unless(&p.block, "Rings", "NoRings").and_then(|r| r.f64("OuterRadius")).map(|v| v as f32),
            parent_giant: bulk_of(p.class.as_deref().unwrap_or("")) >= bulk::NEPTUNE,
            inside_sync_orbit: period > 0.0 && period < spin,
        }
    });
    let inp = GenInput {
        seeds: body_seeds(b),
        obj_type,
        classes,
        radius_km,
        mass_earth,
        t_eq: t_eq as f32,
        t_surf,
        t_class,
        pressure_atm: pressure,
        atm_scale_height_km: scale_h_km as f32,
        atm_height_km: match completion {
            Some(c) if c.atm_height_km > 0.0 => c.atm_height_km,
            _ => atm.and_then(|a| a.f64("Height")).unwrap_or(scale_h_km * 10.0) as f32,
        },
        has_clouds: clouds.is_some() || completion.is_some_and(|c| c.has_clouds),
        ocean_level: ocean_info.map(|o| o.depth_km / bump_km as f32),
        ocean_depth_km: ocean_info.map_or(0.0, |o| o.depth_km),
        bump_height_km: bump_km as f32,
        life: sub_unless(blk, "Life", "NoLife").is_some(),
        icy: bulk == bulk::AQUARIA || completion.is_some_and(|c| c.composition[crate::climate::part::ICE] > 0.0),
        obliquity_rad,
        color_given: blk.vec3("Color").is_some(),
        rotation_days,
        moon,
        has_rings: sub_unless(blk, "Rings", "NoRings").is_some(),
        star: None,
    };
    (inp, given_from(surface, clouds))
}

/// Inputs of SE's temperature model for catalog body `i` when SE keeps its catalog atmosphere:
/// `flux` is the mean-distance insolation (1 at Earth), `t_surf` the surface temperature. The
/// catalog's description gives Earth's internal heat (plus a moon's tidal heating, with SE's mean
/// tidal Q of 600) and air of molar mass 29.
pub fn catalog_thermal(cat: &crate::catalog::Catalog, i: usize, flux: f64, greenhouse: f64, pressure: f32, t_surf: f32, bulk: i32) -> crate::climate::ThermalIn {
    use crate::catalog::ObjectRef;
    let b = &cat.bodies[i];
    let blk = &b.block;
    let mass_earth = (b.mass_kg / crate::units::EARTH_MASS_KG) as f32;
    let radius_km = b.radius_km as f32;
    // SE runs its moon completion (FUN_140502eb0) on every catalog moon: one without an
    // obliquity takes its orbit's inclination.
    let moon = matches!(b.tag, crate::catalog::BodyTag::Moon | crate::catalog::BodyTag::DwarfMoon);
    let obliquity_rad = blk
        .f64("Obliquity")
        .or_else(|| blk.sub("Rotation").and_then(|r| r.f64("Obliquity")))
        .map(|d| (d as f32).to_radians() as f64)
        .or_else(|| b.orbit.as_ref().filter(|_| moon).map(|o| o.inclination))
        .unwrap_or(0.0) as f32;
    let rotation_days = b.rotation.period_seconds().map_or(1.0, |s| (s / 86_400.0) as f32);
    let star_parent = matches!(cat.find(&b.parent), Some(ObjectRef::Star(s)) if !crate::climate::planet_barycentre(cat, s));
    // A moon heated by its planet, or one of a binary planet's pair by its partner at their
    // separation (as the completion, `climate::catalog_body_in`); a generated body's is capped.
    let host = match (crate::climate::binary_partner(cat, i), cat.find(&b.parent)) {
        (Some(partner), _) => Some(partner),
        (None, Some(ObjectRef::Body(p))) => b.orbit.as_ref().map(|o| (cat.bodies[p as usize].mass_kg / crate::units::EARTH_MASS_KG, o.semi_major_axis / crate::units::METERS_PER_AU)),
        _ => None,
    };
    let tidal = match (host, b.orbit.as_ref()) {
        (Some((host_me, a_au)), Some(o)) if o.eccentricity < 1.0 => {
            let mut w = o.eccentricity.powi(2) * crate::climate::tidal_k(radius_km, a_au, host_me, 600.0);
            if b.source == crate::climate::PROCEDURAL_SOURCE {
                w = w.min(crate::climate::tidal_heat_cap_w(mass_earth, t_surf));
            }
            w * crate::climate::small_body_taper(radius_km)
        }
        _ => 0.0,
    };
    let kk = mass_earth as f64 * 5e-12 * crate::units::EARTH_MASS_KG;
    let heat = if kk > 0.0 { (1.0 + tidal / kk) as f32 } else { 1.0 };
    let rot = blk.sub("Rotation");
    let locked = (blk.bool("TidalLocked").unwrap_or(false) || rot.is_some_and(|r| r.bool("TidalLocked").unwrap_or(false)))
        && star_parent
        && bulk < bulk::NEPTUNE;
    crate::climate::ThermalIn {
        flux: flux as f32,
        bond_albedo: blk.f64_any(&["AlbedoBond", "Albedo"]).unwrap_or(b.albedo) as f32,
        heat,
        greenhouse_k: greenhouse as f32,
        pressure_atm: pressure,
        density_kg_m3: if pressure > 0.0 { pressure * 101.325 * 29.0 / (t_surf.max(20.0) * 8.314_46) } else { 0.0 },
        bulk,
        steam: false,
        locked_to_star: locked,
        obliquity_rad: obliquity_rad as f64,
        rotation_days: if locked { b.orbit.as_ref().map_or(1.0, |o| o.period / 86_400.0) } else { rotation_days as f64 },
        mass_earth,
        radius_km,
    }
}

/// Host-star temperature SE's preset choice compares with `SunTemp`: the star a body ultimately
/// orbits, but 5800 K when that star is the first object of its system (the generator reads the
/// star only for an index above 0), i.e. for a star with no barycentre or star above it.
pub fn preset_sun_temp(cat: &crate::catalog::Catalog, i: usize) -> f64 {
    use crate::catalog::ObjectRef;
    let mut name = cat.bodies[i].parent.clone();
    for _ in 0..16 {
        match cat.find(&name) {
            Some(ObjectRef::Body(j)) => name = cat.bodies[j as usize].parent.clone(),
            Some(ObjectRef::Star(s)) => {
                let st = &cat.stars[s as usize];
                if st.barycenter {
                    return 5800.0;
                }
                return if st.parent.as_deref().is_some_and(|p| cat.find(p).is_some()) { st.teff } else { 5800.0 };
            }
            _ => break,
        }
    }
    5800.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brown_dwarf_gets_hot_cloud_bands() {
        let inp = GenInput {
            seeds: (17, 4),
            obj_type: obj::STAR,
            classes: Classes { bulk: -1, ..Default::default() },
            radius_km: 62_000.0,
            mass_earth: 10_000.0,
            t_eq: 1350.0,
            t_surf: 1350.0,
            t_class: 1350.0,
            star: Some(StarCode::of(&crate::star::SpectralClass::parse("L8V"))),
            ..Default::default()
        };
        let g = brown_dwarf(&inp, &[UNSET; N_PARAMS]);
        // Hot: doubled frequency (0.5..1.5 drawn), halved twist (7..15 drawn), no cyclones at 1350 K.
        assert!((1.0..=3.0).contains(&g.v(CLOUDS_MAIN_FREQ)), "{}", g.v(CLOUDS_MAIN_FREQ));
        assert!((3.5..=7.5).contains(&g.v(STRIPE_TWIST)), "{}", g.v(STRIPE_TWIST));
        assert_eq!((g.v(CYCLONE_OCTAVES), g.v(CYCLONE_OCTAVES2)), (0.0, 0.0));
        assert_eq!(g.v(CLOUDS_MAIN_OCTAVES), 12.0);
        // Below 1500 K the star branch's granulation is off: erosion 0, bands only.
        assert_eq!(g.v(EROSION), 0.0);
    }

    #[test]
    fn star_surfaces() {
        let sun = GenInput {
            seeds: catalog_seeds("Sun", ""),
            obj_type: obj::STAR,
            classes: Classes { bulk: -1, ..Default::default() },
            radius_km: 695_700.0,
            mass_earth: 332_946.0,
            t_eq: 5772.0,
            t_surf: 5772.0,
            star: Some(StarCode::of(&crate::star::SpectralClass::parse("G2V"))),
            ..Default::default()
        };
        assert_eq!(sun.star, Some(StarCode { lum: 6, letter: 4, sub10: 20 }));
        let g = generate(&sun, &[UNSET; N_PARAMS]);
        // k ~ 0.98: granules ~2000..7000 x k, five octaves, spots ~50 per radian x 0.1.
        assert!((1900.0..6900.0).contains(&g.v(HILLS_FREQ)), "{}", g.v(HILLS_FREQ));
        assert_eq!((g.v(HILLS_MAGN), g.v(CRACKS_OCTAVES), g.v(COLOR_DIST_FREQ)), (0.03, 5.0, 0.0));
        assert_eq!((g.v(TROPIC_LATITUDE), g.v(ICECAP_LATITUDE)), (0.95, 1.05));
        assert!((0.0078..0.0118).contains(&g.v(MARE_DENSITY)), "{}", g.v(MARE_DENSITY));
        assert!((24.0..76.0).contains(&g.v(MARE_FREQ)));
        assert_eq!(g.v(MONTES_FREQ), 537.0);
        assert!((0.3..0.6).contains(&g.v(CLIMATE_POLE)) && (0.05..0.15).contains(&g.v(CLIMATE_TROPIC)));
        assert!(g.color.is_none());
        // A red giant: big cells, flows, 15 octaves, no spots.
        let giant = GenInput {
            radius_km: 50.0 * 695_700.0,
            mass_earth: 1.2 * 332_946.0,
            t_eq: 4000.0,
            star: Some(StarCode::of(&crate::star::SpectralClass::parse("K3III"))),
            ..sun.clone()
        };
        let g = generate(&giant, &[UNSET; N_PARAMS]);
        assert!(g.v(HILLS_FREQ) < 10.0 && g.v(COLOR_DIST_FREQ) > 0.0, "{}", g.v(HILLS_FREQ));
        assert_eq!((g.v(CRACKS_OCTAVES), g.v(MARE_DENSITY), g.v(TROPIC_LATITUDE)), (15.0, 0.0, 0.9));
    }

    fn rocky(surface: i32) -> GenInput {
        let ocean = if surface <= 3 { Some(0.56) } else { None };
        GenInput {
            seeds: catalog_seeds("Test b", "Test"),
            obj_type: obj::PLANET,
            classes: Classes { bulk: bulk::TERRA, temp: 3, atmo: 3, volq: 1, size: 3, surface },
            radius_km: 6000.0,
            mass_earth: 0.8,
            t_eq: 260.0,
            t_surf: 280.0,
            t_class: 280.0,
            pressure_atm: 1.0,
            atm_scale_height_km: 8.0,
            atm_height_km: 80.0,
            ocean_level: ocean,
            ocean_depth_km: 4.0,
            bump_height_km: 20.0,
            ..Default::default()
        }
    }

    #[test]
    fn rng_streams_match_the_exe_formulas() {
        let mut r = SeRng::new(1, 2);
        // a = 1 * 1664525 + 1013904223
        let a1 = 1u32.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        assert_eq!(r.int(u32::MAX), a1 % u32::MAX);
        let u = r.uniform(1.0);
        assert!((0.0..1.0).contains(&u));
        for _ in 0..2000 {
            let v = r.tnormal(0.9, 0.1, 0.7, 1.0);
            assert!((0.7..=1.0).contains(&v), "{v}");
            let w = r.tnormal(0.1, 0.05, 0.2, 0.0); // reversed bounds, as in SE's arid seaLevel
            assert!((0.0..=0.2).contains(&w), "{w}");
        }
        // Mean of the truncated normal is pulled toward the inside of [lo, hi].
        let n = 20000;
        let m: f32 = (0..n).map(|_| r.tnormal(5.0, 2.0, 2.0, 8.0)).sum::<f32>() / n as f32;
        assert!((m - 5.0).abs() < 0.05, "{m}");
        let g: f32 = (0..n).map(|_| r.gauss(1.0, 0.3)).sum::<f32>() / n as f32;
        assert!((g - 1.0).abs() < 0.01, "{g}");
    }

    #[test]
    fn erf_and_quantile() {
        assert!((erf(0.5) - 0.520_499_877_813_046_5).abs() < 1e-13);
        assert!((erf(3.0) - 0.999_977_909_503_001_4).abs() < 1e-13);
        for p in [1e-9, 0.01, 0.3, 0.5, 0.77, 0.999] {
            let x = inv_norm_cdf(p);
            let back = 0.5 * erfc(-x / std::f64::consts::SQRT_2);
            assert!((back - p).abs() < 1e-12 * p.max(1e-3), "{p} {back}");
        }
    }

    #[test]
    fn catalog_seed_hash() {
        // h = 'a' = 97; then h*131 + '|' (124); then h*131 + 'b' (98).
        let h = ((97i32 * 131 + 124) * 131 + 98) as u32;
        assert_eq!(catalog_seeds("a", "b"), (h.wrapping_mul(0x14ab8d).wrapping_add(0x47f7e9f), h.wrapping_mul(0x56a18d).wrapping_add(0x44796b)));
        assert_ne!(catalog_seeds("Proxima Cen b", "Proxima Cen"), catalog_seeds("Proxima Cen c", "Proxima Cen"));
    }

    #[test]
    fn classes_follow_the_exe_thresholds() {
        assert_eq!(TEMP_NAMES[temp_class(288.0) as usize], "Temperate");
        assert_eq!(TEMP_NAMES[temp_class(90.0) as usize], "Frigid");
        assert_eq!(TEMP_NAMES[temp_class(1000.5) as usize], "Torrid");
        assert_eq!(ATMO_NAMES[atmo_class(bulk::TERRA, 0.006) as usize], "Hypobaric");
        assert_eq!(ATMO_NAMES[atmo_class(bulk::TERRA, 1.0) as usize], "Mesobaric");
        assert_eq!(ATMO_NAMES[atmo_class(bulk::TERRA, 0.0) as usize], "Airless");
        let earth = OceanInfo { depth_km: 10.9, bump_height_km: 19.5, ..Default::default() };
        assert_eq!(VOLQ_NAMES[volq_class(bulk::TERRA, 3, Some(earth)) as usize], "Marine");
        assert_eq!(VOLQ_NAMES[volq_class(bulk::TERRA, 2, None) as usize], "Arid");
        assert_eq!(surface_class(obj::PLANET, bulk::TERRA, 3, 3), 2);
        assert_eq!(surface_class(obj::PLANET, bulk::TERRA, 2, 1), 4);
        assert_eq!(surface_class(obj::PLANET, bulk::TERRA, 1, 1), 5);
        assert_eq!(surface_class(obj::MOON, bulk::AQUARIA, 0, 0), 6);
        assert_eq!(surface_class(obj::ASTEROID, bulk::ASTEROID, 0, 9), 7);
        assert_eq!(surface_class(obj::PLANET, bulk::JUPITER, 7, 9), 8);
        assert_eq!(size_class(bulk::TERRA, 1.0), 3);
        assert_eq!(size_class(bulk::JUPITER, 317.8), 3);
    }

    #[test]
    fn generator_ranges_and_given_values() {
        for surface in 1..=7 {
            for k in 0..30u32 {
                let mut inp = rocky(surface);
                inp.seeds = (k.wrapping_mul(7919), k.wrapping_mul(104_729) ^ 0x55);
                if surface == 7 {
                    inp.classes.bulk = bulk::ASTEROID;
                    inp.obj_type = obj::ASTEROID;
                    inp.radius_km = 120.0;
                    inp.pressure_atm = 0.0;
                }
                let g = generate(&inp, &given_from(None, None));
                let s = g.surf_style();
                assert!((0.0..1.0).contains(&s));
                assert!(g.get(MONTES_MAGN).is_some(), "class {surface}");
                assert!(g.v(MONTES_MAGN) >= 0.0 && g.v(MONTES_MAGN) < 1.0);
                assert!(g.v(CRATER_OCTAVES) <= 30.0);
                assert!(landform_thresholds(&g).map_or(true, |t| (t[4] - 1.0).abs() < 1e-5));
                assert!(g.color.is_some());
                for i in 0..3 {
                    assert!((-1.0..1.0).contains(&g.p[i]));
                }
            }
        }
        // Given keys are kept, and the same seeds give the same result.
        let root = crate::sc::parse("Surface { montesMagn 0.123 SurfStyle 0.42 }");
        let given = given_from(root.sub("Surface"), None);
        let g1 = generate(&rocky(4), &given);
        assert_eq!(g1.v(MONTES_MAGN), 0.123);
        assert_eq!(g1.surf_style(), 0.42);
        assert_eq!(g1, generate(&rocky(4), &given));
    }
}
