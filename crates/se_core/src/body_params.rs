//! Typed views of the planet and star sub-blocks of a SpaceEngine `.sc` body
//! (`Atmosphere`, `Clouds`, `Ocean`, `Aurora`, `Life`, `Corona`, `CometTail`,
//! `AccretionDisk`) plus the `NoXxx` flags that switch features off.
//!
//! Every `from_block` constructor is total: missing keys fall back to the
//! defaults documented on each field. Units follow the catalog vocabulary
//! (see `docs/formats/sc_vocabulary.md`); lengths are km unless noted.

use std::collections::BTreeMap;

use crate::sc::Block;

/// Gas name -> percent (or the unit the catalog uses; the values are stored as-is).
fn composition(b: &Block) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let Some(comp) = b.sub("Composition") {
        for it in &comp.items {
            if it.key.is_empty() {
                continue;
            }
            if let Some(v) = it.f64() {
                out.insert(it.key.clone(), v);
            }
        }
    }
    out
}

fn flag(b: &Block, key: &str) -> bool {
    b.bool(key).unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AtmosphereParams {
    /// `Model` name selecting the scattering model, e.g. `"Earth"`, `"Mars"`.
    pub model: Option<String>,
    /// `Height`, km. Default 0.
    pub height_km: f64,
    /// `Pressure`, atm at the reference surface. Default 0.
    pub pressure_atm: f64,
    /// `Density`, kg/m^3 at the surface. Default 0.
    pub density_kg_m3: f64,
    /// `Greenhouse`, K of surface warming. Default 0.
    pub greenhouse_k: f64,
    /// `Adiabat`, lapse-rate exponent (dimensionless). Default 0.
    pub adiabat: f64,
    /// `Bright`, scattering brightness multiplier. Default 1.0.
    pub bright: f64,
    /// `Opacity`, dimensionless (0 = invisible). Default 1.0.
    pub opacity: f64,
    /// `SkyLight`, sky-ambient multiplier. Default 1.0.
    pub sky_light: f64,
    /// `Hue`, shift applied to the sky colour (dimensionless, catalog units). `None` if absent.
    pub hue: Option<f64>,
    /// `Saturation`, sky colour saturation (0..1 typical). `None` if absent.
    pub saturation: Option<f64>,
    /// Constituent gas -> value from the `Composition` sub-block (percent by volume).
    pub composition: BTreeMap<String, f64>,
}

impl AtmosphereParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            model: b.str("Model").map(str::to_string),
            height_km: b.f64("Height").unwrap_or(0.0),
            pressure_atm: b.f64("Pressure").unwrap_or(0.0),
            density_kg_m3: b.f64("Density").unwrap_or(0.0),
            greenhouse_k: b.f64("Greenhouse").unwrap_or(0.0),
            adiabat: b.f64("Adiabat").unwrap_or(0.0),
            bright: b.f64("Bright").unwrap_or(1.0),
            opacity: b.f64("Opacity").unwrap_or(1.0),
            sky_light: b.f64("SkyLight").unwrap_or(1.0),
            hue: b.f64("Hue"),
            saturation: b.f64("Saturation"),
            composition: composition(b),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloudsParams {
    /// `DiffMap` texture name (diffuse/albedo of the cloud layer).
    pub diff_map: Option<String>,
    /// `DiffMapAlpha` mode string, e.g. `"None"`, `"Water"`.
    pub diff_map_alpha: Option<String>,
    /// `FlipMap`: rotate the cloud map by 180 degrees. Default false.
    pub flip_map: bool,
    /// `BumpMap` texture name for cloud relief.
    pub bump_map: Option<String>,
    /// `BumpHeight`, km elevation span of the bump map. Default 0.
    pub bump_height_km: f64,
    /// `BumpOffset`, km depth of zero bump below the cloud radius. Default 0.
    pub bump_offset_km: f64,
    /// `Height`, km above the surface reference radius. Default 0.
    pub height_km: f64,
    /// `Velocity`, km/h of cloud drift. Default 0.
    pub velocity_kmh: f64,
    /// `Hapke`, Hapke roughness/backscatter parameter (dimensionless). Default 0.
    pub hapke: f64,
    /// `ModulateColor`, RGB multiplier on the cloud colour. Default (1, 1, 1).
    pub modulate_color: [f64; 3],
    /// `ModulateBright`, brightness multiplier. Default 1.0.
    pub modulate_bright: f64,
    /// `Opacity`, dimensionless. Default 1.0.
    pub opacity: f64,
    /// `Coverage`, fraction of the map that is cloud (0..1). Default 1.0.
    pub coverage: f64,
    /// `DayAmbient`, ambient light on the day side (dimensionless). Default 0.
    pub day_ambient: f64,
    /// `RingsWinter`, winter shadowing factor from rings (dimensionless). `None` if absent.
    pub rings_winter: Option<f64>,
    /// `SpotBright`, brightness of the sub-solar spot. Default 0.
    pub spot_bright: f64,
    /// `SpotWidth`, angular width of the spot (radians in catalog units). Default 0.
    pub spot_width: f64,
    /// `mainFreq`, base noise frequency of procedural clouds. Default 0.
    pub main_freq: f64,
    /// `mainOctaves`, noise octaves. Default 0.
    pub main_octaves: u32,
    /// `stripeZones`, number of latitudinal stripe zones. Default 0.
    pub stripe_zones: f64,
    /// `stripeTwist`, twist of stripe pattern. Default 0.
    pub stripe_twist: f64,
    /// `stripeFluct`, stripe fluctuation amount. Default 0.
    pub stripe_fluct: f64,
    /// `cycloneDensity`, procedural cyclone count density. Default 0.
    pub cyclone_density: f64,
    /// `cycloneOctaves`, noise octaves for cyclones. Default 0.
    pub cyclone_octaves: u32,
    /// `cycloneFreq`, cyclone noise frequency. Default 0.
    pub cyclone_freq: f64,
    /// `cycloneMagn`, cyclone distortion magnitude. Default 0.
    pub cyclone_magn: f64,
}

impl CloudsParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            diff_map: b.str("DiffMap").map(str::to_string),
            diff_map_alpha: b.str("DiffMapAlpha").map(str::to_string),
            flip_map: flag(b, "FlipMap"),
            bump_map: b.str("BumpMap").map(str::to_string),
            bump_height_km: b.f64("BumpHeight").unwrap_or(0.0),
            bump_offset_km: b.f64("BumpOffset").unwrap_or(0.0),
            height_km: b.f64("Height").unwrap_or(0.0),
            velocity_kmh: b.f64("Velocity").unwrap_or(0.0),
            hapke: b.f64("Hapke").unwrap_or(0.0),
            modulate_color: b.vec3("ModulateColor").unwrap_or([1.0, 1.0, 1.0]),
            modulate_bright: b.f64("ModulateBright").unwrap_or(1.0),
            opacity: b.f64("Opacity").unwrap_or(1.0),
            coverage: b.f64("Coverage").unwrap_or(1.0),
            day_ambient: b.f64("DayAmbient").unwrap_or(0.0),
            rings_winter: b.f64("RingsWinter"),
            spot_bright: b.f64("SpotBright").unwrap_or(0.0),
            spot_width: b.f64("SpotWidth").unwrap_or(0.0),
            main_freq: b.f64("mainFreq").unwrap_or(0.0),
            main_octaves: b.f64("mainOctaves").unwrap_or(0.0) as u32,
            stripe_zones: b.f64("stripeZones").unwrap_or(0.0),
            stripe_twist: b.f64("stripeTwist").unwrap_or(0.0),
            stripe_fluct: b.f64("stripeFluct").unwrap_or(0.0),
            cyclone_density: b.f64("cycloneDensity").unwrap_or(0.0),
            cyclone_octaves: b.f64("cycloneOctaves").unwrap_or(0.0) as u32,
            cyclone_freq: b.f64("cycloneFreq").unwrap_or(0.0),
            cyclone_magn: b.f64("cycloneMagn").unwrap_or(0.0),
        }
    }

    /// Every `Clouds` sub-block of a body, in file order. A body may have several.
    pub fn all_from_body(b: &Block) -> Vec<Self> {
        b.all("Clouds").filter_map(|i| i.block()).map(Self::from_block).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OceanParams {
    /// `Height`, km of the ocean surface above the reference radius. Default 0.
    pub height_km: f64,
    /// `Hapke`, Hapke parameter of the water surface (dimensionless). Default 0.
    pub hapke: f64,
    /// Constituent -> value from the `Composition` sub-block.
    pub composition: BTreeMap<String, f64>,
}

impl OceanParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            height_km: b.f64("Height").unwrap_or(0.0),
            hapke: b.f64("Hapke").unwrap_or(0.0),
            composition: composition(b),
        }
    }
}

