//! SpaceEngine's completion of solid catalog planets that have no `Atmosphere` block, decoded
//! from SpaceEngine.exe 0.991 (docs/formats/climate.md). SE runs, per body:
//!
//! 1. bulk composition (FUN_140503b60 -> FUN_1401aaa10): iron, silicate, carbon and ice
//!    fractions from truncated normals, shaped by density and temperature, then reordered so
//!    the catalog class dominates;
//! 2. spin and tides (FUN_140503780): moment of inertia, tidal Q, primordial spin, tidal
//!    locking (FUN_140509e60), tidal heating (FUN_1405099c0), volcanism and internal heat
//!    (FUN_140512d50);
//! 3. the atmosphere (FUN_1404494a0 -> FUN_1401aea60): a random volatile inventory per gas
//!    species, loss with age and Jeans escape, then a pressure/temperature/phase solve
//!    (FUN_1401afb00) giving pressure, greenhouse warming, albedo, clouds and liquids;
//! 4. the ocean, bump height and classes (FUN_140505680, FUN_1405057e0).
//!
//! The gas species table and the XUV fractions are read from SpaceEngine.exe at runtime.
//! Arithmetic follows the exe's f32/f64 mix so the random streams line up with SE.

use crate::moon_theories::ExeImage;
use crate::surface_gen::{bulk, obj, SeRng, UNSET};

/// Gas species in SE's composition arrays.
pub const N_GAS: usize = 28;
/// Species the phase solve, losses and escape handle (H2 .. C8H18).
pub const N_ACTIVE: usize = 21;
pub const AR: usize = 3;
pub const O2: usize = 6;
pub const NH3: usize = 8;
pub const H2O: usize = 9;
pub const CO2: usize = 11;
pub const CH4: usize = 15;

const SPECIES_VA: u64 = 0x1_4067_4e30;
const SPECIES_SIZE: u64 = 104;
const XUV_HI_VA: u64 = 0x1_4067_6380;
const XUV_LO_VA: u64 = 0x1_4067_63a0;
const ATM_MODELS_VA: u64 = 0x1_4067_a640;

const INV_2_32: f32 = 2.328_306_4e-10;
const EARTH_MASS_KG: f32 = 5.972_365e24;
const R_GAS: f32 = 8.31447;

/// One record of SE's gas table (104 bytes: name[32] + 18 floats).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Species {
    pub name: String,
    pub triple_t: f32,
    pub triple_p: f32,
    pub crit_t: f32,
    pub crit_p: f32,
    /// Boiling point at 1 bar (K).
    pub boil_t: f32,
    /// Latent heat of vaporisation (kJ/mol).
    pub latent_kj: f32,
    pub molar_mass: f32,
    /// Liquid and solid density (g/cm^3).
    pub rho_liquid: f32,
    pub rho_solid: f32,
    /// Antoine coefficients: log10(P / Pa) = a - b / (T + c).
    pub antoine: [f32; 3],
    /// Outgassed inventory (x1e6) for metal-poor and metal-rich bodies.
    pub outgas: [f32; 2],
    /// Inventory kept by a body colder than 1.2x the triple point.
    pub cold: [f32; 2],
    /// Photochemical destruction rate.
    pub loss_rate: f32,
    pub f17: f32,
}

/// Exe tables the climate model reads.
#[derive(Clone, Debug)]
pub struct ClimateTables {
    pub species: Vec<Species>,
    /// XUV share of the bolometric luminosity at subtype 5 and 0 of each type O..M (+ a tail).
    pub xuv_hi: [f32; 8],
    pub xuv_lo: [f32; 8],
    /// Atmosphere scattering models by SE's index (Sun, Venus, Earth, Mars, Titan, ...).
    pub atm_models: Vec<String>,
}

fn f32s(exe: &ExeImage, va: u64, n: usize) -> Result<Vec<f32>, String> {
    let b = exe.slice(va, n * 4).ok_or_else(|| format!("{va:#x} not mapped"))?;
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

impl ClimateTables {
    pub fn from_exe(exe: &ExeImage) -> Result<Self, String> {
        let mut species = Vec::with_capacity(N_GAS);
        for i in 0..N_GAS as u64 {
            let va = SPECIES_VA + i * SPECIES_SIZE;
            let raw = exe.slice(va, 32).ok_or("species table not mapped")?;
            let name = String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or_default()).into_owned();
            let f = f32s(exe, va + 32, 18)?;
            species.push(Species {
                name,
                triple_t: f[0],
                triple_p: f[1],
                crit_t: f[2],
                crit_p: f[3],
                boil_t: f[4],
                latent_kj: f[5],
                molar_mass: f[6],
                rho_liquid: f[7],
                rho_solid: f[8],
                antoine: [f[9], f[10], f[11]],
                outgas: [f[12], f[14]],
                cold: [f[13], f[15]],
                loss_rate: f[16],
                f17: f[17],
            });
        }
        let ok = species[H2O].name == "H2O" && (species[H2O].molar_mass - 18.015).abs() < 0.01 && species[CO2].name == "CO2";
        if !ok {
            return Err(format!("gas table at {SPECIES_VA:#x} does not look like SE 0.991's"));
        }
        let hi = f32s(exe, XUV_HI_VA, 8)?;
        let lo = f32s(exe, XUV_LO_VA, 8)?;
        if (hi[0] - 0.8).abs() > 1e-6 || (lo[0] - 0.9).abs() > 1e-6 {
            return Err("XUV tables do not look like SE 0.991's".into());
        }
        let mut atm_models = Vec::new();
        for i in 0..16u64 {
            let Some(ptr) = exe.u64(ATM_MODELS_VA + i * 8) else { break };
            let Some(raw) = exe.slice(ptr, 16) else { break };
            let name = String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or_default()).into_owned();
            if name.is_empty() {
                break;
            }
            atm_models.push(name);
        }
        if atm_models.len() != 13 || atm_models[2] != "Earth" {
            return Err(format!("atmosphere model table at {ATM_MODELS_VA:#x} does not look like SE 0.991's"));
        }
        Ok(Self { species, xuv_hi: hi.try_into().unwrap(), xuv_lo: lo.try_into().unwrap(), atm_models })
    }

    /// Load from `<install>/system/SpaceEngine.exe`.
    pub fn from_install(install: &std::path::Path) -> Result<Self, String> {
        Self::from_exe(&ExeImage::load(&install.join("system").join("SpaceEngine.exe"))?)
    }

    /// XUV luminosity share of a star (FUN_1401deec0). `lum` is SE's luminosity class (7 = V,
    /// 9 white dwarf, 10 Wolf-Rayet, 11 remnant), `ty` the spectral type (O..M = 0..6) and
    /// `sub10` the subtype x10. Subtypes from 5 on take the next type's subtype-0 value, as in
    /// the exe.
    pub fn xuv_fraction(&self, lum: u32, ty: u32, sub10: u32) -> f32 {
        let mut ty = ty;
        match lum {
            9 => return 0.0,
            10 => ty = 0,
            11 => match ty {
                12 => return 0.99,
                13 | 14 => return 0.0,
                _ => ty = 2,
            },
            12 | 15 => return 0.0,
            _ if ty > 6 => return 0.0,
            _ => {}
        }
        let (hi, lo) = (self.xuv_hi[ty as usize], self.xuv_lo[ty as usize]);
        let x = sub10.min(127) as f32 / 50.0;
        if sub10 < 50 {
            (hi - lo) * x + lo
        } else if x <= 1.0 {
            (self.xuv_lo[ty as usize + 1] - hi) * x + hi
        } else {
            self.xuv_lo[ty as usize + 1]
        }
    }
}

/// SE's class word fields for a parsed spectral class: (luminosity class, type, subtype x10).
pub fn star_class_bits(sc: &crate::star::SpectralClass) -> (u32, u32, u32) {
    use crate::star::{LumClass, StarKind};
    let sub10 = (sc.subclass * 10.0).round().clamp(0.0, 99.0) as u32;
    let ty = match sc.letter {
        'O' => 0,
        'B' => 1,
        'A' => 2,
        'F' => 3,
        'G' => 4,
        'K' => 5,
        'M' => 6,
        'L' => 7,
        'T' => 8,
        'Y' => 9,
        _ => 7,
    };
    match sc.kind {
        StarKind::WhiteDwarf => (9, ty, sub10),
        StarKind::WolfRayet => (10, 0, sub10),
        StarKind::NeutronStar => (11, 13, 0),
        StarKind::BlackHole => (11, 14, 0),
        StarKind::BrownDwarf => (7, ty.max(7), sub10),
        _ => {
            let lum = match sc.lum_class {
                LumClass::Hypergiant => 0,
                LumClass::Ia => 1,
                LumClass::Ib => 3,
                LumClass::II => 4,
                LumClass::III => 5,
                LumClass::IV => 6,
                LumClass::V => 7,
                LumClass::VI => 8,
                LumClass::VII => 9,
            };
            (lum, ty, sub10)
        }
    }
}

/// Main-sequence lifetime in Gyr (FUN_1401dd7e0); 12.87 for remnants, x1.1 for giants.
pub fn star_lifetime_gyr(mass_sol: f32, lum: u32) -> f32 {
    if lum >= 11 {
        return 12.87;
    }
    let mut t = if mass_sol <= 0.7 {
        mass_sol.powf(-1.125 - mass_sol * 3.75) * 10.0
    } else {
        (mass_sol.powf(-3.75) * 10.0).max(0.002)
    };
    if lum <= 6 {
        t *= 1.1;
    }
    t
}

/// Age of a catalog system (FUN_1404f62e0): a uniform fraction of the shortest stellar
/// lifetime, capped at 12.87 Gyr. The fraction is the first draw of the system's stream A
/// ([`crate::system_gen::age_draw`]).
pub fn system_age_gyr(draw: u32, lifetimes: impl IntoIterator<Item = f32>) -> f32 {
    let age = lifetimes.into_iter().fold(12.87f32, f32::min);
    draw as f32 * age * INV_2_32
}

/// Radiative equilibrium and exosphere temperatures (FUN_14050abe0). `flux` and `xuv` are
/// sums of L / d^2 (L_sun, AU) and xuv_fraction * L / d^2.
pub fn equilibrium_temps(flux: f32, xuv: f32, bond_albedo: Option<f32>) -> (f32, f32) {
    let k = bond_albedo.map_or(0.7, |a| 1.0 - a);
    let t_eq = ((k * flux).powf(0.25) * 278.0).max(20.0);
    let t_exo = (1500.0 * (xuv / 0.072).powf(0.105)).max(t_eq);
    (t_eq, t_exo)
}

/// Greenhouse warming (K) of an atmosphere of `p` atm and molar mass `mu` over a surface at
/// `t` K (FUN_1401b0c60).
pub fn greenhouse(p: f32, mu: f32, t: f32) -> f32 {
    let mut k: f32 = if mu < 10.0 {
        3.0
    } else if mu < 20.0 {
        2.34
    } else if mu < 30.0 {
        1.0
    } else if mu < 45.0 {
        0.15
    } else if mu < 100.0 {
        0.05
    } else {
        0.0
    };
    if p >= 70.0 {
        k *= 8.333;
    } else if p >= 50.0 {
        k *= 6.666;
    } else if p >= 30.0 {
        k *= 3.333;
    } else if p >= 10.0 {
        k += k;
    } else if p >= 5.0 {
        k *= 1.5;
    }
    (((k * 0.75 + 1.0).powf(0.25) - 1.0) * (p.powf(0.4) * 0.43) * t).clamp(0.0, 1000.0)
}

fn sat(x: f32) -> f32 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}

/// `rnd_gauss` in double precision (FUN_140441d10): 12 draws of stream B.
fn gauss_d(r: &mut SeRng, mean: f64, sigma: f64) -> f64 {
    let mut s = 0.0f64;
    for _ in 0..12 {
        r.b = r.b.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
        s += r.b as f64;
    }
    (s * 2.328_306_437e-10 - 6.0) * sigma + mean
}

fn draw_a(r: &mut SeRng) -> u32 {
    r.a = r.a.wrapping_mul(0x0019_660d).wrapping_add(0x3c6e_f35f);
    r.a
}

// ---------------------------------------------------------------------------------------
// Bulk composition
// ---------------------------------------------------------------------------------------

/// Bulk composition fractions in SE's order (record +0x1468).
pub mod part {
    pub const GAS: usize = 0;
    pub const GAS2: usize = 1;
    pub const ICE: usize = 2;
    pub const SILICATE: usize = 3;
    pub const CARBON: usize = 4;
    pub const IRON: usize = 5;
}

fn normalize6(c: &mut [f32; 6]) {
    let s = c[0] + 0.0 + c[1] + c[2] + c[3] + c[4] + c[5];
    if s != 0.0 {
        let k = 1.0 / s;
        c.iter_mut().for_each(|v| *v *= k);
    }
}

/// Replace the ice (and gas) share and rescale the rocky parts (FUN_1401ab4c0).
fn set_ice(c: &mut [f32; 6], ice: f32) {
    let mut d = c[2] - ice;
    c[2] = ice;
    for i in 0..2 {
        if c[i] > 0.0 {
            d += c[i];
            c[i] = 0.0;
        }
    }
    let mut k = d / (c[3] + c[5] + c[4]) + 1.0;
    if k.is_nan() {
        k = 1.0;
    }
    c[5] *= k;
    c[3] *= k;
    c[4] *= k;
}