/// One of the two auroral ovals (`North*` or `South*` keys).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AuroraOval {
    /// Latitude of the oval centre, degrees.
    pub lat_deg: f64,
    /// Longitude of the oval centre, degrees.
    pub lon_deg: f64,
    /// Radius of the oval, km.
    pub radius_km: f64,
    /// Width of the oval band, km.
    pub width_km: f64,
    /// Number of concentric rings.
    pub rings: u32,
    /// Brightness multiplier (dimensionless).
    pub bright: f64,
    /// Flash frequency (catalog units, flashes per unit time). Default 0.
    pub flash_freq: f64,
    /// Drift speed of the aurora pattern (catalog units). Default 0.
    pub move_speed: f64,
    /// Particle count of the effect. Default 0.
    pub particles: u32,
}

impl AuroraOval {
    fn from_block(b: &Block, prefix: &str) -> Self {
        let k = |s: &str| format!("{prefix}{s}");
        Self {
            lat_deg: b.f64(&k("Lat")).unwrap_or(0.0),
            lon_deg: b.f64(&k("Lon")).unwrap_or(0.0),
            radius_km: b.f64(&k("Radius")).unwrap_or(0.0),
            width_km: b.f64(&k("Width")).unwrap_or(0.0),
            rings: b.f64(&k("Rings")).unwrap_or(0.0) as u32,
            bright: b.f64(&k("Bright")).unwrap_or(1.0),
            flash_freq: b.f64(&k("FlashFreq")).unwrap_or(0.0),
            move_speed: b.f64(&k("MoveSpeed")).unwrap_or(0.0),
            particles: b.f64(&k("Particles")).unwrap_or(0.0) as u32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AuroraParams {
    /// `Height`, km of the aurora layer above the surface. Default 0.
    pub height_km: f64,
    /// `TopColor`, RGB of the upper edge of the curtain. Default (0, 0, 0).
    pub top_color: [f64; 3],
    /// `BottomColor`, RGB of the lower edge of the curtain. Default (0, 0, 0).
    pub bottom_color: [f64; 3],
    /// Northern oval, from `North*` keys.
    pub north: AuroraOval,
    /// Southern oval, from `South*` keys.
    pub south: AuroraOval,
}

impl AuroraParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            height_km: b.f64("Height").unwrap_or(0.0),
            top_color: b.vec3("TopColor").unwrap_or([0.0; 3]),
            bottom_color: b.vec3("BottomColor").unwrap_or([0.0; 3]),
            north: AuroraOval::from_block(b, "North"),
            south: AuroraOval::from_block(b, "South"),
        }
    }
}

/// One `Life` block. Bodies can carry several (e.g. one per biome).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LifeParams {
    /// `Class`, e.g. `"Organic"`.
    pub class: Option<String>,
    /// `Type`, e.g. `"Multicellular"`.
    pub kind: Option<String>,
    /// `Biome`, e.g. `"Marine/Terrestrial"`.
    pub biome: Option<String>,
}

impl LifeParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            class: b.str("Class").map(str::to_string),
            kind: b.str("Type").map(str::to_string),
            biome: b.str("Biome").map(str::to_string),
        }
    }

    pub fn all_from_body(b: &Block) -> Vec<Self> {
        b.all("Life").filter_map(|i| i.block()).map(Self::from_block).collect()
    }
}

/// Stellar `Corona` block (FUN_14046bfc0 / FUN_140468860 read it into the body at +0x1cdc..+0x1cec).
/// A key the block leaves out is `None` and gets SE's per-star default ([`Corona::resolve`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CoronaParams {
    /// `Radius`, km: the half-size of the corona planes (Eta Carinae A: 4.0159e8 km, 5.8 star radii).
    pub radius_km: Option<f64>,
    /// `Period` of the ray pattern, years (SE keeps days: x 365.24218985, DAT_1406d51f0).
    pub period_yr: Option<f64>,
    /// `Brightness`, multiplier.
    pub brightness: Option<f64>,
    /// `RayDensity`: the angular frequency of the rays (`Shape.x` of corona_plane_proc.glsl).
    pub ray_density: Option<f64>,
    /// `RayCurv`: how fast the rays change along the radius (`Shape.y`).
    pub ray_curv: Option<f64>,
}

impl CoronaParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            radius_km: b.f64("Radius"),
            period_yr: b.f64("Period"),
            brightness: b.f64("Brightness"),
            ray_density: b.f64("RayDensity"),
            ray_curv: b.f64("RayCurv"),
        }
    }
}

/// SE's days per catalog year of `Period` (DAT_1406d51f0).
pub const CORONA_DAYS_PER_YEAR: f64 = 365.242_189_85;

/// What SE's corona defaults read of a star (FUN_140507070): the class word's luminosity code
/// (bits 11-14) and letter (bits 7-10), the radius, the temperature (+0x1248) and the mass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoronaStar {
    pub lum: u32,
    pub letter: u32,
    pub radius_km: f64,
    pub teff: f64,
    pub mass_earth: f64,
}