/// Bulk class a rocky composition implies (FUN_1401ab690/650/610; 4 for >= 10% ice).
pub fn bulk_from_composition(c: &[f32; 6]) -> Option<i32> {
    let rocky = c[0] + c[1] <= 0.0 && c[2] < 0.1;
    let mut class = None;
    if rocky && c[3] <= c[5] && c[4] <= c[5] {
        class = Some(bulk::FERRIA);
    }
    if rocky && c[5] <= c[3] && c[4] <= c[3] {
        class = Some(bulk::TERRA);
    }
    if rocky && c[5] <= c[4] && c[3] <= c[4] {
        class = Some(bulk::CARBONIA);
    }
    if c[0] + c[1] <= 0.0 && c[2] >= 0.1 {
        class = Some(bulk::AQUARIA);
    }
    class
}

/// Rocky composition (FUN_1401aaa10). `dens` is density / 6 g/cm^3, `t` the equilibrium
/// temperature (0 below 600 K for catalog bodies), `met` the star's metallicity parameter.
fn rocky_composition(r: &mut SeRng, met: f32, dens: f32, t: f32, extra: f32, class: i32) -> [f32; 6] {
    let mut c = [0.0f32; 6];
    c[5] = r.tnormal(0.15, 0.1, 0.0, 0.9);
    c[3] = r.tnormal(0.6, 0.2, 0.0, 1.0);
    c[4] = r.tnormal(0.6, 0.2, 0.0, 1.0);
    let ice0 = r.tnormal(0.15, 0.1, 0.0, 1.0);
    let x = dens + 1.0;
    c[5] *= if x < 0.0 { 0.1 } else if x > 1.0 { 1.0 } else { x * 0.9 + 0.1 };
    let y = dens - 0.8;
    let dense = if y < 0.0 { 1.0 } else if y > 1.0 { 0.1 } else { 1.0 - y * 0.9 };
    c[3] *= dense;
    let warm = sat((t - 200.0) / 400.0);
    c[4] *= warm * warm * dense * sat((met - 0.8) * 5.0);
    let z = dens - 0.3;
    let light = if z < 0.0 { 1.0 } else if z > 1.0 { 0.001 } else { 1.0 - z * 0.999 };
    c[2] = (1.0 - sat((t - 100.0) / 50.0)) * light * ice0;
    let u = draw_a(r) as f32 * INV_2_32;
    if u < 0.6 && t != 0.0 && extra != UNSET && c[2] > 0.0 {
        let a = sat((200.0 - t) / 100.0);
        let b = sat((extra - 0.0001) / 0.0029).powf(0.33);
        let k = 1.0 - (1.0 - sat(((u - 0.6) + 0.1) * 10.0)) * a * (1.0 - b);
        c[5] *= k;
        c[3] *= k;
        c[4] *= k;
    }
    normalize6(&mut c);
    let ice_cap = |r: &mut SeRng, c: &[f32; 6]| {
        let u = draw_a(r);
        if c[2] >= 0.1 {
            u as f32 * INV_2_32 * 0.1
        } else {
            c[2]
        }
    };
    match class {
        bulk::FERRIA => {
            let ice = ice_cap(r, &c);
            set_ice(&mut c, ice);
            let mut top = c[5];
            if top < c[3] {
                (c[5], c[3], top) = (c[3], top, c[3]);
            }
            if top < c[4] {
                (c[5], c[4]) = (c[4], top);
            }
        }
        bulk::TERRA => {
            let ice = ice_cap(r, &c);
            set_ice(&mut c, ice);
            let mut top = c[3];
            if top < c[4] {
                (c[3], c[4], top) = (c[4], top, c[4]);
            }
            if top < c[5] {
                (c[3], c[5]) = (c[5], top);
            }
        }
        bulk::CARBONIA => {
            let ice = ice_cap(r, &c);
            set_ice(&mut c, ice);
            let mut top = c[4];
            if top < c[3] {
                (c[4], c[3], top) = (c[3], top, c[3]);
            }
            if top < c[5] {
                (c[4], c[5]) = (c[5], top);
            }
        }
        bulk::AQUARIA => {
            let u = draw_a(r);
            let ice = if c[2] < 0.1 { (u as f32 * INV_2_32 * 0.2 + 1.0) * 0.1 } else { c[2] };
            set_ice(&mut c, ice);
        }
        _ => {}
    }
    c
}

/// Thickness (km) of the ice layer a body's ice share makes (FUN_1401ab120).
pub fn ice_layer_km(c: &[f32; 6], radius_km: f32, mass_earth: f32) -> f32 {
    if c[2] == 0.0 {
        return 0.0;
    }
    let v = (c[2] * mass_earth * EARTH_MASS_KG / 1000.0 * 1e-9) as f64;
    let inner = ((radius_km * radius_km * radius_km) as f64 - v * 0.238_732_414_6).cbrt();
    radius_km - (inner as f32).max(0.0)
}

// ---------------------------------------------------------------------------------------
// Gas phases
// ---------------------------------------------------------------------------------------

/// Composition and state arrays of the atmosphere (record +0x1480): gas, liquid and ice
/// inventories per species, then the totals the solve keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Volatiles {
    pub gas: [f32; N_GAS],
    pub liquid: [f32; N_GAS],
    pub ice: [f32; N_GAS],
    pub gas_sum: f32,
    pub liquid_sum: f32,
    pub ice_sum: f32,
    /// Total volatile inventory in Earth atmosphere masses.
    pub atm_mass: f32,
    /// Mean liquid density (g/cm^3).
    pub liquid_density: f32,
    pub ocean_depth_km: f32,
    pub ocean_fraction: f32,
    pub cloud_cover: f32,
    pub ice_fraction: f32,
    pub albedo: f32,
    /// Bit 0: global water ocean on a water-rich body; bit 1: water boiled off; bits 4..7 the
    /// ocean floor kind.
    pub flags: u32,
}

impl Default for Volatiles {
    fn default() -> Self {
        Self {
            gas: [0.0; N_GAS],
            liquid: [0.0; N_GAS],
            ice: [0.0; N_GAS],
            gas_sum: 0.0,
            liquid_sum: 0.0,
            ice_sum: 0.0,
            atm_mass: 0.0,
            liquid_density: 0.0,
            ocean_depth_km: 0.0,
            ocean_fraction: 0.0,
            cloud_cover: 0.0,
            ice_fraction: 0.0,
            albedo: 0.0,
            flags: 0,
        }
    }
}

fn sum28(a: &[f32; N_GAS]) -> f32 {
    a.iter().fold(0.0f32, |s, v| s + v)
}

/// Saturation vapour pressure (atm) of species `s` at `t` K (FUN_1401b2860).
pub fn vapour_pressure(tab: &ClimateTables, s: usize, t: f32) -> f32 {
    let td = t as f64;
    let log10_pa = match s {
        H2O if t <= 273.16 => {
            let x = (t.ln() as f64 * 6.1295027 + ((-5504.4088 / td - 3.5704628) - td * 0.017337458) + td * 6.5204209e-06 * td) as f32;
            if x < -60.0 {
                return 0.0;
            }
            return x.exp() / 101325.0;
        }
        H2O if t <= 333.16 => (10.2326 - 1750.286 / (td - 38.1)) as f32,
        H2O => (10.0917 - 1668.21 / (td - 45.1)) as f32,
        CO2 => {
            let x = ((168.4011 - 7062.404 / td + td * 3.368548e-05 * td) - t.ln() as f64 * 24.03761) as f32;
            if x < -60.0 {
                return 0.0;
            }
            return x.exp() / 760.0;
        }
        NH3 => (9.67956 - 1002.711 / (td - 25.215)) as f32,
        CH4 => {
            let x = (6.61184 - 389.93 / (td - 7.15)) as f32;
            if x < -26.0 {
                return 0.0;
            }
            return 10f32.powf(x) / 760.0;
        }
        _ => {
            let a = &tab.species[s].antoine;
            if t + a[2] <= 0.001 {
                return 0.0;
            }
            a[0] - a[1] / (t + a[2])
        }
    };
    if log10_pa < -26.0 {
        return 0.0;
    }
    10f32.powf(log10_pa) / 101325.0
}

/// Split each species' inventory `inv` into gas, liquid and ice at `p` atm and `t` K
/// (FUN_1401b0370). `water_rich` turns water into a global ocean or steam.
fn partition(tab: &ClimateTables, v: &mut Volatiles, p: f32, t: f32, inv: &[f32; N_GAS], water_rich: bool) {
    let x = p + 0.001;
    let total = sum28(&v.gas);
    v.flags &= !3;
    for s in 0..N_ACTIVE {
        let inv_s = inv[s];
        if inv_s == 0.0 {
            v.liquid[s] = 0.0;
            continue;
        }
        let sp = &tab.species[s];
        let pv = vapour_pressure(tab, s, t);
        let partial = (p * v.gas[s]) / total;
        if pv == 0.0 || partial == 0.0 {
            v.gas[s] = 0.0;
            v.ice[s] = inv_s;
            v.liquid[s] = 0.0;
            continue;
        }
        let l = (R_GAS / sp.latent_kj) * 0.001;
        let mut tb = 1.0 / (1.0 / sp.boil_t - x.ln() * l);
        if sp.crit_t <= tb {
            tb = sp.crit_t;
        }
        // 0 solid, 1 liquid, 2 gas, 3 supercritical
        let phase = if sp.triple_p <= p {
            if sp.crit_p <= p {
                if sp.triple_t <= t {
                    if sp.crit_t <= t {
                        if (sp.crit_p / sp.crit_t) * (t - sp.crit_t) + sp.crit_p <= p {
                            3
                        } else {
                            2
                        }
                    } else {
                        1
                    }
                } else {
                    0
                }
            } else if sp.triple_t <= t {
                if tb <= t {
                    2
                } else {
                    1
                }
            } else {
                0
            }
        } else if t < sp.triple_t {
            let q = (t / sp.triple_t) * (t / sp.triple_t);
            let p_sub = if q < 0.0 {
                0.0
            } else if q <= 1.0 {
                sp.triple_p * q
            } else {
                sp.triple_p
            };
            if p < p_sub {
                2
            } else {
                0
            }
        } else {
            2
        };
        let frac = sat(((t - sp.triple_t) / (tb - sp.triple_t).max(1.0)) * 0.9 + 0.1);
        if s == H2O && water_rich {
            if phase == 1 {
                v.liquid[s] = inv_s;
                v.ice[s] = 0.0;
                v.gas[s] = (total * partial) / p;
                v.flags |= 1;
                continue;
            }
            if phase == 3 || tb < t {
                let g = (1.0 / sp.boil_t - 1.0 / t).exp();
                v.liquid[s] = 0.0;
                v.ice[s] = 0.0;
                v.gas[s] = ((g / l) * total) / p;
                v.flags |= 2;
                continue;
            }
        }
        let gas = (frac * 0.99 + 0.01) * inv_s * sat(pv / partial);
        let mut rest = inv_s - gas;
        if rest < 1e-20 {
            rest = 0.0;
        }
        v.gas[s] = gas;
        match phase {
            0 => {
                v.ice[s] = rest;
                v.liquid[s] = 0.0;
            }
            1 => {
                v.ice[s] = 0.0;
                v.liquid[s] = rest;
            }
            _ => {
                v.ice[s] = 0.0;
                v.liquid[s] = 0.0;
            }
        }
    }
    v.liquid_sum = 0.0;
    v.ice_sum = 0.0;
    v.liquid_density = 0.0;
    for s in 0..N_ACTIVE {
        if v.liquid[s] > 0.0 {
            v.liquid_sum += v.liquid[s];
            v.liquid_density += tab.species[s].rho_liquid * v.liquid[s];
        }
        if v.ice[s] > 0.0 {
            v.ice_sum += v.ice[s];
        }
    }
    if v.liquid_sum != 0.0 {
        v.liquid_density /= v.liquid_sum;
    }
}

/// Cloud cover from condensing species over an ocean (FUN_1401b0f60).
fn cloud_cover(tab: &ClimateTables, v: &Volatiles, area_km2: f32, t: f32, ocean: f32) -> f32 {
    if ocean == 0.0 {
        return 0.0;
    }
    let k = 1.839e-8 / area_km2;
    let mut c = 0.0f32;
    for s in 0..N_ACTIVE {
        if v.liquid[s] != 0.0 {
            let d = ((t - tab.species[s].triple_t) - 14.0).min(0.0);
            c += (d * 0.0698).exp() * v.gas[s] * 5.2e18 * k;
        }
    }
    sat(c)
}

/// Mean molar mass of the gas phase (FUN_1401ad4b0).
pub fn molar_mass(tab: &ClimateTables, gas: &[f32; N_GAS]) -> f32 {
    let n = sum28(gas);
    if n == 0.0 {
        return 0.0;
    }
    gas.iter().zip(&tab.species).fold(0.0f32, |s, (g, sp)| s + g * sp.molar_mass) / n
}

/// Initial volatile inventory (FUN_1401aeda0): what a body of `t` K keeps cold plus an
/// outgassed amount scaled by 10^U(-1, 1). `met` mixes the poor/rich columns.
fn initial_inventory(tab: &ClimateTables, r: &mut SeRng, v: &mut Volatiles, t: f32, met: f32, water_rich: bool) {
    let f = sat((met - 0.7) * 10.0);
    let mix = |a: [f32; 2]| if f < 0.0 { a[0] } else if f <= 1.0 { (a[1] - a[0]) * f + a[0] } else { a[1] };
    for s in 0..N_ACTIVE {
        let sp = &tab.species[s];
        v.gas[s] = if t < sp.triple_t * 1.2 || (water_rich && s == H2O) { mix(sp.cold) } else { 0.0 };
    }
    for s in 0..N_ACTIVE {
        let u = draw_a(r) as f32;
        let k = 10f32.powf((u + u) * INV_2_32 - 1.0);
        v.gas[s] = k * (mix(tab.species[s].outgas) * 1_000_000.0 + v.gas[s]);
    }
    for g in &mut v.gas[N_ACTIVE..] {
        *g = 0.0;
    }
}

/// Loss over `age` Gyr by photochemistry (FUN_1401b1380); scales the pressure.
fn age_losses(tab: &ClimateTables, v: &mut Volatiles, p: &mut f32, t: f32, age: f32) {
    let before = sum28(&v.gas);
    let half = age * 0.5;
    for s in 0..N_ACTIVE {
        let rate = tab.species[s].loss_rate;
        if v.gas[s] == 0.0 || rate == 0.0 {
            continue;
        }
        let keep = 1.0 / (rate + 1.0);
        let warm_old = age > 2.0 && t > 270.0 && t < 400.0;
        let k = match s {
            AR => age * 0.0375,
            O2 if warm_old => (keep as f64).powf((half as f64).powf(0.25) * (*p * 0.25 + 0.89) as f64) as f32,
            CO2 if warm_old => keep.powf(half.sqrt() * (*p + 0.75)) * 1.5,
            _ => keep.powf((*p + 0.75) * half),
        };
        v.gas[s] *= k;
    }
    *p = (sum28(&v.gas) / before) * *p;
    if *p > 1e6 {
        *p = 1e6;
    }
}

/// Jeans escape over `age` Gyr (FUN_1401b1a00).
fn escape(tab: &ClimateTables, v: &mut Volatiles, p: &mut f32, t_exo: f32, radius_km: f32, g: f32, age: f32) {
    let before = sum28(&v.gas);
    let tk = t_exo * 24943.41;
    for s in 0..N_ACTIVE {
        if v.gas[s] == 0.0 {
            continue;
        }
        let vel = (tk / tab.species[s].molar_mass).sqrt();
        let lambda = (g * 3000.0 * radius_km) / (vel * vel);
        if lambda < 80.0 {
            let tau = (lambda.exp() * ((vel * vel * vel) / (g * 2000.0 * g * radius_km))) / 3.155_7e16;
            let x = -age / tau;
            v.gas[s] = if x >= -20.0 { x.exp() * v.gas[s] } else { 0.0 };
        }
    }
    *p = (sum28(&v.gas) / before) * *p;
    if *p > 1e6 {
        *p = 1e6;
    }
}

/// Drop trace amounts below 1e-7 of the phase total (FUN_1401b2210).
fn prune(v: &mut Volatiles) {
    for (arr, total) in [(&mut v.gas, v.gas_sum), (&mut v.liquid, v.liquid_sum), (&mut v.ice, v.ice_sum)] {
        let cut = total * 1e-7;
        if cut != 0.0 {
            for x in arr.iter_mut() {
                if *x < cut {
                    *x = 0.0;
                }
            }
        }
    }
}

/// What the climate solve needs from the body.
struct SolveBody {
    radius_km: f32,
    mass_earth: f32,
    gravity: f32,
    flux: f32,
    heat: f32,
    bulk: i32,
    /// Keep the catalog Bond albedo (record flag 0x100000).
    albedo_given: bool,
    composition: [f32; 6],
}

/// Global ocean depth (km) a liquid inventory makes (`5.2e18` kg per Earth atmosphere).
fn liquid_depth_km(v: &Volatiles, area_km2: f32) -> f32 {
    (v.liquid_sum * 5.2e18 / (v.liquid_density * 1000.0)) / (area_km2 * 1e9)
}

/// Pressure/temperature/phase iteration (FUN_1401afb00). Returns whether it converged.
fn climate_solve(tab: &ClimateTables, v: &mut Volatiles, b: &SolveBody, p: &mut f32, t: &mut f32, gh: &mut f32, mu: &mut f32, water_rich: bool) -> bool {
    let inv = v.gas;
    let p_init = *p;
    let area = ((b.radius_km * b.radius_km) as f64 * 12.566_370_61) as f32;
    let k278 = 278f64.powf(4.0);
    let (mut p0, mut t0) = (*p, *t);
    let (mut p_new, mut t_new);
    let mut iter = 0;
    let converged = loop {
        partition(tab, v, p0, t0, &inv, water_rich);
        p_new = (sum28(&v.gas) * p_init) / v.atm_mass;
        if !b.albedo_given {
            let ocean = if v.liquid_sum > 0.0 {
                if v.flags & 1 == 0 {
                    sat((liquid_depth_km(v, area) * 0.71) / 2.72)
                } else {
                    1.0
                }
            } else {
                0.0
            };
            v.ocean_fraction = ocean;
            let cloud = cloud_cover(tab, v, area, t0, ocean);
            v.cloud_cover = cloud;
            let ice = if v.liquid[H2O] == 0.0 { 0.0 } else { sat(((328.0 - t0) / 90.0).powf(5.0).min(ocean * 1.5)) };
            v.ice_fraction = ice;
            let (mut land, mut sea, mut cap) = (1.0 - ocean - ice, ocean, ice);
            let n = u8::from(sea > 0.0) + u8::from(cap > 0.0) + u8::from(land > 0.0);
            let share = cloud / n as f32;
            for x in [&mut land, &mut sea, &mut cap] {
                *x = if *x <= share { 0.0 } else { *x - share };
            }
            let a = if p_new > 1e-6 {
                cloud * 0.52 + land * if water_rich { 0.6 } else { 0.15 } + sea * 0.04 + cap * 0.7
            } else if !water_rich {
                cloud * 0.52 + land * 0.07 + cap * 0.43
            } else {
                cloud * 0.52 + land * 0.43 + cap * 0.67
            };
            let thick = sat((p_new - 100.0) / 900.0);
            let thick = thick * thick;
            v.albedo = (0.3 - a) * thick + a;
        }
        *mu = molar_mass(tab, &v.gas);
        let teff = ((((1.0 - v.albedo) as f64 * k278 * b.flux as f64) + (b.heat * 1_048_576.0) as f64) as f32).powf(0.25).min(3500.0);
        let g = greenhouse(p_new, *mu, teff);
        t_new = teff + g;
        *gh = g;
        if (iter != 0 && (p0 - p_new).abs() <= (p0 + p_new).abs() * 0.01 && (t0 - t_new).abs() <= (t0 + t_new).abs() * 0.01) || p_new < 1e-10 {
            break true;
        }
        iter += 1;
        p0 += (p_new - p0) * 0.45;
        t0 += (t_new - t0) * 0.45;
        if iter > 49 {
            p_new = p0;
            t_new = t0;
            break false;
        }
    };
    partition(tab, v, p_new, t_new, &inv, water_rich);
    // With no liquid SE tests the stale register, which holds the pressure iterate.
    let mut depth_test = p0;
    if v.liquid_sum <= 0.0 {
        v.ocean_depth_km = 0.0;
    } else if v.flags & 1 == 0 {
        v.ocean_depth_km = (liquid_depth_km(v, area) * 5.0) / 2.72;
    } else {
        v.ocean_depth_km = ice_layer_km(&b.composition, b.radius_km, b.mass_earth);
        if v.ocean_depth_km < 25.0 {
            v.flags &= !1;
        }
    }
    if v.liquid_sum > 0.0 {
        depth_test = v.ocean_depth_km;
    }
    if depth_test >= 0.01 {
        ocean_floor(v, b, p_new, t_new);
    } else {
        v.ocean_depth_km = 0.0;
        v.liquid_sum = 0.0;
        v.liquid = [0.0; N_GAS];
        v.flags &= 0xffff_ff0e;
    }
    v.gas_sum = sum28(&v.gas);
    *p = p_new;
    *t = t_new;
    converged
}

/// Cap the ocean at the depth where the pressure reaches 1 GPa and set the floor kind
/// (FUN_1401ae140).
fn ocean_floor(v: &mut Volatiles, b: &SolveBody, p: f32, t: f32) {
    let cap = if v.liquid_density > 0.0 { (1e9 - p * 101325.0) / (b.gravity * v.liquid_density * 1_000_000.0) } else { 1e38 };
    let flags = v.flags & 0xffff_ff0f;
    v.flags = if cap <= v.ocean_depth_km {
        v.ocean_depth_km = cap;
        if v.liquid[H2O] > 0.0 || b.bulk == bulk::AQUARIA {
            flags | if t >= 600.0 { 0x80 } else { 0x40 }
        } else {
            flags | 0x10
        }
    } else {
        flags | if b.bulk == bulk::AQUARIA { 0x20 } else { 0x10 }
    };
}

// ---------------------------------------------------------------------------------------
// The completion
// ---------------------------------------------------------------------------------------

/// A solid catalog body as SE's completion sees it. Units as SE: km, Earth masses, K, Gyr, AU,
/// days. `flux`/`xuv` are host-star sums (see [`equilibrium_temps`]).
#[derive(Clone, Debug)]
pub struct BodyIn {
    pub seeds: (u32, u32),
    pub obj_type: i32,
    /// Catalog class (`bulk::NONE` when none is given).
    pub bulk: i32,
    pub mass_earth: f32,
    pub radius_km: f32,
    pub bond_albedo: Option<f32>,
    pub age_gyr: f32,
    /// Host star's `CtoO` (carbon-to-oxygen ratio); the body's own is it times U(0.8, 1.2).
    pub star_c_to_o: Option<f32>,
    pub flux: f32,
    pub xuv: f32,
    /// Flux at the mean distance (mode 2), which the temperature model reads.
    pub flux_mean: f32,
    /// Orbit around the parent: semi-major axis (AU), eccentricity, period (days), parent mass
    /// (Earth masses). `None` for rogue bodies.
    pub orbit: Option<(f64, f64, f64, f32)>,
    /// Orbit inclination (degrees); SE adds it to a drawn obliquity, and a moon takes it as its own.
    pub orbit_incl_deg: f64,
    /// Orbit's ascending node (degrees); the equator's node is drawn around it.
    pub orbit_node_deg: f64,
    /// A moon whose catalog orbit omits the inclination / the node: SE draws them in the moon's
    /// completion ([`crate::system_gen::moon_angles`]), ahead of everything else.
    pub orbit_angles_drawn: [bool; 2],
    /// The parent is a star or a barycentre of stars, so a locked body keeps a day and a night side.
    pub star_parent: bool,
    /// Catalog values that stop SE drawing its own (they change which random numbers follow).
    pub obliquity_deg: Option<f64>,
    pub eq_node_deg: Option<f64>,
    pub rotation_days: Option<f64>,
    pub tidally_locked: bool,
    pub no_ocean: bool,
    pub no_clouds: bool,
    /// The catalog's `Interior` composition (fractions in SE's order: hydrogen, helium, ices,
    /// silicates, carbides, metals), which SE keeps instead of drawing one.
    pub composition: Option<[f32; 6]>,
    /// A body SE's system generator made: its tidal heating is capped (FUN_1405099c0 with its last
    /// argument 0), where catalog bodies' is not.
    pub procedural: bool,
}

/// What SE's completion fills in.
#[derive(Clone, Debug, Default)]
pub struct Completion {
    pub bulk: i32,
    pub composition: [f32; 6],
    pub bond_albedo: f32,
    pub t_eq: f32,
    pub t_exo: f32,
    pub t_surf: f32,
    /// Average temperature behind the class word (record +0x1860, [`class_temperature`]).
    pub t_class: f32,
    /// The temperature model's inputs ([`display`] reads them too).
    pub thermal: Option<ThermalIn>,
    pub moment_of_inertia: f32,
    pub tidal_q: f32,
    /// Tidal heating (W) and internal heat (1 = Earth's, adds (32 K)^4 to T^4).
    pub tidal_heat_w: f32,
    pub heat: f32,
    pub volcano_activity: f32,
    pub lava_coverage: f32,
    /// Spin: tidally locked, and the period (days) SE derives.
    pub locked: bool,
    pub rotation_days: Option<f64>,
    /// Spin axis (radians): obliquity and the equator's ascending node, in the orbit's reference
    /// frame (the orbit's own inclination and node put the axis along the orbit's normal).
    pub obliquity_rad: f64,
    pub eq_node_rad: f64,
    pub pressure_atm: f32,
    /// Pressure SE's info panel shows: the surface's, or a giant's at depth (its model keeps the
    /// 1 atm level as the surface).
    pub panel_pressure_atm: f32,
    pub greenhouse_k: f32,
    pub molar_mass: f32,
    pub gravity: f32,
    pub density_kg_m3: f32,
    pub scale_height_km: f32,
    /// Visual atmosphere height (km): where the pressure falls to 0.001 atm, 0.5-5% of R.
    pub atm_height_km: f32,
    pub volatiles: Volatiles,
    pub has_ocean: bool,
    pub ocean_depth_km: f32,
    pub bump_km: f32,
    pub has_clouds: bool,
    /// SE's generated cloud layers, bottom up ([`cloud_layers`]): a terrestrial body's 1 to
    /// [`PLANET_CLOUDS_TERRA`]. Empty for giants, whose clouds SE draws in the surface.
    pub clouds: Vec<CloudGen>,
    pub converged: bool,
}