impl CoronaStar {
    /// The class word fields FUN_140507070 tests, from a parsed class: luminosity codes 0 hypergiant,
    /// 1 Ia, 3 Ib, 4 II, 5 III, 6 IV, 7 V (brown dwarfs are V with the letters L/T/Y, 7..9), 8 VI,
    /// 9 carbon, 10 Wolf-Rayet, 11 remnants (letter 12 neutron star, 13 black hole, below 12 white
    /// dwarfs). Inferred from how FUN_140507070, FUN_1401df100 and FUN_1404cb030 branch on them;
    /// SE's class parser was not traced.
    pub fn class_code(sc: &crate::star::SpectralClass) -> (u32, u32) {
        use crate::star::{LumClass, StarKind};
        let letter = match sc.letter {
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
        match sc.kind {
            StarKind::WhiteDwarf => (11, 0),
            StarKind::NeutronStar => (11, 12),
            StarKind::BlackHole => (11, 13),
            StarKind::WolfRayet => (10, 0),
            StarKind::Carbon => (9, 6),
            StarKind::BrownDwarf => (7, letter.max(7)),
            StarKind::Subdwarf => (8, letter),
            StarKind::Normal => {
                let lum = match sc.lum_class {
                    LumClass::Hypergiant => 0,
                    LumClass::Ia => 1,
                    LumClass::Ib => 3,
                    LumClass::II => 4,
                    LumClass::III => 5,
                    LumClass::IV => 6,
                    LumClass::V => 7,
                    LumClass::VI => 8,
                    LumClass::VII => 11,
                };
                (lum, letter)
            }
        }
    }
}

/// A star's corona with every key resolved, as FUN_1405481c0 draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corona {
    pub radius_km: f64,
    pub period_days: f64,
    pub brightness: f64,
    pub ray_density: f64,
    pub ray_curv: f64,
}

/// SE's `rnd_tn(mean, sigma, lo, hi)`: a normal draw, redrawn until it falls inside `lo..=hi`.
fn rnd_tn(gauss: &mut impl FnMut(f64, f64) -> f64, mean: f64, sigma: f64, lo: f64, hi: f64) -> f64 {
    for _ in 0..32 {
        let v = gauss(mean, sigma);
        if (lo..=hi).contains(&v) {
            return v;
        }
    }
    mean.clamp(lo, hi)
}

impl Corona {
    /// SE's corona of a star (FUN_140507070): the catalog's `Corona` keys, and for those it leaves
    /// out SE's random defaults (`gauss(mean, sigma)` gives the normal draws). `None` with
    /// `NoCorona` (body flag bit 14) and for black holes, which FUN_140507070 marks `NoCorona`.
    /// - `Radius` 5.5 star radii (4.5..6.5).
    /// - `RayDensity` 3.5 sqrt(Teff / 5880) and `RayCurv` 15 / sqrt(g / 28) clamped to 3..180 (g in
    ///   Earth gravities, the Sun's being 28), so hot stars get dense rays and dense stars straight
    ///   ones; remnants 35 and 150, Wolf-Rayet stars 25 and 20.
    /// - `Brightness` 0.5; Wolf-Rayet stars 1, neutron stars 1.5.
    /// - `Period` 10 days, 1 day for brown dwarfs, 10 days x radius / 37.5 million km for giants and
    ///   carbon stars, 0.01 days for white dwarfs and 1e-4 for neutron stars.
    pub fn resolve(given: Option<&CoronaParams>, no_corona: bool, star: &CoronaStar, gauss: &mut impl FnMut(f64, f64) -> f64) -> Option<Self> {
        if no_corona || (star.lum == 11 && star.letter == 13) || star.teff < 879.339 {
            // FUN_1404c07f0 gives 0 below 879.339 K, so FUN_1405481c0 draws nothing there.
            return None;
        }
        let g = given.cloned().unwrap_or_default();
        let radius_km = g.radius_km.unwrap_or_else(|| rnd_tn(gauss, 5.5, 0.3, 4.5, 6.5) * star.radius_km);
        let remnant = star.lum == 11;
        let wolf_rayet = star.lum == 10;
        let ray_density = g.ray_density.unwrap_or_else(|| {
            if remnant {
                rnd_tn(gauss, 3.5, 0.5, 2.0, 5.0) * 10.0
            } else if wolf_rayet {
                rnd_tn(gauss, 25.0, 2.0, 15.0, 35.0)
            } else {
                rnd_tn(gauss, 3.5, 0.5, 2.0, 5.0) * (star.teff.max(0.0) / 5880.0).sqrt()
            }
        });
        let ray_curv = g.ray_curv.unwrap_or_else(|| {
            if remnant {
                rnd_tn(gauss, 15.0, 2.0, 9.0, 21.0) * 10.0
            } else if wolf_rayet {
                rnd_tn(gauss, 20.0, 2.0, 10.0, 30.0)
            } else {
                // FUN_1404dcbf0: surface gravity in Earth's (mass / (radius / 6371 km)^2).
                let g_earth = star.mass_earth / (star.radius_km / 6371.0).powi(2);
                (rnd_tn(gauss, 15.0, 2.0, 9.0, 21.0) / (g_earth / 28.0).max(1e-12).sqrt()).clamp(3.0, 180.0)
            }
        });
        let neutron = remnant && star.letter == 12;
        let brightness = g.brightness.unwrap_or(if neutron {
            1.5
        } else if wolf_rayet {
            1.0
        } else {
            0.5
        });
        let period_days = match g.period_yr {
            Some(p) => p * CORONA_DAYS_PER_YEAR,
            None => {
                let p = rnd_tn(gauss, 10.0, 3.0, 0.5, 50.0);
                if star.lum <= 5 || star.lum == 9 {
                    p * star.radius_km / 3.75e7
                } else if remnant {
                    p * if neutron { 1.0e-5 } else { 1.0e-3 }
                } else if star.lum == 7 && star.letter >= 7 {
                    p * 0.1
                } else {
                    p
                }
            }
        };
        Some(Self { radius_km, period_days, brightness, ray_density, ray_curv })
    }