/// main-def.cfg `PlanetCloudsTerra` (DAT_14075f814): the most cloud layers SE gives a terrestrial
/// body. (`PlanetCloudsGasGiant` is read but no code uses it; giants get 1 or 2.)
pub const PLANET_CLOUDS_TERRA: u32 = 3;

/// One of SE's generated cloud layers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CloudGen {
    /// Height above the radius (km).
    pub height_km: f32,
    /// Relief of the cloud tops (the layer's `BumpHeight`, km).
    pub bump_km: f32,
    /// Wind (`Velocity`, km/h).
    pub velocity_kmh: f32,
}

/// What SE's cloud layers of a terrestrial body depend on.
#[derive(Clone, Copy, Debug)]
pub struct CloudsIn {
    pub radius_km: f32,
    /// `BumpHeight` (km).
    pub bump_km: f32,
    /// Ocean height (km), 0 without an ocean: SE's `BumpOffset`, which puts the sea at the radius.
    pub sea_km: f32,
    /// Atmosphere height (km).
    pub atm_height_km: f32,
    pub pressure_atm: f32,
    /// Surface temperature (K).
    pub t_surf: f32,
}

/// SE's cloud layers of a terrestrial body without `Clouds` blocks. FUN_1405057e0: `1 + u_count %
/// PlanetCloudsTerra` layers (one under an atmosphere thinner than 0.5% of the radius), spread
/// evenly over a span from the cloud base (mid-relief, or the sea when that is higher):
/// `min(2 n U(2, 3), U(0.2, 0.3) H)` km, or `U(0.2, 0.3) H` alone under a dense atmosphere (over 1 atm
/// below 100 K, over 10 atm above), the lowest layer one step above the base. Each layer's relief is
/// `U(5, 7) clamp(R / 6371, 0.5, 10)` km, at most half the step. FUN_140510f40 then gives each layer of
/// a round body a wind of `rnd_tn(100, 50, 2, 500)` km/h on stream B. Stream A draws in SE's order:
/// the span (2), the surface's +0x1170 (1, unused here), one per layer.
pub fn cloud_layers(r: &mut SeRng, u_count: u32, c: CloudsIn) -> Vec<CloudGen> {
    let mut n = if PLANET_CLOUDS_TERRA == 0 { 1 } else { 1 + u_count % PLANET_CLOUDS_TERRA };
    if c.atm_height_km < c.radius_km * 0.005 {
        n = 1;
    }
    let bump = c.bump_km.max(1e-6);
    let base = (c.sea_km / bump).max(0.5) * bump - c.sea_km;
    let u1 = draw_a(r);
    let u2 = draw_a(r);
    let atm_span = (u1 as f32 * 0.1 * INV_2_32 + 0.2) * c.atm_height_km;
    let n_span = (u2 as f32 * INV_2_32 + 2.0) * n as f32 * 2.0;
    let dense = (c.t_surf < 100.0 && c.pressure_atm > 1.0) || (c.t_surf >= 100.0 && c.pressure_atm > 10.0);
    let span = if dense { atm_span } else { n_span.min(atm_span) };
    let step = span / n as f32;
    let _ = draw_a(r);
    let k = (c.radius_km / 6371.0).clamp(0.5, 10.0);
    let mut out: Vec<CloudGen> = (1..=n)
        .map(|i| {
            let u = draw_a(r) as f32;
            CloudGen {
                height_km: i as f32 * step + base,
                bump_km: (((u + u) * INV_2_32 + 5.0) * k).min(step * 0.5),
                velocity_kmh: 0.0,
            }
        })
        .collect();
    for l in &mut out {
        l.velocity_kmh = r.tnormal(100.0, 50.0, 2.0, 500.0);
    }
    out
}

impl Completion {
    pub fn has_atmosphere(&self) -> bool {
        self.pressure_atm > 1e-9
    }

    /// The oxygen share (0..1) of the gas phase.
    pub fn o2_share(&self) -> f32 {
        let n = sum28(&self.volatiles.gas);
        if n > 0.0 { self.volatiles.gas[O2] / n } else { 0.0 }
    }

    /// Gas-phase composition in volume percent, largest first.
    pub fn gas_percent<'a>(&self, tab: &'a ClimateTables) -> Vec<(&'a str, f32)> {
        let n = sum28(&self.volatiles.gas);
        let mut v: Vec<_> = (0..N_GAS)
            .filter(|&s| self.volatiles.gas[s] > 0.0 && n > 0.0)
            .map(|s| (tab.species[s].name.as_str(), self.volatiles.gas[s] / n * 100.0))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v
    }

    /// SE's ocean as the volatiles class sees it.
    pub fn ocean_info(&self) -> Option<crate::surface_gen::OceanInfo> {
        self.has_ocean.then(|| {
            let global = self.volatiles.flags & 1 != 0;
            crate::surface_gen::OceanInfo {
                depth_km: self.ocean_depth_km,
                bump_height_km: self.bump_km,
                global,
                global_shallow: global && self.volatiles.flags & 0x10 != 0,
            }
        })
    }
}

/// SE's `BumpHeight` default for a body without one (FUN_1404d60e0 in FUN_1405057e0):
/// 20 km / sqrt(g / g_earth), at most 20 km and 5% of the radius (90% for asteroids).
pub fn se_bump_default_km(mass_earth: f32, radius_km: f32, asteroid: bool) -> f32 {
    let g = mass_earth / ((radius_km / 6371.0) * (radius_km / 6371.0));
    let b = (20.0 / g.sqrt()).min(20.0);
    b.min(radius_km * if asteroid { 0.9 } else { 0.05 })
}

/// SE's `AtmoHeightHarb` (main-def.cfg, default true: "use HarbingerDawn's formula to calculate
/// visual model height"); off, a rocky body's atmosphere reaches up to its 0.001-atm level.
const ATMO_HEIGHT_HARB: bool = true;

/// The visual height (km) SE gives a rocky body's atmosphere without a catalog `Height`
/// (FUN_140448ef0 with `AtmoHeightHarb`): p^0.1 x 7.8 scale heights, at least twice SE's default
/// `BumpHeight` and at most 10% of the radius.
pub fn se_visual_atm_height_km(pressure_atm: f32, scale_height_km: f32, mass_earth: f32, radius_km: f32) -> f32 {
    let h = pressure_atm.max(0.0).powf(0.1) * scale_height_km * 7.8;
    let g = mass_earth / ((radius_km / 6371.0) * (radius_km / 6371.0));
    let bump = if g > 0.0 { (20.0 / g.sqrt()).min(20.0) } else { 20.0 };
    h.max(bump * 2.0).min(radius_km * 0.1)
}

/// Height (km) where the pressure falls to `p` atm (FUN_14044a050); `h02` is the 0.2 atm height.
fn height_at(p0: f32, p: f32, scale_h: f32, gamma: f32, t: f32, g: f32, mu: f32, h02: f32) -> f32 {
    if p0 <= 0.0 {
        0.0
    } else if p0 < 0.2 {
        (-(p / p0).ln() * scale_h).max(0.0)
    } else if p < 0.2 {
        h02 - (((0.2 / p0).powf(gamma) * (t * R_GAS)) / (g * mu)) * (p / 0.2).ln()
    } else {
        ((1.0 - (p / p0).powf(gamma)) * scale_h) / gamma
    }
}

/// The body's C/O parameter (record +0x1614, FUN_140503b60): the host star's `CtoO` times
/// U(0.8, 1.2), unset without one. Stream A's first value of the body's seeds.
pub fn body_c_to_o(b: &BodyIn) -> f32 {
    let mut r = SeRng::new(b.seeds.0, b.seeds.1);
    let u = draw_a(&mut r) as f32;
    b.star_c_to_o.map_or(UNSET, |c| (1.2 - u * 0.4 * INV_2_32) * c)
}

/// Giant composition (FUN_1401aa900, then FUN_1401aa7a0 for Jupiters or FUN_1401aa850 for
/// Neptunes): a heavy core of ice, rock, carbon (with C/O above 0.8) and iron making up a drawn
/// share of the mass, the rest hydrogen and 7.5-11% helium.
fn giant_composition(r: &mut SeRng, c_to_o: f32, jupiter: bool) -> [f32; 6] {
    let mut c = [0.0f32; 6];
    c[part::ICE] = r.tnormal(0.75, 0.02, 0.65, 0.85);
    c[part::SILICATE] = r.tnormal(0.15, 0.01, 0.05, 0.25);
    c[part::IRON] = r.tnormal(0.05, 0.005, 0.01, 0.1);
    let carbon = sat((c_to_o - 0.8) * 5.0);
    c[part::CARBON] = r.tnormal(0.35, 0.05, 0.05, 0.55) * carbon;
    normalize6(&mut c);
    let heavy = if jupiter { r.tnormal(0.4, 0.2, 0.05, 0.8) } else { r.tnormal(0.85, 0.2, 0.6, 0.95) };
    for x in &mut c[part::ICE..] {
        *x *= heavy;
    }
    let he = r.tnormal(0.085, 0.002, 0.075, 0.11);
    let gas = 1.0 - heavy;
    c[part::GAS] = (1.0 - he) * gas;
    c[part::GAS2] = gas * he;
    c
}

/// Atmosphere of a giant (FUN_140449860, inventory FUN_1401ae280/FUN_1401ae690). Returns the
/// pressure at depth (atm), the temperature there and the molar mass; SE's reference level stays
/// at 1 atm. The gas share of the mass grows with log10 M (one stream-A draw), the gases start
/// from the species' nebular columns times 1.15^U(-1, 1), then lose to Jeans escape.
#[allow(clippy::too_many_arguments)]
fn giant_atmosphere(tab: &ClimateTables, r: &mut SeRng, v: &mut Volatiles, m: f32, rad: f32, g: f32, t: f32, t_exo: f32, age: f32, met: f32) -> (f32, f32, f32) {
    let x = (m.log10() - 1.0).powi(3);
    let share0 = if x < 0.0 { 0.05 } else if x <= 1.0 { x * 0.9 + 0.05 } else { 0.95 };
    let share = ((draw_a(r) as f32 * INV_2_32 * 0.1 + 0.95) * share0).clamp(0.05, 0.95);
    let p_deep = (((m * 1_250_000.0 * m) as f64 / ((rad / 6378.14) as f64).powf(4.0)) as f32 * share).min(1e6);
    let t_deep = (t / (1.0 / p_deep).powf(0.28)).min(5000.0);
    v.atm_mass = (share * m) / 8.7073e-07;
    let f = sat((met - 0.7) * 10.0);
    for s in 0..N_ACTIVE {
        let [lo, hi] = tab.species[s].cold;
        let u = draw_a(r) as f32;
        v.gas[s] = 1.15f32.powf((u + u) * INV_2_32 - 1.0) * ((hi - lo) * f + lo);
    }
    let before = sum28(&v.gas);
    let mut p = 1.0;
    escape(tab, v, &mut p, t_exo, rad, g, age);
    let after = sum28(&v.gas);
    v.atm_mass *= after / before;
    v.gas_sum = v.atm_mass;
    let k = v.atm_mass / after;
    v.gas.iter_mut().for_each(|x| *x *= k);
    (p_deep, t_deep, molar_mass(tab, &v.gas))
}

/// Bulk composition of a catalog body (FUN_140503b60 for catalog bodies, then FUN_1401aa500).
/// `None` for bodies the catalog calls giants or stars that are too small for SE's giants.
pub fn bulk_composition(b: &BodyIn, t_eq: f32) -> Option<([f32; 6], i32)> {
    let mut r = SeRng::new(b.seeds.0, b.seeds.1);
    // C/O and [Fe/H] of the body, scaled from the star's (FUN_140503b60).
    draw_a(&mut r);
    draw_a(&mut r);
    let c_to_o = body_c_to_o(b);
    // 10% "hot" flag (only read when the mass or radius is missing).
    draw_a(&mut r);
    let (m, rad) = (b.mass_earth, b.radius_km);
    // Giants by size (FUN_1401aa500 with fixed thresholds for catalog bodies). Bodies the catalog
    // calls giants (or stars) below those sizes are left alone.
    let jupiter = m >= 60.0 && rad >= 40000.0;
    if jupiter || (m >= 10.0 && rad >= 18000.0) {
        if matches!(b.bulk, bulk::CHTHONIA | bulk::BROWN_DWARF | bulk::SUN) {
            return None;
        }
        return Some((giant_composition(&mut r, c_to_o, jupiter), b.bulk));
    }
    if matches!(b.bulk, bulk::NEPTUNE | bulk::JUPITER | bulk::CHTHONIA | bulk::BROWN_DWARF | bulk::SUN) {
        return None;
    }
    let t = if t_eq < 600.0 { 0.0 } else { t_eq };
    let dens = sat(((m * EARTH_MASS_KG * 1e-12) / ((rad * rad * rad) as f64 * 4.188_790_329_622_07) as f32) / 6.0);
    let c = rocky_composition(&mut r, c_to_o, dens, t, 0.0, b.bulk);
    let class = if b.bulk == bulk::NONE {
        let (mean, sigma, lo, hi) = if t_eq <= 173.0 && c[2] > 0.5 { (200.0, 10.0, 180.0, 220.0) } else { (300.0, 20.0, 260.0, 340.0) };
        if r.tnormal(mean, sigma, lo, hi) > rad {
            bulk::ASTEROID
        } else {
            bulk_from_composition(&c).unwrap_or(bulk::TERRA)
        }
    } else {
        b.bulk
    };
    Some((c, class))
}

/// Bulk composition and class of a procedural planet (FUN_1401aa500 without the catalog flag):
/// `ty` is SE's generator type (FUN_140504e10: 1 terra, 2 ice giant, 3 gas giant). The giant
/// thresholds are drawn per planet; a terra at least `tn(10)` Earth masses gets a Neptune's
/// make-up and one at least `tn(60)` a Jupiter's. A rocky body reads `met` (the planet's
/// metallicity, record +0x1610) where catalog bodies read their density, its equilibrium
/// temperature (600 K for SE's "cold terra" flag, ProbColdTerra) and its mass.
pub fn procedural_composition(r: &mut SeRng, ty: u8, mass_earth: f32, t_eq: f32, cold: bool, c_to_o: f32, met: f32) -> ([f32; 6], i32) {
    let mass_ice = r.tnormal(10.0, 1.5, 6.0, 20.0);
    let mass_gas = r.tnormal(60.0, 10.0, 30.0, 100.0);
    // The radius thresholds, unread without a radius.
    r.tnormal(18000.0, 1000.0, 15000.0, 40000.0);
    r.tnormal(40000.0, 3000.0, 30000.0, 50000.0);
    let jupiter = ty == 3 || (ty != 2 && mass_earth >= mass_gas);
    if jupiter || ty == 2 || mass_earth >= mass_ice {
        let c = giant_composition(r, c_to_o, jupiter);
        return (c, if jupiter { bulk::JUPITER } else { bulk::NEPTUNE });
    }
    let c = rocky_composition(r, c_to_o, met, if cold { 600.0 } else { t_eq }, mass_earth, bulk::NONE);
    let class = bulk_from_composition(&c).unwrap_or(bulk::TERRA);
    (c, class)
}