    /// The animation phase of the three corona planes (FUN_1405481c0's `EyePos.w`): the time in
    /// periods times 0.001, 0.0012365131 and 0.0007631217, wrapped to 0..1 and scaled by 10000.
    pub fn phases(&self, days: f64) -> [f32; 3] {
        let cycles = days / self.period_days.max(1e-12);
        [0.001, 0.001_236_513_1, 0.000_763_121_7].map(|k| {
            let t = cycles * k;
            ((t - t.floor()) * 10000.0) as f32
        })
    }
}

/// FUN_1404c07f0: the corona's brightness for a star temperature,
/// 10^(0.163017 - 1960.37/T + 5603960/T^2 - 7.60241e9/T^3), with T capped at 1e5 K and 0 below 879.339 K.
pub fn corona_temperature_bright(teff: f64) -> f64 {
    if teff < 879.339 {
        return 0.0;
    }
    let t = teff.min(1.0e5);
    10f64.powf(0.163_017 - 1960.37 / t + 5_603_960.0 / (t * t) - 7.602_41e9 / (t * t * t))
}

/// `CometTail` block. Colours are linear RGB; lengths are in the catalog's units.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CometTailParams {
    /// `GasToDust`, gas-to-dust mass ratio (dimensionless). Default 0.
    pub gas_to_dust: f64,
    /// `MaxLength`, maximum tail length (catalog units). Default 0.
    pub max_length: f64,
    /// `GasColor`, RGB of the gas tail. Default (0, 0, 0).
    pub gas_color: [f64; 3],
    /// `DustColor`, RGB of the dust tail. Default (0, 0, 0).
    pub dust_color: [f64; 3],
    /// `Particles`, number of particles simulated. Default 0.
    pub particles: u32,
    /// `Bright`, overall brightness. Default 1.0.
    pub bright: f64,
    /// `GasBright`, gas-tail brightness multiplier. Default 1.0.
    pub gas_bright: f64,
    /// `DustBright`, dust-tail brightness multiplier. Default 1.0.
    pub dust_bright: f64,
}

impl CometTailParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            gas_to_dust: b.f64("GasToDust").unwrap_or(0.0),
            max_length: b.f64("MaxLength").unwrap_or(0.0),
            gas_color: b.vec3("GasColor").unwrap_or([0.0; 3]),
            dust_color: b.vec3("DustColor").unwrap_or([0.0; 3]),
            particles: b.f64("Particles").unwrap_or(0.0) as u32,
            bright: b.f64("Bright").unwrap_or(1.0),
            gas_bright: b.f64("GasBright").unwrap_or(1.0),
            dust_bright: b.f64("DustBright").unwrap_or(1.0),
        }
    }
}

/// `AccretionDisk` block (black holes, neutron stars, binaries). Most keys are
/// dimensionless or in catalog units; `*_km` keys are kilometres.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccretionDiskParams {
    /// `Brightness`, multiplier. Default 1.0.
    pub brightness: f64,
    /// `Density`, catalog units. Default 0.
    pub density: f64,
    /// `Luminosity`, catalog units. Default 0.
    pub luminosity: f64,
    /// `LuminosityBol`, bolometric luminosity. Default 0.
    pub luminosity_bol: f64,
    /// `AccretionRate`, mass accretion rate (catalog units). Default 0.
    pub accretion_rate: f64,
    /// `InnerRadius`, inner edge in catalog units (often a fraction of the parent radius). Default 0.
    pub inner_radius: f64,
    /// `OuterRadius`, outer edge in catalog units. Default 0.
    pub outer_radius: f64,
    /// `InnerRadiusKm`, inner edge, km. Default 0.
    pub inner_radius_km: f64,
    /// `OuterRadiusKm`, outer edge, km. Default 0.
    pub outer_radius_km: f64,
    /// `Temperature`, K at the inner edge. Default 0.
    pub temperature: f64,
    /// `JetVelocity`, jet speed (fraction of c in catalog units). Default 0.
    pub jet_velocity: f64,
    /// `JetBrightness`, jet brightness multiplier. Default 0.
    pub jet_brightness: f64,
    /// `JetDensity`, jet density (catalog units). Default 0.
    pub jet_density: f64,
    /// `JetStartTemp` / `JetEndTemp`, jet temperature range in K. Default 0.
    pub jet_start_temp: f64,
    pub jet_end_temp: f64,
    /// `JetLength`, catalog units. Default 0.
    pub jet_length: f64,
    /// `DiskNoiseContrast`, contrast of the disk noise (dimensionless). Default 0.
    pub disk_noise_contrast: f64,
    /// `DiskTempContrast`, temperature contrast across the disk (dimensionless). Default 0.
    pub disk_temp_contrast: f64,
    /// `ThicknessPow`, power-law exponent of disk thickness. Default 0.
    pub thickness_pow: f64,
}