/// Radius (km) SE gives a procedural body of `mass_earth` with composition `c` (FUN_1404df8a0).
/// Giants follow SE's fit in Jupiter masses and equilibrium temperature (inflated when hot, /1.8
/// below 60 Earth masses, at most 1.5 Jupiter radii); solid bodies the Seager-style law
/// `R = r1 10^(log10(M/m1)/3 - 0.25 - 0.14565 (M/m1)^0.27)` with r1 interpolated in SE's tables
/// by the ice share, else the carbon share, else the silicate share (FUN_1401abd00).
pub fn procedural_radius_km(c: &[f32; 6], mass_earth: f32, t_eq: f32) -> f32 {
    const EARTH_R: f32 = 6378.14;
    if c[part::GAS] + c[part::GAS2] > 0.0 {
        let mj = mass_earth / 317.8284;
        let f1 = mj.cbrt() * 1.47;
        let x = mj / 3.16;
        let f2 = mj.powf(0.28) * 764.0;
        let f4 = (0.59 / mj).powf(1.03);
        let f3 = (t_eq / f2).powf(f4 * 0.05 + 1.15) * 0.05 + 1.0;
        let mut r = (f3 as f64 * (f1 as f64 / ((x * x).cbrt() as f64 + 1.0))) as f32;
        if mass_earth < 60.0 {
            r /= 1.8;
        }
        return r.min(1.5) * 11.2215 * EARTH_R;
    }
    // (key, r1) rows, keys descending; m1 is 6.41 Earth masses in every row.
    const ICE: [(f32, f32); 6] = [(1.0, 5.742), (0.75, 5.47), (0.5, 5.13), (0.25, 4.822), (0.1, 4.565), (0.0, 4.237)];
    const CARBON: [(f32, f32); 5] = [(1.0, 4.279), (0.68, 4.088), (0.5, 3.811), (0.3, 3.68), (0.0, 3.125)];
    const SILICATE: [(f32, f32); 4] = [(1.0, 4.237), (0.675, 4.048), (0.3, 3.648), (0.0, 3.125)];
    let (tab, v): (&[(f32, f32)], f32) = if c[part::ICE] > 0.001 {
        (&ICE, c[part::ICE])
    } else if c[part::CARBON] > 0.001 {
        (&CARBON, c[part::CARBON])
    } else {
        (&SILICATE, c[part::SILICATE])
    };
    let (mut lo, mut hi) = (0usize, tab.len() - 1);
    while lo + 1 != hi {
        let mid = (lo + hi) / 2;
        if v <= tab[mid].0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let t = (v - tab[hi].0) / (tab[lo].0 - tab[hi].0);
    let r1 = if t < 0.0 {
        tab[hi].1
    } else if t > 1.0 {
        tab[lo].1
    } else {
        (tab[lo].1 - tab[hi].1) * t + tab[hi].1
    };
    let m = mass_earth / 6.41;
    10f32.powf(m.log10() / 3.0 - 0.25 - m.powf(0.27) * 0.145_652_5) * r1 * EARTH_R
}

/// Run SE's completion for a solid body without an `Atmosphere` block.
pub fn complete(tab: &ClimateTables, b: &BodyIn) -> Option<Completion> {
    let bond0 = b.bond_albedo.unwrap_or(0.3);
    let (t_eq0, t_exo) = equilibrium_temps(b.flux, b.xuv, Some(bond0));
    let (comp, mut class) = match b.composition {
        // A given composition stands (FUN_1401aa500 returns early on the body's flag).
        Some(c) => {
            let class = if b.bulk != bulk::NONE {
                b.bulk
            } else {
                bulk_from_composition(&c).unwrap_or(if c[part::GAS] + c[part::GAS2] > 0.0 { bulk::NEPTUNE } else { bulk::TERRA })
            };
            (c, class)
        }
        None => bulk_composition(b, t_eq0)?,
    };
    let mut out = Completion { bulk: class, composition: comp, bond_albedo: bond0, t_eq: t_eq0, t_exo, ..Default::default() };
    let (m, rad) = (b.mass_earth, b.radius_km);
    let mut r = SeRng::new(b.seeds.0, b.seeds.1);

    // SE's system step completes catalog planets (FUN_140503780) and moons (FUN_140502eb0) with
    // its catalog flag set: no asteroid-belt draw for dwarf planets and no cap on tidal heating.
    // Spin axis: a planet draws the equator's node, then the obliquity (degrees, around the
    // orbit's inclination), each when the catalog gives none. A moon first draws the orbit angles
    // its catalog leaves out and takes the orbit's inclination and node as its own.
    let incl_rad = b.orbit_incl_deg.to_radians();
    let node_rad = b.orbit_node_deg.to_radians();
    out.obliquity_rad = if matches!(b.obj_type, obj::MOON | obj::DWARF_MOON) {
        for _ in b.orbit_angles_drawn.iter().filter(|&&d| d) {
            draw_a(&mut r);
        }
        out.eq_node_rad = b.eq_node_deg.map_or(node_rad, f64::to_radians);
        b.obliquity_deg.map_or(incl_rad, f64::to_radians)
    } else {
        out.eq_node_rad = b.eq_node_deg.unwrap_or_else(|| gauss_d(&mut r, 0.0, 5.0) + b.orbit_node_deg).to_radians();
        b.obliquity_deg
            .unwrap_or_else(|| {
                let wide = (draw_a(&mut r) as f32 * INV_2_32) < 0.2;
                gauss_d(&mut r, 0.0, if wide { 180.0 } else { 40.0 }) + b.orbit_incl_deg
            })
            .to_radians()
    };
    // Moment of inertia factor (FUN_14050a5f0) and tidal Q (FUN_14050a520).
    let giant = comp[part::GAS] + comp[part::GAS2] > 0.0;
    if giant {
        out.moment_of_inertia = r.tnormal(0.22, 0.04, 0.18, 0.3);
        out.tidal_q = r.tnormal(200_000.0, 50_000.0, 20_000.0, 4_000_000.0) / r.tnormal(0.4, 0.05, 0.1, 0.6);
    } else {
        let x = (m.log10() + 2.5) / 1.5;
        let i0 = if x < 0.0 { 0.4 } else if x > 1.0 { 0.33 } else { 0.4 - x * 0.069_999_99 };
        out.moment_of_inertia = r.tnormal(i0, 0.002, 0.28, 0.4);
        out.tidal_q = r.tnormal(600.0, 100.0, 200.0, 1000.0);
    }

    // Spin evolution (FUN_14050a340, FUN_140509e60). As the spin slows the axis turns toward the
    // orbit's pole; a locked or resonant body (and one the catalog locks) keeps the orbit's.
    let ecc = b.orbit.map_or(0.0, |o| o.1);
    if b.tidally_locked {
        out.locked = true;
        if b.orbit.is_some() {
            out.obliquity_rad = incl_rad;
            out.eq_node_rad = node_rad;
        }
    } else if b.rotation_days.is_none() {
        let mi = if giant { m.min(317.8284) } else { m };
        let p0 = (out.moment_of_inertia / mi).sqrt() as f64 * ((rad as f64 * 1.11) / 6371.0);
        out.rotation_days = Some(p0);
        if let Some((a_au, e, period, host)) = b.orbit.filter(|o| o.1 < 1.0) {
            let q = (a_au as f32).powi(3) / (host * 3.003_489_4e-6);
            let rr = rad / 6371.0;
            let (w0, worb) = (1.0 / p0, 1.0 / period);
            let despin = (w0 - worb).abs() * (((q * q) as f64 * 1.5 * m as f64) / (rr * rr * rr) as f64 * out.moment_of_inertia as f64 * out.tidal_q as f64) as f32 as f64;
            if (b.age_gyr as f64) < despin {
                let f = b.age_gyr as f64 / despin;
                out.rotation_days = Some(1.0 / (w0 + (worb - w0) * f));
                out.eq_node_rad += (node_rad - out.eq_node_rad) * f;
                out.obliquity_rad += (incl_rad - out.obliquity_rad) * f;
            } else {
                // Spin-orbit resonance odds by eccentricity; 1:1 otherwise.
                let (chance, ratio) = if e > 0.17 {
                    if e > 0.33 {
                        if e > 0.44 {
                            if e > 0.5 {
                                (0.0, 1.0)
                            } else {
                                let t = (e - 0.44) / 0.06;
                                (if t < 0.0 { 0.55 } else if t > 1.0 { 1.0 } else { t * 0.45 + 0.55 }, 1.0 / 3.0)
                            }
                        } else {
                            let t = (e - 0.33) / 0.11;
                            (if t < 0.0 { 0.4 } else if t > 1.0 { 1.0 } else { t * 0.6 + 0.4 }, 0.4)
                        }
                    } else {
                        let t = (e - 0.17) / 0.16;
                        (if t < 0.0 { 0.22 } else if t > 1.0 { 1.0 } else { t * 0.78 + 0.22 }, 0.5)
                    }
                } else {
                    let t = (e - 0.04) / 0.13;
                    (t.clamp(0.0, 1.0), 2.0 / 3.0)
                };
                let u = draw_a(&mut r) as f32 * INV_2_32;
                let ratio = if (chance as f32) < u { 1.0 } else { ratio };
                out.locked = ratio == 1.0;
                out.rotation_days = Some(ratio * period);
                out.obliquity_rad = incl_rad;
                out.eq_node_rad = node_rad;
            }
        }
    }

    // Tidal heating (FUN_1405099c0). For a generated body SE lowers the eccentricity until the
    // heating is at most `(T/32 K)^4 M 5e-12` W, T 500 K for a body above 200 K, else 50 K (its
    // surface temperature then; the equilibrium one here).
    if let Some((a_au, _, _, host)) = b.orbit {
        let mut heat = ecc * ecc * tidal_k(rad, a_au, host as f64, out.tidal_q);
        if b.procedural {
            heat = heat.min(tidal_heat_cap_w(m, out.t_eq));
        }
        out.tidal_heat_w = (heat * small_body_taper(rad)) as f32;
    }

    // Volcanism and internal heat (FUN_140512d50).
    let tidal = out.tidal_heat_w;
    let s1 = (m.sqrt() * 5.49478 - 0.447_797).max(0.0);
    let mut young_t = if m >= 10.0 { m.ln() * 6.27314 + (m * 0.061_728_4 - 5.06173) } else { m };
    young_t *= 0.001;
    let tide = ((tidal.log10() - 13.0) * 0.125).max(0.0);
    let hot = {
        let x = ((out.t_eq - 800.0) / 1200.0).max(0.0);
        (x * x).max(0.0)
    };
    let young = ((1.0 - (b.age_gyr / young_t) as f64) as f32).max(0.0);
    out.lava_coverage = (hot + tide + young).max(0.0);
    let v1 = if s1 > 0.0 { ((s1 / b.age_gyr).log10() + 1.0).max(0.0) } else { 0.0 };
    let v2 = if tidal > 0.0 { ((tidal.log10() - 9.0) * 0.5).max(0.0) } else { 0.0 };
    let v = v2 + v1 + young * 3.0;
    out.volcano_activity = if giant { 0.0 } else { (v * v * 0.5).clamp(0.0, 2.0) };
    let kk = m * 5e-12 * EARTH_MASS_KG;
    let g1 = r.gauss(1.0, 0.1);
    let burst = (10f32.powf(young * 7.0 + 13.0) - 1e13).max(0.0);
    out.heat = (g1 * kk + tidal + burst) / kk;

    // Surface temperature before the atmosphere (FUN_1404d6850).
    let t_surf0 = (((out.t_eq as f64).powf(4.0) + (out.heat * 1_048_576.0) as f64).powf(0.25) as f32).min(3500.0);

    // Atmosphere (FUN_1404494a0; giants FUN_140449860, read at their 1 atm level).
    let mut v = Volatiles::default();
    let (mut p, mut t, mut gh, mut mu) = (0.0f32, t_surf0, UNSET, UNSET);
    let g = m / ((rad / 6371.0) * (rad / 6371.0)) * 9.80665;
    out.gravity = g;
    if giant {
        let (p_deep, t_deep, mu_g) = giant_atmosphere(tab, &mut r, &mut v, m, rad, g, t_surf0, out.t_exo, b.age_gyr, body_c_to_o(b));
        (p, t, mu, gh) = (1.0, t_deep, mu_g, 0.0);
        out.panel_pressure_atm = p_deep;
    } else if rad > 180.0 {
        let pscale = ((m * 1_250_000.0 * m) as f64 / ((rad / 6378.14) as f64).powf(4.0)) as f32;
        let mut vf = 10f32.powf(r.tnormal(-5.0, 2.0, -10.0, -3.0));
        if m > 0.5 {
            vf = ((m + m) * (m + m) * vf).min(0.0005);
        }
        p = vf * pscale;
        v.atm_mass = (vf * m) / 8.7073e-07;
        let water_rich = comp[0] + comp[1] <= 0.0 && comp[2] >= 0.1 && ice_layer_km(&comp, rad, m) > 25.0;
        initial_inventory(tab, &mut r, &mut v, t, body_c_to_o(b), water_rich);
        let before = sum28(&v.gas);
        age_losses(tab, &mut v, &mut p, t, b.age_gyr);
        escape(tab, &mut v, &mut p, out.t_exo, rad, g, b.age_gyr);
        let mut water_rich = water_rich;
        let mut composition = comp;
        if water_rich && t > 200.0 && v.gas[H2O] == 0.0 {
            water_rich = false;
            set_ice(&mut composition, 0.0);
            if let Some(c) = bulk_from_composition(&composition) {
                class = c;
            }
        }
        if p > 1e-10 {
            let after = sum28(&v.gas);
            v.atm_mass *= after / before;
            let k = v.atm_mass / after;
            v.gas.iter_mut().for_each(|x| *x *= k);
            if p > 0.0 {
                v.albedo = bond0;
                let body = SolveBody {
                    radius_km: rad,
                    mass_earth: m,
                    gravity: g,
                    flux: b.flux,
                    heat: out.heat,
                    bulk: class,
                    albedo_given: b.bond_albedo.is_some(),
                    composition,
                };
                out.converged = climate_solve(tab, &mut v, &body, &mut p, &mut t, &mut gh, &mut mu, water_rich);
                // FUN_1404df720: new Bond albedo and equilibrium temperature.
                out.bond_albedo = v.albedo;
                out.t_eq = ((((1.0 - v.albedo) as f64 * 278f64.powf(4.0) * b.flux as f64) + (out.heat * 1_048_576.0) as f64) as f32).powf(0.25).min(3500.0);
            }
        }
        if p < 1e-10 {
            p = 0.0;
            v.gas_sum = 0.0;
            v.liquid_sum = 0.0;
        }
        prune(&mut v);
        out.composition = composition;
    }
    if mu == UNSET {
        mu = molar_mass(tab, &v.gas);
    }
    if gh == UNSET {
        gh = greenhouse(p, mu, t_surf0);
    }
    if p <= 1e-9 {
        p = 0.0;
    }
    out.pressure_atm = p;
    if !giant {
        out.panel_pressure_atm = p;
    }
    out.molar_mass = mu;
    out.greenhouse_k = if p > 0.0 { gh } else { 0.0 };
    out.density_kg_m3 = if p > 0.0 { (p * 101.325 * mu) / (t * R_GAS) } else { 0.0 };

    // FUN_1404cabf0: crushing or steam atmospheres turn the body into a mini-Neptune, a giant
    // without a catalog class becomes a Neptune or (over 25% gas) a Jupiter; small bodies become
    // asteroids.
    let gas = out.composition[part::GAS] + out.composition[part::GAS2];
    let crushing = class != bulk::ASTEROID && (p > 1000.0 || v.flags & 2 != 0);
    if gas > 0.0 || crushing {
        if class == bulk::NONE || crushing {
            class = if gas > 0.25 { bulk::JUPITER } else { bulk::NEPTUNE };
            out.pressure_atm = out.pressure_atm.min(1.0);
            out.greenhouse_k = 0.0;
        }
    } else if rad >= if out.composition[2] > 0.5 { 200.0 } else { 300.0 } {
        if let Some(c) = bulk_from_composition(&out.composition) {
            class = c;
        }
    } else {
        class = bulk::ASTEROID;
    }
    out.bulk = class;

    // Atmosphere heights (FUN_140448ef0).
    if out.pressure_atm > 0.0 && mu > 0.0 {
        out.scale_height_km = (t * R_GAS) / (mu * g);
        let h = |pp: f32, h02: f32| height_at(out.pressure_atm, pp, out.scale_height_km, 0.25, t, g, mu, h02);
        let h02 = h(0.2, 0.0);
        out.atm_height_km = if class >= bulk::NEPTUNE {
            // Giants (classes from Neptune up): 15.6 scale heights at the equilibrium temperature, 100 km to 5% of R.
            ((out.t_eq * R_GAS) / (g * mu) * 15.6).max(100.0).min(rad * 0.05).min(rad * 0.1)
        } else if ATMO_HEIGHT_HARB {
            se_visual_atm_height_km(out.pressure_atm, out.scale_height_km, m, rad)
        } else {
            let mut top = h(0.001, h02).max(0.0);
            if top <= 0.0 {
                top = h(1e-9, h02) * 0.5;
            }
            top.clamp(rad * 0.005, rad * 0.05).min(rad * 0.1)
        };
    }

    // Surface temperature with greenhouse (FUN_1404d6850 again).
    let t_base = (((out.t_eq as f64).powf(4.0) + (out.heat * 1_048_576.0) as f64).powf(0.25) as f32).min(3500.0);
    out.t_surf = if class < bulk::NEPTUNE || class == bulk::NONE { t_base + out.greenhouse_k } else { t_base };

    // Ocean, bump and clouds (FUN_1405057e0). Giants draw no layer count (they get 1 or 2).
    let giant = (bulk::NEPTUNE..=bulk::BROWN_DWARF).contains(&class);
    out.has_clouds = out.pressure_atm > 0.05 && !b.no_clouds;
    let u_clouds = (out.has_clouds && !giant).then(|| draw_a(&mut r));
    let asteroid = class == bulk::ASTEROID;
    let bump = se_bump_default_km(m, rad, asteroid);
    out.bump_km = bump;
    out.has_ocean = !asteroid && !b.no_ocean && v.liquid_sum > 0.0;
    if out.has_ocean {
        out.ocean_depth_km = v.ocean_depth_km;
        if out.ocean_depth_km >= bump && v.liquid[H2O] > 0.0 {
            out.bump_km = draw_a(&mut r) as f32 * 1.5 * INV_2_32 + (out.ocean_depth_km + 0.5);
        }
    }
    if let Some(u) = u_clouds {
        let c = CloudsIn {
            radius_km: rad,
            bump_km: out.bump_km,
            sea_km: if out.has_ocean { out.ocean_depth_km } else { 0.0 },
            atm_height_km: out.atm_height_km,
            pressure_atm: out.pressure_atm,
            t_surf: out.t_surf,
        };
        out.clouds = cloud_layers(&mut r, u, c);
    }
    // The temperature model SE sets up once the system is built (FUN_14025f1c0); its average
    // picks the class word. Locked rotation without a period follows the orbit.
    let thermal = ThermalIn {
        flux: b.flux_mean,
        bond_albedo: out.bond_albedo,
        heat: out.heat,
        greenhouse_k: out.greenhouse_k,
        pressure_atm: out.pressure_atm,
        density_kg_m3: out.density_kg_m3,
        bulk: class,
        steam: v.flags & 2 != 0,
        locked_to_star: out.locked && class < bulk::NEPTUNE && b.star_parent,
        obliquity_rad: out.obliquity_rad,
        rotation_days: out.rotation_days.or(b.rotation_days).unwrap_or_else(|| b.orbit.map_or(1.0, |o| o.2)),
        mass_earth: m,
        radius_km: rad,
    };
    out.t_class = class_temperature(&thermal);
    out.thermal = Some(thermal);
    out.volatiles = v;
    Some(out)
}

/// Tidal heating per unit e^2 (W) of a body of radius `radius_km` on an orbit of `a_au` around a host
/// of `host_mass_earth`, tidal quality `q` (FUN_1405099c0). SE caps a procedural planet's heating
/// where it would warm the planet by 50 K (500 K above 200 K); catalog bodies have no cap.
pub fn tidal_k(radius_km: f32, a_au: f64, host_mass_earth: f64, q: f32) -> f64 {
    (a_au * 149_597_870.7 * 1000.0).powf(-7.5)
        * (host_mass_earth * 5.972_365_131_085_89e24).powf(2.5)
        * 6.67408e-11f64.powf(1.5)
        * (1.05e17 / q as f64)
        * (radius_km as f64).powf(5.0)
}

/// Tidal heating fades out for bodies under 300 km radius (none below 150 km).
pub fn small_body_taper(radius_km: f32) -> f64 {
    if radius_km < 300.0 {
        let s = sat((radius_km - 150.0) / 150.0);
        ((3.0 - (s + s)) * s * s) as f64
    } else {
        1.0
    }
}

/// Inputs of SE's planet temperature model (record +0x184c, set up by FUN_14025f1c0 after the
/// system's bodies are generated). Its average (+0x1860) picks the temperature word of the class;
/// the info panel's day/night and local temperatures come from the same model at display time.
#[derive(Clone, Copy, Debug)]
pub struct ThermalIn {
    /// Insolation (+0x1244), 1 at Earth.
    pub flux: f32,
    pub bond_albedo: f32,
    /// Internal heat (+0x121c), 1 for Earth's; adds `heat * 2^20` to T^4.
    pub heat: f32,
    /// Greenhouse warming (atmosphere +0x6c), 0 without an atmosphere.
    pub greenhouse_k: f32,
    pub pressure_atm: f32,
    /// Surface air density (atmosphere +0x54).
    pub density_kg_m3: f32,
    pub bulk: i32,
    /// Steam or supercritical envelope (composition flag 2, +0x15f8).
    pub steam: bool,
    /// Tidally locked to a star (+0x165c bit 2).
    pub locked_to_star: bool,
    pub obliquity_rad: f64,
    /// Rotation period (+0x164c), days.
    pub rotation_days: f64,
    pub mass_earth: f32,
    pub radius_km: f32,
}

const SIGMA_SB: f32 = 5.670_374e-8;

impl ThermalIn {
    /// Giants, and solid bodies under a crushing or steam envelope.
    fn thick(&self) -> bool {
        (self.bulk != bulk::ASTEROID && (self.pressure_atm > 1000.0 || self.steam)) || self.bulk > 4
    }

    /// Internal heat and greenhouse on top of a radiative T^4 (FUN_14025e390).
    fn surface(&self, t4: f32) -> f32 {
        (t4 + self.heat * 1_048_576.0).powf(0.25) + self.greenhouse_k
    }
}

/// Fold an angle into [-pi/2, pi/2] the way SE does (FUN_14025db00).
fn fold_angle(x: f32) -> f32 {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let d = x as f64;
    (if d > PI {
        d - TAU
    } else if d > FRAC_PI_2 {
        PI - d
    } else if d < -PI {
        d + TAU
    } else if d < -FRAC_PI_2 {
        -PI - d
    } else {
        d
    }) as f32
}

/// Latitude term of the averages: cos of an effective latitude that shrinks with obliquity,
/// blended with the night/day ratio `r`.
fn latitude_term(lat_rad: f64, obliquity_rad: f64, r: f32) -> f32 {
    let u = fold_angle(obliquity_rad as f32).abs();
    let a = lat_rad / ((u + u) as f64 / std::f64::consts::PI + 1.0);
    ((a.abs() as f32).cos() * (1.0 - r) + r).abs()
}

/// (x^-3 + c)^(-1/3), a constant of the vertical averages.
fn cube_mix(x: f32, c: f64) -> f32 {
    ((x.powi(-3) as f64) + c).powf(-0.333_333_343_267_441) as f32
}

/// Average temperature of a body as SE's temperature model gives it (record +0x1860,
/// FUN_14025f1c0 -> FUN_14025df00), which sets the temperature class (> 1000 K Torrid ... <= 90 K
/// Frigid). Starts from the radiative temperature `(F * 1370 (1 - A) / (f sigma))^1/4` with heat
/// spread over a day side (f = 2, locked), the whole sphere (f = 4, 0.2 atm or more) or not at all
/// (f = 1), then averages over day and night and over latitude, and adds internal heat and the
/// greenhouse warming.
pub fn class_temperature(m: &ThermalIn) -> f32 {
    let (p, a, flux) = (m.pressure_atm, m.bond_albedo, m.flux);
    let thick = m.thick();
    let locked = m.locked_to_star;

    // Heat spread factor, sub-stellar temperature and the vertical mixing term (FUN_14025f1c0).
    let s4 = ((1.0 - a) * flux * 1370.0 / (if p >= 0.2 { 4.0 } else { 1.0 } * SIGMA_SB)).powf(0.25);
    let mut mix = 20.0f32;
    if p >= 0.01 {
        mix = (10.0 - 2.0 * m.density_kg_m3.ln()).abs();
        if thick {
            mix = 5.0;
        }
    }
    if locked {
        mix = 66.666_67;
    }
    let h = mix / 100.0;

    // Night to day ratio (FUN_14025e9b0): rotation and air spread the day's heat.
    let floor_t = m.surface(0.0);
    let (t_min, t_max) = if thick {
        (s4 * 0.75, s4)
    } else if p <= 0.001 {
        (floor_t, s4)
    } else {
        let mut dt = ((25.0 - ((p as f64) * 1.01325).log10() * 11.0) * (2.0 / m.rotation_days)) as f32;
        if s4 < dt {
            dt = s4 * 0.99;
        }
        let dt = if dt >= 5.0 { dt } else { 5.0 };
        ((s4 - dt).max(floor_t), s4 + dt)
    };
    let ratio = (t_min.max(20.0) / t_max.max(20.0)).abs() / (1.0 - h);
    let night = if ratio > 1.0 { 0.99 } else { ratio };

    // Radiative temperature for the spread factor (FUN_14025e400; SE passes the luminosity through
    // 3.846e26 W out and 3.826e26 W back).
    let f = if locked { 2.0 } else if p >= 0.2 { 4.0 } else { 1.0 };
    let mut t5 = (flux * 1370.0 * (3.826e26 / 3.846e26) as f32 * (1.0 - a) / (f * SIGMA_SB)).powf(0.25);
    if f == 4.0 {
        t5 /= ((0.994_837_7f32).cos() * (1.0 - night) + night).abs();
    }
    if t5 == 0.0 {
        t5 = 20.0;
    }

    let tc = if t5 <= 20.0 {
        20.0
    } else if locked {
        // FUN_14025cc40: the night side at a third of the day side. Its wind terms cancel.
        let r = (t5 / 3.0) / t5;
        let q = r * 0.1 + r;
        t5 * ((1.0 - q) * 0.25 + q)
    } else if !thick {
        spread_rotating(p, m.obliquity_rad, night, h, t5)
    } else {
        // FUN_14025cd30 with the optical-depth term of FUN_14025df00. Wind (FUN_14025f1c0) only
        // enters through that term; its own terms cancel here too.
        let rho = m.density_kg_m3;
        let mut wind = if p == 0.0 || rho == 0.0 { 0.0 } else { (1.0 / rho).powf(0.25) * 100.0 };
        if wind.is_nan() {
            wind = 0.0;
        }
        while wind > 2000.0 {
            wind /= 10.0;
        }
        let rr = m.radius_km;
        let g = m.mass_earth / ((rr / 6371.0) * (rr / 6371.0)) * 9.806_65;
        let v = ((g + g) * rr * 1000.0).sqrt() * 0.001;
        if v * 1000.0 <= wind {
            wind = ((v * 1000.0) as f64 * 0.75) as f32;
        }
        let tau = (((wind * 0.001).exp() - 1.0) * 1.788_51).max(0.1);
        spread_thick(m.obliquity_rad, wind, night, h, tau_factor(tau) * t5)
    };
    let t = m.surface((tc as f64).powi(4) as f32);
    if t >= 20.0 { t } else { 20.0 }
}

/// Day-side peak of the vertical heat curve for optical depth `tau` (at most 1).
fn tau_peak(tau: f32) -> f32 {
    let x = (tau as f64 / 25.132_741_228_718_345) as f32;
    let t = ((1.0 - tau * tau) as f64 / 631.654_681_669_719 + 1.0 + (x * tau) as f64 / 100.530_964_914_873) - (x * x * 0.125) as f64;
    (t as f32).min(1.0)
}

/// The optical-depth factor (0.95 .. 1) SE applies to a thick atmosphere's temperature.
fn tau_factor(tau: f32) -> f32 {
    ((tau_peak(tau) - 0.372_400_6) * 0.050_000_01) / 0.627_599_36 + 0.95
}

/// Average over a rotating body under thin or no air of radiative temperature `t`
/// (FUN_14025cf40): latitude 52.5 deg with air, 41 deg without, and the mixing `h`.
fn spread_rotating(p: f32, obliquity_rad: f64, night: f32, h: f32, t: f32) -> f32 {
    use std::f64::consts::{PI, TAU};
    let d6 = cube_mix(0.999_998_4, 1184.352_510_482_5);
    let q = (0.694_704_65f32).powi(-3) as f64;
    let w = ((794.016_604_658_579 - q).powf(0.666_666_686_534_882) - (201.840_349_417_331 - q).powf(0.666_666_686_534_882))
        * 0.007_957_747_273_174_44;
    let k = (((w as f32) as f64 + ((PI - PI.powi(3) / 96.0) as f32) as f64) / TAU) as f32;
    let lat = ((if p > 0.0 { 52.5f64 } else { 41.0 } * PI) / 180.0) as f32;
    latitude_term(lat as f64, obliquity_rad, night) * (((k - d6) / (1.0 - d6)) * h + (1.0 - h)) * t
}

/// Average under a giant's or a crushing atmosphere (FUN_14025cd30): latitude 60 deg (pi/3 in
/// f32), the wind term and the mixing `h`.
fn spread_thick(obliquity_rad: f64, wind: f32, night: f32, h: f32, t: f32) -> f32 {
    use std::f64::consts::TAU;
    let d4 = cube_mix(-1.251_902_8, 592.176_255_241_248);
    let d5 = cube_mix(0.999_998_4, 1184.352_510_482_5);
    let xw = wind * 0.000_15;
    let k2 = ((((xw.sin() as f64 - (xw as f64 - TAU).sin()) * ((1.0 - d4) * 0.5) as f64 + d4 as f64 * TAU) as f32) as f64 / TAU) as f32;
    latitude_term(1.047_197_580_337_524_4, obliquity_rad, night) * (((k2 - d5) / (1.0 - d5)) * h + (1.0 - h)) * t
}

/// Look of a generated atmosphere (FUN_140448500): scattering model index into
/// [`ClimateTables::atm_models`], `Bright`, `Opacity`, `SkyLight`, and a height factor. SE leaves
/// hue and saturation untouched in catalog systems.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphereLook {
    pub model: usize,
    pub bright: f32,
    pub opacity: f32,
    pub sky_light: f32,
    pub height_scale: f32,
}

/// Pick the scattering model of a solid body's generated atmosphere (FUN_140448500). SE seeds
/// stream A with the body's seed A + 0x8535f; the fourth value decides. `arid` is volatiles
/// class 1, `life` the body's life flags.
pub fn atmosphere_look(seed_a: u32, c: &Completion, arid: bool, life: bool) -> Option<AtmosphereLook> {
    let p = c.pressure_atm;
    if p <= 1e-9 {
        return None;
    }
    let mut r = SeRng::new(seed_a.wrapping_add(0x8535f), 0);
    for _ in 0..3 {
        draw_a(&mut r);
    }
    let u1 = draw_a(&mut r) as f32 * INV_2_32;
    // Giants: Earth's scattering, a quarter of them Jupiter's; brightness 3, the haze thinning
    // from 500 to 1500 K.
    if c.bulk > bulk::AQUARIA {
        let t = c.t_eq;
        let opacity = if t < 500.0 { 1.0 } else if t <= 1500.0 { 1.0 - ((t - 500.0) * 0.8) / 1000.0 } else { 0.2 };
        return Some(AtmosphereLook { model: if u1 >= 0.75 { 7 } else { 2 }, bright: 3.0, opacity, sky_light: 1.0, height_scale: 1.0 });
    }
    let v = &c.volatiles;
    let gs = v.gas_sum;
    let pct = |s: usize| if gs != 0.0 { v.gas[s] * 100.0 / gs } else { v.gas[s] * 100.0 };
    // Hydrocarbon seas (FUN_1401ada10): the main liquid is one of CH4 .. C8H18.
    let hydrocarbon_seas = v.liquid_sum > 0.0 && {
        let mut best = (0usize, v.liquid[0].max(0.0));
        for s in 1..27 {
            if best.1 < v.liquid[s] {
                best = (s, v.liquid[s]);
            }
        }
        v.liquid[27] <= best.1 && (15..21).contains(&best.0)
    };
    // 10 Ethereal on warm or half the airless-ish bodies, else 6 Pluto.
    let tenuous = if c.t_eq > 250.0 || u1 >= 0.5 { 10 } else { 6 };
    let model = if life && p > 0.001 && u1 < 0.25 {
        12
    } else if pct(13) > 0.1 && p > 0.001 {
        if u1 >= 0.8 { 9 } else { 8 }
    } else if pct(CH4) > 0.2 && p > 0.001 {
        if !arid || p >= 2.0 || u1 >= 0.5 { 5 } else { 4 }
    } else if arid {
        if p <= 0.001 {
            tenuous
        } else if p <= 0.05 {
            if u1 >= 0.8 {
                7
            } else if u1 >= 0.4 {
                3
            } else {
                11
            }
        } else if p <= 1.5 {
            if u1 >= 0.9 {
                7
            } else if u1 >= 0.8 {
                3
            } else {
                2
            }
        } else if p <= 20.0 {
            if u1 >= 0.8 {
                7
            } else if u1 >= 0.2 {
                3
            } else {
                9
            }
        } else if u1 < 0.25 {
            1
        } else if u1 < 0.5 {
            3
        } else if u1 < 0.75 {
            7
        } else {
            9
        }
    } else {
        let base = if hydrocarbon_seas { 4 } else { 2 };
        if p <= 0.001 {
            tenuous
        } else if p <= 0.05 {
            if u1 >= 0.8 { 7 } else { 11 }
        } else if p <= 15.0 {
            if u1 >= 0.9 { 7 } else { base }
        } else if p <= 50.0 {
            if u1 >= 0.8 { 7 } else { 9 }
        } else if u1 < 0.333 {
            1
        } else if u1 < 0.666 {
            7
        } else {
            9
        }
    };
    let bright = ((p * 10.0).powf(0.2) * 10.0).clamp(0.5, 10.0);
    Some(AtmosphereLook {
        model,
        bright,
        opacity: if p <= 0.001 { 0.0 } else { 1.0 },
        sky_light: bright / 3.0,
        height_scale: if model == 3 || model == 6 { 1.5 } else { 1.0 },
    })
}

/// Distances (AU) at which SE reads a body's starlight (FUN_14050abe0). Every level of the body's
/// hierarchy sits at periapsis (FUN_1404ea7d0), so the distance to the star is the periapsis of the
/// orbit around it (of a planet, or of a planet's barycentre); a mode then scales it with the
/// eccentricity of the body's own orbit (of its planet's, for a moon): x1 (mode 0, read by the
/// completion) and x(1 + e^2/2) / (1 - e) (mode 2, read by the temperature model). For a planet
/// that is periapsis and the time-averaged distance a (1 + e^2/2). Also returns the object whose
/// position stands for the star.
/// Star record `s` is a barycentre of planets (a binary planet's, from SE's moon generator), not of
/// stars.
pub fn planet_barycentre(cat: &crate::catalog::Catalog, s: u32) -> bool {
    use crate::catalog::ObjectRef;
    let st = &cat.stars[s as usize];
    st.barycenter && !st.names.iter().any(|n| cat.children_of(n).iter().any(|c| matches!(c, ObjectRef::Star(_))))
}

/// Body `i` as one of a binary planet's pair: its partner's mass (Earth masses) and the pair's
/// separation (AU), from its own distance to their barycentre. SE heats and despins each with the
/// other as host (FUN_1405099c0); the barycentre's total mass at the body's distance from it would
/// overheat it by `(M_total / M_partner)^10`.
pub fn binary_partner(cat: &crate::catalog::Catalog, i: usize) -> Option<(f64, f64)> {
    use crate::catalog::ObjectRef;
    let b = &cat.bodies[i];
    let Some(ObjectRef::Star(s)) = cat.find(&b.parent) else { return None };
    if !planet_barycentre(cat, s) {
        return None;
    }
    let mut pair: Vec<usize> = cat
        .children_of(&b.parent)
        .iter()
        .filter_map(|c| match c {
            ObjectRef::Body(j) => Some(*j as usize),
            _ => None,
        })
        .collect();
    pair.sort_by(|&x, &y| cat.bodies[y].mass_kg.total_cmp(&cat.bodies[x].mass_kg));
    pair.truncate(2);
    let other = match pair[..] {
        [p, q] if p == i => q,
        [p, q] if q == i => p,
        _ => return None,
    };
    let (m, mo) = (b.mass_kg, cat.bodies[other].mass_kg);
    let a_au = b.orbit.as_ref()?.semi_major_axis / crate::units::METERS_PER_AU;
    (mo > 0.0).then(|| (mo / crate::units::EARTH_MASS_KG, a_au * (m + mo) / mo))
}

pub fn reference_distances_au(cat: &crate::catalog::Catalog, i: usize) -> Option<(f64, f64, crate::catalog::ObjectRef)> {
    use crate::catalog::{BodyTag, ObjectRef};
    let b = &cat.bodies[i];
    let own = b.orbit.as_ref()?;
    let e = match (b.tag, cat.find(&b.parent)) {
        (BodyTag::Moon | BodyTag::DwarfMoon, Some(ObjectRef::Body(p))) => cat.bodies[p as usize].orbit.as_ref().map_or(own.eccentricity, |o| o.eccentricity),
        _ => own.eccentricity,
    };
    // Climb through planets and planetary barycentres to the orbit around the star(s).
    let stellar = |s: u32| {
        let st = &cat.stars[s as usize];
        !st.barycenter
            || st.names.iter().any(|n| cat.children_of(n).iter().any(|c| matches!(c, ObjectRef::Star(k) if !cat.stars[*k as usize].barycenter)))
    };
    let mut carrier = (b.orbit.as_ref()?, b.parent.as_str());
    loop {
        match cat.find(carrier.1)? {
            ObjectRef::Body(p) => {
                let pb = &cat.bodies[p as usize];
                carrier = (pb.orbit.as_ref()?, pb.parent.as_str());
            }
            ObjectRef::Star(s) if !stellar(s) => {
                let st = &cat.stars[s as usize];
                carrier = (st.orbit.as_ref()?, st.parent.as_deref()?);
            }
            host @ ObjectRef::Star(_) => {
                let q = carrier.0.semi_major_axis * (1.0 - carrier.0.eccentricity) / crate::units::METERS_PER_AU;
                if !(q > 0.0) || e >= 1.0 {
                    return None;
                }
                return Some((q, q * (1.0 + 0.5 * e * e) / (1.0 - e), host));
            }
            _ => return None,
        }
    }
}