impl AccretionDiskParams {
    pub fn from_block(b: &Block) -> Self {
        Self {
            brightness: b.f64("Brightness").unwrap_or(1.0),
            density: b.f64("Density").unwrap_or(0.0),
            luminosity: b.f64("Luminosity").unwrap_or(0.0),
            luminosity_bol: b.f64("LuminosityBol").unwrap_or(0.0),
            accretion_rate: b.f64("AccretionRate").unwrap_or(0.0),
            inner_radius: b.f64("InnerRadius").unwrap_or(0.0),
            outer_radius: b.f64("OuterRadius").unwrap_or(0.0),
            inner_radius_km: b.f64("InnerRadiusKm").unwrap_or(0.0),
            outer_radius_km: b.f64("OuterRadiusKm").unwrap_or(0.0),
            temperature: b.f64("Temperature").unwrap_or(0.0),
            jet_velocity: b.f64("JetVelocity").unwrap_or(0.0),
            jet_brightness: b.f64("JetBrightness").unwrap_or(0.0),
            jet_density: b.f64("JetDensity").unwrap_or(0.0),
            jet_start_temp: b.f64("JetStartTemp").unwrap_or(0.0),
            jet_end_temp: b.f64("JetEndTemp").unwrap_or(0.0),
            jet_length: b.f64("JetLength").unwrap_or(0.0),
            disk_noise_contrast: b.f64("DiskNoiseContrast").unwrap_or(0.0),
            disk_temp_contrast: b.f64("DiskTempContrast").unwrap_or(0.0),
            thickness_pow: b.f64("ThicknessPow").unwrap_or(0.0),
        }
    }
}

/// Every optional sub-block of a body, plus its `NoXxx` switches.
///
/// A `NoXxx` key is a bare flag (`NoRings` alone means true) or `NoXxx true/false`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BodyExtras {
    pub atmosphere: Option<AtmosphereParams>,
    /// All `Clouds` blocks, in file order.
    pub clouds: Vec<CloudsParams>,
    pub ocean: Option<OceanParams>,
    pub aurora: Option<AuroraParams>,
    /// All `Life` blocks, in file order.
    pub life: Vec<LifeParams>,
    pub corona: Option<CoronaParams>,
    pub comet_tail: Option<CometTailParams>,
    pub accretion_disk: Option<AccretionDiskParams>,
    pub no_atmosphere: bool,
    pub no_clouds: bool,
    pub no_ocean: bool,
    pub no_rings: bool,
    pub no_aurora: bool,
    pub no_lava: bool,
    pub no_life: bool,
    pub no_comet_tail: bool,
    pub no_accretion_disk: bool,
    pub no_corona: bool,
}