/// Light a body gets from its system's stars, as SE reads it.
#[derive(Clone, Copy, Debug, Default)]
pub struct StarLight {
    /// Sum of L_bol / d^2 (L_sun, AU) and of xuv_fraction * L_bol / d^2 at periapsis (completion).
    pub flux: f64,
    pub xuv: f64,
    /// L_bol / d^2 at the mean distance (temperature model, [`class_temperature`]).
    pub flux_mean: f64,
    pub age_gyr: f64,
    /// Mass of the body's parent in Earth masses (0 if unknown).
    pub parent_mass_earth: f64,
    /// `CtoO` of the body's host star, if the catalog gives one.
    pub star_c_to_o: Option<f64>,
}

/// System age (Gyr) as SE sets it (FUN_1404f62e0): the largest catalog `Age` among the system's
/// stars, else a uniform fraction of the shortest stellar lifetime (see [`system_age_gyr`]), drawn
/// from the root record's position seed.
pub fn catalog_system_age(stars: &[&crate::catalog::StarRecord], root: &crate::sc::Block) -> f64 {
    let given = stars.iter().filter_map(|s| s.block.f64("Age")).fold(f64::NEG_INFINITY, f64::max);
    if given.is_finite() {
        return given;
    }
    let lifetimes = stars.iter().map(|s| star_lifetime_gyr((s.mass_kg / crate::units::SOLAR_MASS_KG) as f32, star_class_bits(&s.class).0));
    system_age_gyr(crate::system_gen::age_draw(root), lifetimes) as f64
}

/// Light a body gets from its system's stars at `jd` as SE reads it: periapsis flux for the
/// completion, mean-distance flux for the temperature model, the system age and the parent's mass.
pub fn star_light(eph: &crate::ephemeris::Ephemeris, cat: &crate::catalog::Catalog, obj: crate::catalog::ObjectRef, jd: f64) -> StarLight {
    let stars = cat.system_root(obj).map(|root| system_stars(cat, root)).unwrap_or_default();
    star_light_from(eph, cat, obj, jd, &stars)
}

/// The stars (not barycentres) of the system under `root`, the root included.
pub fn system_stars(cat: &crate::catalog::Catalog, root: u32) -> Vec<u32> {
    system_stars_in(cat, &system_walk(cat, root))
}

/// The members of the system under `root` with the root pushed last (not deduplicated): the walk
/// [`system_stars`] and [`display::catalog_view`] filter, worth keeping while the catalog stands.
pub fn system_walk(cat: &crate::catalog::Catalog, root: u32) -> Vec<crate::catalog::ObjectRef> {
    let mut members = cat.system_members(&cat.stars[root as usize].names[0]);
    members.push(crate::catalog::ObjectRef::Star(root));
    members
}

/// [`system_stars`] from a [`system_walk`].
pub fn system_stars_in(cat: &crate::catalog::Catalog, walk: &[crate::catalog::ObjectRef]) -> Vec<u32> {
    use crate::catalog::ObjectRef;
    walk.iter().filter_map(|&s| match s {
        ObjectRef::Star(i) if !cat.stars[i as usize].barycenter => Some(i),
        _ => None,
    }).collect()
}

/// [`star_light`] with the system's stars from [`system_stars`], for many bodies of one system.
pub fn star_light_from(eph: &crate::ephemeris::Ephemeris, cat: &crate::catalog::Catalog, obj: crate::catalog::ObjectRef, jd: f64, stars: &[u32]) -> StarLight {
    use crate::catalog::ObjectRef;
    let pos = eph.position(obj, jd);
    let mut light = StarLight::default();
    if let Some(root) = cat.system_root(obj) {
        let mut lit = Vec::new();
        for &i in stars {
            let st = &cat.stars[i as usize];
            // A rogue planet's system centre gives no light.
            if st.class.kind == crate::star::StarKind::Planemo {
                continue;
            }
            let lum = st.lum_bol();
            let d_au = eph.position(ObjectRef::Star(i), jd).delta_m(pos).length() / crate::units::METERS_PER_AU;
            let (lc, ty, sub) = star_class_bits(&st.class);
            light.flux += lum / (d_au * d_au).max(1e-6);
            if let Some(t) = cat.climate.as_ref() {
                light.xuv += t.xuv_fraction(lc, ty, sub) as f64 * lum / (d_au * d_au).max(1e-6);
            }
            lit.push(st);
        }
        light.age_gyr = catalog_system_age(&lit, &cat.stars[root as usize].block);
        // SE reads the light at reference points of the orbit (reference_distances_au).
        let (mut peri, mut mean) = (1.0, 1.0);
        if let ObjectRef::Body(j) = obj {
            if let Some((q, m, host)) = reference_distances_au(cat, j as usize) {
                let r = eph.position(host, jd).delta_m(pos).length() / crate::units::METERS_PER_AU;
                if r > 0.0 {
                    peri = (r / q).powi(2);
                    mean = (r / m).powi(2);
                }
            }
        }
        light.flux_mean = light.flux * mean;
        light.flux *= peri;
        light.xuv *= peri;
    }
    if let ObjectRef::Body(i) = obj {
        let b = &cat.bodies[i as usize];
        light.parent_mass_earth = match cat.find(&b.parent) {
            Some(ObjectRef::Star(s)) => {
                light.star_c_to_o = cat.stars[s as usize].block.f64("CtoO");
                cat.stars[s as usize].mass_kg
            }
            Some(ObjectRef::Body(j)) => cat.bodies[j as usize].mass_kg,
            _ => 0.0,
        } / crate::units::EARTH_MASS_KG;
    }
    light
}

/// Equilibrium temperature of a body (K) from the starlight `flux` (L_sun / AU^2), its albedo
/// and any catalog greenhouse warming: SE's max(20, 278 (F (1 - A))^1/4) ([`equilibrium_temps`])
/// with the Bond albedo as SE's loader sets it (`AlbedoBond`, else `Albedo`, else `AlbedoGeom`).
pub fn equilibrium_temperature(flux: f64, b: &crate::catalog::BodyRecord) -> f64 {
    let blk = &b.block;
    let bond = blk.f64("AlbedoBond").or_else(|| blk.f64("Albedo")).or_else(|| blk.f64("AlbedoGeom")).unwrap_or(b.albedo);
    let greenhouse = blk.sub("Atmosphere").and_then(|a| a.f64("Greenhouse")).unwrap_or(0.0);
    (278.0 * (flux * (1.0 - bond)).max(0.0).powf(0.25)).max(20.0) + greenhouse
}

/// SE's completion input for catalog body `i`, or `None` when SE keeps the catalog's own
/// atmosphere or the body is not a planet, dwarf planet or moon (moons go through
/// FUN_140502eb0, which differs only in the spin axis).
pub fn catalog_body_in(cat: &crate::catalog::Catalog, i: usize, light: &StarLight) -> Option<BodyIn> {
    use crate::catalog::BodyTag;
    let b = &cat.bodies[i];
    let blk = &b.block;
    let obj_type = match b.tag {
        BodyTag::Planet => obj::PLANET,
        BodyTag::DwarfPlanet => obj::DWARF_PLANET,
        BodyTag::Moon => obj::MOON,
        BodyTag::DwarfMoon => obj::DWARF_MOON,
        _ => return None,
    };
    let moon = matches!(obj_type, obj::MOON | obj::DWARF_MOON);
    let angles = crate::system_gen::given_angles(blk).unwrap_or((None, None));
    if blk.sub("Atmosphere").is_some() || blk.bool("NoAtmosphere").unwrap_or(false) || blk.str("Model").is_some() {
        return None;
    }
    if light.flux <= 0.0 || b.mass_kg <= 0.0 || b.radius_km <= 0.0 {
        return None;
    }
    let class = b.class.as_deref().unwrap_or("");
    let bulk = if class.is_empty() { bulk::NONE } else { crate::surface_gen::bulk_of(class) };
    let orbit = b.orbit.as_ref().filter(|_| light.parent_mass_earth > 0.0).map(|o| {
        let a_au = o.semi_major_axis / crate::units::METERS_PER_AU;
        let period_s = if o.period > 0.0 {
            o.period
        } else {
            std::f64::consts::TAU * (o.semi_major_axis.powi(3) / (crate::units::G * light.parent_mass_earth * crate::units::EARTH_MASS_KG)).sqrt()
        };
        // One of a binary planet's pair takes its partner, at their separation, as its host.
        match binary_partner(cat, i) {
            Some((partner_me, sep_au)) => (sep_au, o.eccentricity, period_s / 86_400.0, partner_me as f32),
            None => (a_au, o.eccentricity, period_s / 86_400.0, light.parent_mass_earth as f32),
        }
    });
    let rot = blk.sub("Rotation");
    let given = |key: &str| blk.f64(key).is_some() || rot.is_some_and(|r| r.f64(key).is_some());
    Some(BodyIn {
        seeds: crate::surface_gen::body_seeds(b),
        obj_type,
        bulk,
        mass_earth: (b.mass_kg / crate::units::EARTH_MASS_KG) as f32,
        radius_km: b.radius_km as f32,
        // SE's loader: `Albedo` sets both albedos, `AlbedoBond`/`AlbedoGeom` one each, and a
        // missing Bond albedo takes the geometric one (FUN_140468860).
        bond_albedo: blk.f64("AlbedoBond").or_else(|| blk.f64("Albedo")).or_else(|| blk.f64("AlbedoGeom")).map(|v| v as f32),
        age_gyr: light.age_gyr as f32,
        star_c_to_o: light.star_c_to_o.map(|v| v as f32),
        flux: light.flux as f32,
        xuv: light.xuv as f32,
        flux_mean: light.flux_mean as f32,
        orbit,
        orbit_incl_deg: b.orbit.as_ref().map_or(0.0, |o| o.inclination.to_degrees()),
        orbit_node_deg: b.orbit.as_ref().map_or(0.0, |o| o.ascending_node.to_degrees()),
        orbit_angles_drawn: [moon && angles.0.is_none(), moon && angles.1.is_none()],
        star_parent: matches!(cat.find(&b.parent), Some(crate::catalog::ObjectRef::Star(s)) if !planet_barycentre(cat, s)),
        obliquity_deg: blk.f64("Obliquity").or_else(|| rot.and_then(|r| r.f64("Obliquity"))),
        eq_node_deg: ["EqAscendNode", "EquatorAscNode"].iter().find_map(|k| blk.f64(k).or_else(|| rot.and_then(|r| r.f64(k)))),
        rotation_days: if given("RotationPeriod") { b.rotation.period_seconds().map(|s| s / 86_400.0) } else { None },
        tidally_locked: blk.bool("TidalLocked").unwrap_or(false) || rot.is_some_and(|r| r.bool("TidalLocked").unwrap_or(false)),
        no_ocean: blk.bool("NoOcean").unwrap_or(false),
        no_clouds: blk.bool("NoClouds").unwrap_or(false),
        composition: interior_composition(blk),
        procedural: b.source == PROCEDURAL_SOURCE,
    })
}

/// The catalog source of bodies loaded from SE's system generator (`procgen`); SE caps their tidal
/// heating ([`tidal_heat_cap_w`]).
pub const PROCEDURAL_SOURCE: &str = "procgen";

/// SE's cap on a generated body's tidal heating (FUN_1405099c0): `(T/32 K)^4 M 5e-12` W with T
/// 500 K for a body warmer than 200 K, else 50 K.
pub fn tidal_heat_cap_w(mass_earth: f32, t_k: f32) -> f64 {
    let t_cap: f64 = if t_k > 200.0 { 500.0 } else { 50.0 };
    (t_cap * 0.031_25).powi(4) * mass_earth as f64 * 5.972_365_131_085_89e24 * 5e-12
}

/// SE's names for the six composition parts, in its order.
pub const COMPOSITION_KEYS: [&str; 6] = ["Hydrogen", "Helium", "Ices", "Silicates", "Carbides", "Metals"];

/// A body's `Interior { Composition { ... } }` (percent) as normalised fractions.
fn interior_composition(blk: &crate::sc::Block) -> Option<[f32; 6]> {
    let c = blk.sub("Interior")?.sub("Composition")?;
    let mut v = [0.0f32; 6];
    for (k, x) in COMPOSITION_KEYS.iter().zip(v.iter_mut()) {
        *x = c.f64(k).unwrap_or(0.0).max(0.0) as f32;
    }
    let sum: f32 = v.iter().sum();
    if sum <= 0.0 {
        return None;
    }
    v.iter_mut().for_each(|x| *x /= sum);
    Some(v)
}

pub mod display;

#[cfg(test)]
mod tests;