impl BodyExtras {
    /// Reads the sub-blocks and `NoXxx` flags of one body (planet, moon, star...).
    pub fn from_body_block(b: &Block) -> Self {
        Self {
            atmosphere: b.sub("Atmosphere").map(AtmosphereParams::from_block),
            clouds: CloudsParams::all_from_body(b),
            ocean: b.sub("Ocean").map(OceanParams::from_block),
            aurora: b.sub("Aurora").map(AuroraParams::from_block),
            life: LifeParams::all_from_body(b),
            corona: b.sub("Corona").map(CoronaParams::from_block),
            comet_tail: b.sub("CometTail").map(CometTailParams::from_block),
            accretion_disk: b.sub("AccretionDisk").map(AccretionDiskParams::from_block),
            no_atmosphere: flag(b, "NoAtmosphere"),
            no_clouds: flag(b, "NoClouds"),
            no_ocean: flag(b, "NoOcean"),
            no_rings: flag(b, "NoRings"),
            no_aurora: flag(b, "NoAurora"),
            no_lava: flag(b, "NoLava"),
            no_life: flag(b, "NoLife"),
            no_comet_tail: flag(b, "NoCometTail"),
            no_accretion_disk: flag(b, "NoAccretionDisk"),
            no_corona: flag(b, "NoCorona"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc::parse;

    const EARTH_LIKE: &str = r#"
Atmosphere
{
    Height      60      // km
    Greenhouse  33
    Pressure    1.0
    Density     1.2929
    Adiabat     0.28
    Model       "Earth"
    Bright      10.0
    Opacity     1.0
    SkyLight    3.0
    Composition // values in percent
    {
        N2  77.7729
        O2  20.9
        Ar  0.9
    }
}
Clouds
{
    DiffMap        "Earth/Clouds"
    FlipMap        true
    Height         9.0
    Velocity       60
    Hapke          0.2
    ModulateColor  ( 1.0 1.0 1.0 )
    mainOctaves    6
}
Clouds
{
    DiffMap        "Earth/Clouds-Storm"
    Height         12
}
Ocean
{
    Height      10.896
    Composition
    {
        H2O 100
    }
}
Aurora
{
    Height       100
    TopColor     ( 0.5 0.5 0.5 )
    BottomColor  ( 0.0 1.0 0.0 )
    NorthLat     82
    NorthLon     -113
    NorthRings   3
    NorthBright  0.3
    SouthLat     -63
    SouthRings   2
}
Life
{
    Class  "Organic"
    Type   "Multicellular"
    Biome  "Marine/Terrestrial"
}
Life
{
    Class  "Organic"
}
NoRings true
NoAurora
"#;

    #[test]
    fn atmosphere_reads_keys_and_composition() {
        let b = parse(EARTH_LIKE);
        let a = AtmosphereParams::from_block(b.sub("Atmosphere").unwrap());
        assert_eq!(a.model.as_deref(), Some("Earth"));
        assert_eq!(a.height_km, 60.0);
        assert_eq!(a.pressure_atm, 1.0);
        assert_eq!(a.bright, 10.0);
        assert_eq!(a.hue, None);
        assert_eq!(a.composition.len(), 3);
        assert!((a.composition["N2"] - 77.7729).abs() < 1e-9);
    }

    #[test]
    fn atmosphere_defaults_when_keys_missing() {
        let a = AtmosphereParams::from_block(&Block::default());
        assert_eq!(a.model, None);
        assert_eq!(a.bright, 1.0);
        assert_eq!(a.opacity, 1.0);
        assert!(a.composition.is_empty());
    }

    #[test]
    fn clouds_all_from_body_returns_every_block() {
        let b = parse(EARTH_LIKE);
        let c = CloudsParams::all_from_body(&b);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].diff_map.as_deref(), Some("Earth/Clouds"));
        assert!(c[0].flip_map);
        assert_eq!(c[0].velocity_kmh, 60.0);
        assert_eq!(c[0].modulate_color, [1.0, 1.0, 1.0]);
        assert_eq!(c[0].main_octaves, 6);
        assert_eq!(c[1].height_km, 12.0);
        assert_eq!(c[1].modulate_bright, 1.0);
    }

    #[test]
    fn ocean_and_aurora() {
        let b = parse(EARTH_LIKE);
        let o = OceanParams::from_block(b.sub("Ocean").unwrap());
        assert_eq!(o.height_km, 10.896);
        assert_eq!(o.composition["H2O"], 100.0);

        let au = AuroraParams::from_block(b.sub("Aurora").unwrap());
        assert_eq!(au.height_km, 100.0);
        assert_eq!(au.bottom_color, [0.0, 1.0, 0.0]);
        assert_eq!(au.north.lat_deg, 82.0);
        assert_eq!(au.north.lon_deg, -113.0);
        assert_eq!(au.north.rings, 3);
        assert_eq!(au.north.bright, 0.3);
        assert_eq!(au.south.lat_deg, -63.0);
        assert_eq!(au.south.rings, 2);
        assert_eq!(au.south.bright, 1.0);
    }

    #[test]
    fn life_blocks_are_all_kept() {
        let b = parse(EARTH_LIKE);
        let l = LifeParams::all_from_body(&b);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].kind.as_deref(), Some("Multicellular"));
        assert_eq!(l[0].biome.as_deref(), Some("Marine/Terrestrial"));
        assert_eq!(l[1].biome, None);
    }

    #[test]
    fn body_extras_collects_everything_and_flags() {
        let b = parse(EARTH_LIKE);
        let x = BodyExtras::from_body_block(&b);
        assert!(x.atmosphere.is_some());
        assert_eq!(x.clouds.len(), 2);
        assert!(x.ocean.is_some());
        assert!(x.aurora.is_some());
        assert_eq!(x.life.len(), 2);
        assert!(x.corona.is_none());
        assert!(x.no_rings);
        assert!(x.no_aurora);
        assert!(!x.no_atmosphere);
        assert!(!x.no_clouds);
    }

    #[test]
    fn body_extras_empty_block_is_all_off() {
        let x = BodyExtras::from_body_block(&Block::default());
        assert_eq!(x, BodyExtras::default());
        assert!(x.clouds.is_empty());
        assert!(!x.no_ocean);
    }

    #[test]
    fn star_sub_blocks() {
        let b = parse(
            "Corona\n{\n    Radius 4.0159e+008\n    Brightness 1.4504\n}\n\
             CometTail\n{\n    GasColor ( 0.003 0.009 0.015 )\n    Particles 2500\n}\n\
             AccretionDisk\n{\n    Brightness 0.5\n    InnerRadiusKm 137000\n}\n\
             NoCometTail true\n",
        );
        let x = BodyExtras::from_body_block(&b);
        let c = x.corona.unwrap();
        assert!((c.radius_km.unwrap() - 4.0159e8).abs() < 1.0);
        assert_eq!(c.brightness, Some(1.4504));
        assert_eq!(c.ray_density, None);
        assert!(!x.no_corona);
        let t = x.comet_tail.unwrap();
        assert_eq!(t.gas_color, [0.003, 0.009, 0.015]);
        assert_eq!(t.particles, 2500);
        assert_eq!(t.bright, 1.0);
        let d = x.accretion_disk.unwrap();
        assert_eq!(d.brightness, 0.5);
        assert_eq!(d.inner_radius_km, 137_000.0);
        assert!(x.no_comet_tail);
    }

    /// The mean of every draw, so the defaults come out at SE's central values.
    fn mean(m: f64, _s: f64) -> f64 {
        m
    }

    fn sun() -> CoronaStar {
        CoronaStar { lum: 7, letter: 4, radius_km: 695_700.0, teff: 5880.0, mass_earth: 332_946.0 }
    }

    #[test]
    fn corona_defaults_follow_the_star() {
        let c = Corona::resolve(None, false, &sun(), &mut mean).unwrap();
        assert!((c.radius_km - 5.5 * 695_700.0).abs() < 1.0);
        assert_eq!((c.period_days, c.brightness, c.ray_density), (10.0, 0.5, 3.5));
        // The Sun's gravity is 28 Earth's (FUN_1404dcbf0's normalisation).
        assert!((c.ray_curv - 15.0).abs() < 0.1, "{}", c.ray_curv);
        // A giant: period with the radius, straight rays capped at 180.
        let giant = CoronaStar { lum: 4, letter: 6, radius_km: 6.0e8, teff: 3500.0, mass_earth: 5.0e6 };
        let g = Corona::resolve(None, false, &giant, &mut mean).unwrap();
        assert!((g.period_days - 160.0).abs() < 1e-6);
        assert_eq!(g.ray_curv, 180.0);
        // Wolf-Rayet, brown dwarf, neutron star.
        let wr = Corona::resolve(None, false, &CoronaStar { lum: 10, letter: 0, ..sun() }, &mut mean).unwrap();
        assert_eq!((wr.ray_density, wr.ray_curv, wr.brightness), (25.0, 20.0, 1.0));
        let bd = Corona::resolve(None, false, &CoronaStar { letter: 8, teff: 1200.0, ..sun() }, &mut mean).unwrap();
        assert!((bd.period_days - 1.0).abs() < 1e-9);
        let ns = Corona::resolve(None, false, &CoronaStar { lum: 11, letter: 12, ..sun() }, &mut mean).unwrap();
        assert_eq!((ns.ray_density, ns.ray_curv, ns.brightness), (35.0, 150.0, 1.5));
        assert!((ns.period_days - 1.0e-4).abs() < 1e-12);
    }

    #[test]
    fn corona_keys_and_switches() {
        let b = parse("Corona { Radius 4.0159e+008 Period 0.031437 Brightness 1.4504 RayDensity 17.244 RayCurv 3.9393 }\n");
        let x = BodyExtras::from_body_block(&b);
        let c = Corona::resolve(x.corona.as_ref(), x.no_corona, &sun(), &mut mean).unwrap();
        assert_eq!((c.radius_km, c.brightness, c.ray_density, c.ray_curv), (4.0159e8, 1.4504, 17.244, 3.9393));
        assert!((c.period_days - 0.031437 * CORONA_DAYS_PER_YEAR).abs() < 1e-9);
        let off = BodyExtras::from_body_block(&parse("NoCorona true\n"));
        assert!(off.no_corona);
        assert!(Corona::resolve(None, true, &sun(), &mut mean).is_none());
        // Black holes and stars below 879 K have none.
        assert!(Corona::resolve(None, false, &CoronaStar { lum: 11, letter: 13, ..sun() }, &mut mean).is_none());
        assert!(Corona::resolve(None, false, &CoronaStar { teff: 800.0, ..sun() }, &mut mean).is_none());
        // Out-of-range draws are redrawn, then fall back to the mean.
        let c = Corona::resolve(None, false, &sun(), &mut |_m: f64, _s: f64| 100.0).unwrap();
        assert!((c.radius_km - 5.5 * 695_700.0).abs() < 1.0);
    }

    #[test]
    fn corona_phase_and_temperature_brightness() {
        let c = Corona { radius_km: 1.0, period_days: 10.0, brightness: 0.5, ray_density: 3.5, ray_curv: 15.0 };
        // 10 phase units per period on the first plane.
        let p = c.phases(10.0);
        assert!((p[0] - 10.0).abs() < 1e-3 && (p[1] - 12.365_131).abs() < 1e-3);
        // Wrapped every 1000 periods.
        assert!((c.phases(15_000.0)[0] - 5000.0).abs() < 1e-2);
        assert!((corona_temperature_bright(5880.0) - 0.9).abs() < 0.01);
        assert!(corona_temperature_bright(3000.0) < corona_temperature_bright(20000.0));
        assert_eq!(corona_temperature_bright(800.0), 0.0);
    }
}
