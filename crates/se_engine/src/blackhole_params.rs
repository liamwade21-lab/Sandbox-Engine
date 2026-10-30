//! SpaceEngine's accretion-disk and jet parameters of a black hole, neutron star or white dwarf:
//! the catalog `AccretionDisk` keys (FUN_14046c410) and the defaults SE fills in for the keys a
//! catalog leaves out (FUN_140507690, run from the star set-up FUN_1405045a0 for every class-11
//! star but type 14). SE's class word (body +0x1620) of these is class 11 (0x5800) with type
//! < 12 a white dwarf, 12 a neutron star, 13 a black hole and 14 a wormhole.
//!
//! SE stores lengths in km in the body (`InnerRadius` and the other AU keys are multiplied by
//! 149597870.7). `R` below is the star's radius (a hole's is its Schwarzschild radius, FUN_1405045a0)
//! and `Rg` the Schwarzschild radius, 2.9532 km per solar mass (body +0x1b04). SE's own random
//! stream is seeded from the star record; here it is seeded from the name, so random defaults
//! (disk temperature, inner thickness, spin, whether a disk exists at all) differ from SE's draw
//! but follow its distributions.

use se_core::sc::Block;

const AU_KM: f64 = 149_597_870.7;
/// Schwarzschild radius per solar mass, km (SE: 8.87005e-6 km per Earth mass).
pub const RG_PER_MSUN_KM: f64 = 2.953_25;
/// SE's "unset" sentinel is -1e38; here `None`.
type Opt = Option<f64>;

/// The class-11 stars SE draws with its GR shader (einstein.glsl).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remnant {
    /// Type < 12: STAR define, metric scaled by Rg / R (`local_9e3` in FUN_140270900, `bVar4` in
    /// FUN_140507690).
    WhiteDwarf,
    /// Type 12: STAR define.
    NeutronStar,
    /// Type 13 (`local_9e4`, `bVar3`).
    BlackHole,
}

impl Remnant {
    pub fn of(kind: se_core::star::StarKind) -> Option<Self> {
        use se_core::star::StarKind;
        match kind {
            StarKind::WhiteDwarf => Some(Remnant::WhiteDwarf),
            StarKind::NeutronStar => Some(Remnant::NeutronStar),
            StarKind::BlackHole => Some(Remnant::BlackHole),
            _ => None,
        }
    }
}

/// Resolved disk + jet parameters, SE units (km, K, g/cm^3, catalog luminosities).
#[derive(Clone, Debug, PartialEq)]
pub struct DiskParams {
    pub inner_km: f64,
    pub outer_km: f64,
    pub inner_thick_km: f64,
    pub outer_thick_km: f64,
    pub thickness_pow: f64,
    pub detail_r: f64,
    pub detail_v: f64,
    pub twist: f64,
    pub octave_dist: [f64; 3],
    pub octave_scale: f64,
    pub noise_contrast: f64,
    pub temp_contrast: f64,
    pub outer_spin: f64,
    /// `Luminosity` (visual, solar units) and `LuminosityBol`.
    pub luminosity: f64,
    pub luminosity_bol: f64,
    pub temperature: f64,
    /// `AccretionRate`, solar masses per year.
    pub accretion_rate: f64,
    /// `Density`, g/cm^3.
    pub density: f64,
    pub brightness: f64,
    pub lighting_bright: f64,
    pub shadow_contrast: f64,
    pub shadow_length: f64,
    pub jet_velocity: f64,
    pub jet_length_km: f64,
    pub jet_start_radius_km: f64,
    pub jet_end_radius_km: f64,
    pub jet_start_temp: f64,
    pub jet_end_temp: f64,
    pub jet_thickness: f64,
    pub jet_distortion: f64,
    pub jet_density: f64,
    pub jet_brightness: f64,
}

/// The catalog keys of an `AccretionDisk` block as SE reads them (FUN_14046c410): later keys
/// override earlier ones, AU keys are converted to km.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskKeys {
    pub inner_km: Opt,
    pub outer_km: Opt,
    pub inner_thick_km: Opt,
    pub outer_thick_km: Opt,
    pub thickness_pow: Opt,
    pub jet_length_km: Opt,
    pub jet_start_radius_km: Opt,
    pub jet_end_radius_km: Opt,
    pub accretion_rate: Opt,
    pub temperature: Opt,
    pub density: Opt,
    pub luminosity: Opt,
    pub luminosity_bol: Opt,
    pub outer_spin: Opt,
    pub detail_r: Opt,
    pub detail_v: Opt,
    pub octave_dist: [Opt; 3],
    pub octave_scale: Opt,
    pub noise_contrast: Opt,
    pub temp_contrast: Opt,
    pub twist: Opt,
    pub brightness: Opt,
    pub jet_velocity: Opt,
    pub jet_start_temp: Opt,
    pub jet_end_temp: Opt,
    pub jet_distortion: Opt,
    pub jet_thickness: Opt,
    pub jet_density: Opt,
    pub jet_brightness: Opt,
    pub lighting_bright: Opt,
    pub shadow_contrast: Opt,
    pub shadow_length: Opt,
}

impl DiskKeys {
    /// FUN_14046c410, in its order (a later key overwrites the same field).
    pub fn parse(b: &Block) -> Self {
        let f = |k: &str| b.f64(k);
        let au = |k: &str| b.f64(k).map(|v| v * AU_KM);
        let pick = |v: &[Opt]| v.iter().rev().find_map(|x| *x);
        let thick_both_km = f("ThicknessKm");
        let thick_both_au = au("Thickness");
        DiskKeys {
            inner_km: pick(&[f("InnerRadiusKm"), au("InnerRadius")]),
            outer_km: pick(&[f("OuterRadiusKm"), f("RadiusKm"), au("OuterRadius"), au("Radius")]),
            inner_thick_km: pick(&[f("InnerThicknessKm"), thick_both_km, au("InnerThickness"), thick_both_au]),
            outer_thick_km: pick(&[f("OuterThicknessKm"), thick_both_km, au("OuterThickness"), thick_both_au]),
            thickness_pow: f("ThicknessPow"),
            jet_length_km: pick(&[f("JetLengthKm"), au("JetLength")]),
            jet_start_radius_km: pick(&[f("JetStartRadiusKm"), au("JetStartRadius")]),
            jet_end_radius_km: pick(&[f("JetEndRadiusKm"), au("JetEndRadius")]),
            // kg/s -> solar masses per year (_DAT_1406d3f20).
            accretion_rate: pick(&[f("AccretionRate"), f("AccretionRateKgPerSec").map(|v| v * 1.586_995_247_12e-23)]),
            temperature: f("Temperature"),
            density: f("Density"),
            luminosity: pick(&[f("Luminosity"), f("Lum")]),
            luminosity_bol: pick(&[f("LuminosityBol"), f("LumBol")]),
            outer_spin: f("DiskOuterSpin"),
            detail_r: f("DetailScaleR"),
            detail_v: f("DetailScaleV"),
            octave_dist: [f("OctaveDistortionX"), f("OctaveDistortionY"), f("OctaveDistortionZ")],
            octave_scale: f("OctaveScale"),
            noise_contrast: f("DiskNoiseContrast"),
            temp_contrast: f("DiskTempContrast"),
            twist: f("TwistMagn"),
            brightness: pick(&[f("Exposure"), f("Brightness")]),
            jet_velocity: f("JetVelocity"),
            jet_start_temp: f("JetStartTemp"),
            jet_end_temp: f("JetEndTemp"),
            jet_distortion: f("JetDistortion"),
            jet_thickness: f("JetThickness"),
            jet_density: f("JetDensity"),
            jet_brightness: f("JetBrightness"),
            lighting_bright: f("LightingBright"),
            shadow_contrast: f("ShadowContrast"),
            shadow_length: f("ShadowLength"),
        }
    }
}

/// SE's per-thread LCG (`x * 0x19660d + 0x3c6ef35f`), two streams like SE's TLS slots 0x24
/// (uniforms) and 0x20 (FUN_140443d50's Gaussian).
pub struct SeRng {
    a: u32,
    b: u32,
}

impl SeRng {
    pub fn from_name(name: &str) -> Self {
        let h = se_core::proc_surface::name_seed(name);
        SeRng { a: h as u32, b: (h >> 32) as u32 }
    }
    fn step(x: u32) -> u32 {
        x.wrapping_mul(0x19660d).wrapping_add(0x3c6e_f35f)
    }
    /// Uniform in [0, 1) (`uVar * 2^-32`).
    pub fn uniform(&mut self) -> f64 {
        self.a = Self::step(self.a);
        self.a as f64 * 2.328_306_44e-10
    }
    /// FUN_140443d50: mean + sigma * (sum of 12 uniforms - 6).
    pub fn gauss(&mut self, mean: f64, sigma: f64) -> f64 {
        let mut s = 0.0;
        for _ in 0..12 {
            self.b = Self::step(self.b);
            s += self.b as f64;
        }
        (s * 2.328_306_44e-10 - 6.0) * sigma + mean
    }
}

/// rnd_tn(mean, sigma, 0, 0.9999): a normal draw, redrawn until inside the range.
fn truncated_gauss(rng: &mut SeRng, mean: f64, sigma: f64) -> f64 {
    for _ in 0..32 {
        let s = rng.gauss(mean, sigma);
        if (0.0..=0.9999).contains(&s) {
            return s;
        }
    }
    mean
}

/// SE's default `KerrSpin` (FUN_1405045a0, body +0x122c): a black hole's is a normal draw around
/// 0.5 (sigma 0.15, cut to 0..0.9999) up to 1000 solar masses, `0.5 sqrt(u) + 0.4999` above; a
/// neutron star's a normal draw around 0.2 (sigma 0.05); a white dwarf's `(0.2 u)^2`.
pub fn default_spin(kind: Remnant, mass_sun: f64, rng: &mut SeRng) -> f64 {
    match kind {
        Remnant::BlackHole if mass_sun <= 1000.0 => truncated_gauss(rng, 0.5, 0.15),
        Remnant::BlackHole => rng.uniform().sqrt() * 0.5 + 0.4999,
        Remnant::NeutronStar => truncated_gauss(rng, 0.2, 0.05),
        Remnant::WhiteDwarf => (rng.uniform() * 0.2).powi(2),
    }
}

/// The accretion disk's temperature from its accretion rate (FUN_1404dfdc0, the inverse of the
/// default rate): `14000 ((1 - sqrt(3/x)) rate 6.30122e9)^0.3 (Rg / 2.95325)^0.25
/// (Rg x 1e-5)^-0.75`, `x` the inner edge in Rg (R / Rg for a white dwarf).
fn rate_temperature(rate: f64, rg_km: f64, x: f64) -> f64 {
    let f = (1.0 - (3.0 / x).sqrt()).max(0.0);
    f.powf(0.3) * (rate * 6.301_215_74e9).powf(0.3) * 14_000.0 * (rg_km / RG_PER_MSUN_KM).powf(0.25) * (rg_km * x * 1.0e-5).powf(-0.75)
}

/// FUN_140507690 for a white dwarf (`bVar4`), neutron star or black hole (`bVar3`), without SE's
/// `param_4` (an accretion parameter of procedural systems, unset for catalog stars). `keys` is
/// the parsed `AccretionDisk` block (`None` when the star has none); `radius_km` the star's
/// radius (body +0x1110; a hole's Schwarzschild radius), `rg_km` its Schwarzschild radius
/// (+0x1b04). Returns `None` when the star ends up without a disk.
pub fn resolve(keys: Option<&DiskKeys>, no_disk: bool, kind: Remnant, mass_sun: f64, radius_km: f64, rg_km: f64, rng: &mut SeRng) -> Option<DiskParams> {
    if no_disk {
        return None;
    }
    let (r, rg) = (radius_km, rg_km);
    let wd = kind == Remnant::WhiteDwarf;
    // The inner edge in Rg of SE's alpha-disk formulas: 4.32 (maximum temperature) and 3.94453
    // (maximum density), R / Rg for a white dwarf.
    let (x_temp, x_dens) = if wd { (r / rg, r / rg) } else { (4.32_f64, 3.944_53_f64) };
    // bVar2: supermassive (> 400 solar masses) holes always get a disk; otherwise a star without
    // the tag gets one with probability 0.2 (DAT_1406d4204).
    let massive = mass_sun >= 400.0;
    let empty = DiskKeys::default();
    let k = match keys {
        Some(k) => k,
        None => {
            if !massive && rng.uniform() >= 0.2 {
                return None;
            }
            &empty
        }
    };
    // Disk temperature: a Gaussian draw (FUN_140443d50): N(6000, 700) K for a stellar hole,
    // N(30000, 3000) for a neutron star or white dwarf.
    let mut t = k.temperature.unwrap_or_else(|| {
        if massive {
            if mass_sun < 1.0e5 { rng.gauss(4000.0, 600.0) } else { rng.gauss(10000.0, 1500.0) }
        } else if kind == Remnant::BlackHole {
            rng.gauss(6000.0, 700.0)
        } else {
            rng.gauss(30000.0, 3000.0)
        }
    });
    let mass_cap = mass_sun * 3.0e-8;
    let accretion_rate = match k.accretion_rate {
        Some(a) => a,
        None => {
            let a = (t / 14000.0).powf(3.333_333_25);
            let b = (rg / RG_PER_MSUN_KM).powf(-0.833_333_313);
            let c = (rg * x_temp * 1.0e-5).powf(2.5);
            let rate = (a * b * c / (1.0 - (3.0 / x_temp).sqrt()) / 6.301_215_74e9).min(mass_cap);
            // A white dwarf's disk temperature follows from the (capped) rate.
            if wd {
                t = rate_temperature(rate, rg, x_temp);
            }
            rate
        }
    };
    let inner_km = k.inner_km.unwrap_or(r * 1.01);
    let outer_km = match k.outer_km {
        Some(o) => o,
        None => {
            // The default outer edge exists only for disks hotter than 2000 K and wider than
            // twice the inner radius; otherwise SE drops the disk (type 0).
            let o = inner_km * 30.0;
            if !(t > 2000.0 && inner_km * 2.0 < o) {
                return None;
            }
            o
        }
    };
    // Heating fraction: 0 at 3000 K, 1 from 10000 K, squared.
    let heat = ((t - 3000.0) / 7000.0).clamp(0.0, 1.0).powi(2);
    let inner_thick_km = k.inner_thick_km.unwrap_or_else(|| (rng.uniform() * 0.75 + 0.25) * 0.2 * r);
    let outer_thick_km = k.outer_thick_km.unwrap_or(inner_thick_km * (heat * 18.0 + 2.0)).max(r * 0.55);
    let density = k.density.unwrap_or_else(|| {
        let a = (1.0 - (3.0 / x_dens).sqrt()).powf(0.55);
        let b = (accretion_rate * 6.301_215_74e9).powf(0.55) * 3.1e-8;
        let c = (rg / RG_PER_MSUN_KM).powf(0.625);
        let d = (rg * x_dens * 1.0e-5).powf(-1.875);
        let rho = a * b * c * d;
        if wd { rho } else { rho.min(0.001) }
    });
    let luminosity = k.luminosity.unwrap_or_else(|| {
        let m = rg / RG_PER_MSUN_KM;
        let acc = (accretion_rate / (m * 3.0e-8)).powf(0.666_666_687) * m.powf(1.333_333_37) * 26.13;
        // Thermal light of the inner edge: at 4.32 Rg, at 2 R for a white dwarf.
        let (tt, rr) = if wd { (rate_temperature(accretion_rate, rg, r / rg), 2.0 * r / 695_700.0) } else { (t, rg * 4.32 / 695_700.0) };
        let thermal = (tt / 5572.0).powi(4) * (rr * rr * 0.1) * 0.5;
        thermal.max(acc)
    });
    let luminosity_bol = k.luminosity_bol.unwrap_or_else(|| {
        let bc = if t < 10300.0 {
            -0.41
        } else if t <= 47300.0 {
            -0.41 - (t - 10300.0) * 3.999_999_76 / 37000.0
        } else {
            -4.41
        };
        (bc * -0.921_034_038).exp() / 1.202_264_43 * luminosity
    });
    let jet_start_radius_km = k.jet_start_radius_km.unwrap_or(if wd { 3.0 } else { 1.5 } * r);
    let jet_start_temp = k.jet_start_temp.unwrap_or(t);
    Some(DiskParams {
        inner_km,
        outer_km,
        inner_thick_km,
        outer_thick_km,
        thickness_pow: k.thickness_pow.unwrap_or(3.0),
        detail_r: k.detail_r.unwrap_or(6.0),
        detail_v: k.detail_v.unwrap_or(6.0 - heat * 5.0),
        twist: k.twist.unwrap_or(2500.0),
        octave_dist: [
            k.octave_dist[0].unwrap_or(0.5),
            k.octave_dist[1].unwrap_or(0.35),
            k.octave_dist[2].unwrap_or(0.2),
        ],
        octave_scale: k.octave_scale.unwrap_or(0.5),
        noise_contrast: k.noise_contrast.unwrap_or(1.0),
        temp_contrast: k.temp_contrast.unwrap_or(0.15),
        outer_spin: k.outer_spin.unwrap_or(0.04),
        luminosity,
        luminosity_bol,
        temperature: t,
        accretion_rate,
        density,
        brightness: k.brightness.unwrap_or(1.0),
        lighting_bright: k.lighting_bright.unwrap_or(1.0),
        shadow_contrast: k.shadow_contrast.unwrap_or(1.0),
        shadow_length: k.shadow_length.unwrap_or(0.5),
        jet_velocity: k.jet_velocity.unwrap_or(0.1 + heat * 0.75),
        jet_length_km: k.jet_length_km.unwrap_or(jet_start_radius_km * 30.0),
        jet_start_radius_km,
        jet_end_radius_km: k.jet_end_radius_km.unwrap_or(if wd { 2.5 } else { 5.0 } * jet_start_radius_km),
        jet_start_temp,
        jet_end_temp: k.jet_end_temp.unwrap_or(jet_start_temp * 0.75),
        jet_thickness: k.jet_thickness.unwrap_or(0.3),
        jet_distortion: k.jet_distortion.unwrap_or(0.4),
        jet_density: k.jet_density.unwrap_or(heat.powi(3).clamp(0.0, 1.0) * 1.0e-4 * density),
        jet_brightness: k.jet_brightness.unwrap_or(1.0),
    })
}

/// Disk parameters of a catalog black hole, neutron star or white dwarf (its `AccretionDisk`
/// block, `NoAccretionDisk`), `radius_km` the star's radius (unused for a hole).
pub fn of_star(block: &Block, name: &str, kind: Remnant, mass_sun: f64, radius_km: f64) -> Option<DiskParams> {
    let rg_km = RG_PER_MSUN_KM * mass_sun;
    let r_km = if kind == Remnant::BlackHole { rg_km } else { radius_km };
    let keys = block.sub("AccretionDisk").map(DiskKeys::parse);
    let no_disk = block.bool("NoAccretionDisk").unwrap_or(false);
    resolve(keys.as_ref(), no_disk, kind, mass_sun, r_km, rg_km, &mut SeRng::from_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(src: &str) -> Block {
        se_core::sc::parse(src)
    }

    #[test]
    fn au_keys_override_km_keys() {
        let b = block("InnerRadiusKm 100\nInnerRadius 1\nThicknessKm 5\nOuterThickness 2");
        let k = DiskKeys::parse(&b);
        assert!((k.inner_km.unwrap() - AU_KM).abs() < 1.0);
        assert_eq!(k.inner_thick_km, Some(5.0));
        assert!((k.outer_thick_km.unwrap() - 2.0 * AU_KM).abs() < 1.0);
    }

    #[test]
    fn stellar_defaults_follow_se() {
        // A 10 solar mass hole with an empty `AccretionDisk { }` and a fixed temperature.
        let keys = DiskKeys { temperature: Some(6000.0), ..Default::default() };
        let r = 2.953_25 * 10.0;
        let d = resolve(Some(&keys), false, Remnant::BlackHole, 10.0, r, r, &mut SeRng::from_name("t")).unwrap();
        assert!((d.inner_km - 1.01 * r).abs() < 1e-9);
        assert!((d.outer_km - 30.3 * r).abs() < 1e-6);
        let heat = (3000.0f64 / 7000.0).powi(2);
        assert!((d.detail_v - (6.0 - 5.0 * heat)).abs() < 1e-9);
        assert!((d.jet_length_km - 45.0 * r).abs() < 1e-6);
        assert!((d.jet_end_radius_km - 7.5 * r).abs() < 1e-6);
        assert!(d.inner_thick_km >= 0.05 * r && d.inner_thick_km <= 0.2 * r);
        assert!(d.outer_thick_km >= 0.55 * r);
        assert!(d.density > 0.0 && d.density <= 0.001);
        assert!((d.jet_end_temp - 4500.0).abs() < 1e-9);
        assert_eq!((d.twist, d.detail_r, d.octave_scale, d.temp_contrast), (2500.0, 6.0, 0.5, 0.15));
    }

    #[test]
    fn cold_or_missing_disks_drop() {
        let hole = Remnant::BlackHole;
        let cold = DiskKeys { temperature: Some(1500.0), ..Default::default() };
        assert!(resolve(Some(&cold), false, hole, 10.0, 29.5, 29.5, &mut SeRng::from_name("c")).is_none());
        assert!(resolve(Some(&DiskKeys::default()), true, hole, 10.0, 29.5, 29.5, &mut SeRng::from_name("n")).is_none());
        // Without the tag a stellar hole gets a disk about one time in five; a supermassive one always.
        let n = (0..1000).filter(|i| resolve(None, false, hole, 10.0, 29.5, 29.5, &mut SeRng::from_name(&format!("h{i}"))).is_some()).count();
        assert!((120..280).contains(&n), "{n}");
        assert!(resolve(None, false, hole, 4.0e6, 1.2e7, 1.2e7, &mut SeRng::from_name("smbh")).is_some());
        // Neutron stars and white dwarfs roll the same 20%.
        let ns = (0..1000).filter(|i| resolve(None, false, Remnant::NeutronStar, 1.4, 12.0, 4.13, &mut SeRng::from_name(&format!("q{i}"))).is_some()).count();
        assert!((120..280).contains(&ns), "{ns}");
    }

    #[test]
    fn bolometric_correction() {
        let keys = DiskKeys { temperature: Some(5000.0), luminosity: Some(100.0), ..Default::default() };
        let d = resolve(Some(&keys), false, Remnant::BlackHole, 10.0, 29.5, 29.5, &mut SeRng::from_name("b")).unwrap();
        assert!((d.luminosity_bol - 10f64.powf(0.4 * 0.41) / 1.202_264_43 * 100.0).abs() < 1e-6);
    }

    #[test]
    fn neutron_star_disk_follows_se() {
        // The Crab Pulsar's `AccretionDisk { Brightness 1 }`: 1.4 solar masses, radius 10 km. The
        // geometry follows the star's radius, the alpha-disk formulas its Schwarzschild radius.
        let (r, rg) = (10.0, RG_PER_MSUN_KM * 1.4);
        let keys = DiskKeys { brightness: Some(1.0), ..Default::default() };
        let d = resolve(Some(&keys), false, Remnant::NeutronStar, 1.4, r, rg, &mut SeRng::from_name("Crab Pulsar")).unwrap();
        assert!((d.inner_km - 10.1).abs() < 1e-9);
        assert!((d.outer_km - 303.0).abs() < 1e-6);
        assert!((d.jet_start_radius_km - 15.0).abs() < 1e-9 && (d.jet_end_radius_km - 75.0).abs() < 1e-9);
        // N(30000, 3000) K: hot, so jets and a thick outer disk.
        assert!((15_000.0..45_000.0).contains(&d.temperature), "{}", d.temperature);
        assert!((d.jet_velocity - 0.85).abs() < 1e-9);
        assert!(d.density > 0.0 && d.density <= 0.001);
        assert!(d.accretion_rate <= 1.4 * 3.0e-8);
    }

    #[test]
    fn white_dwarf_disk_follows_se() {
        // A white dwarf: x = R / Rg replaces 4.32 and 3.94453, the temperature is the rate's
        // (FUN_1404dfdc0), jets start at 3 R and end at 2.5 x that, the density is not capped.
        let (m, r) = (1.0, 7000.0);
        let rg = RG_PER_MSUN_KM * m;
        let keys = DiskKeys { temperature: Some(20_000.0), ..Default::default() };
        let d = resolve(Some(&keys), false, Remnant::WhiteDwarf, m, r, rg, &mut SeRng::from_name("wd")).unwrap();
        assert!((d.jet_start_radius_km - 21_000.0).abs() < 1e-6 && (d.jet_end_radius_km - 52_500.0).abs() < 1e-6);
        // Uncapped rate: the temperature comes back (rate_temperature inverts the rate formula).
        if d.accretion_rate < m * 3.0e-8 {
            assert!((d.temperature / 20_000.0 - 1.0).abs() < 1e-3, "{}", d.temperature);
        } else {
            assert!(d.temperature < 20_000.0);
        }
        let back = rate_temperature(d.accretion_rate, rg, r / rg);
        assert!((back / d.temperature - 1.0).abs() < 1e-3);
    }

    #[test]
    fn spin_in_range() {
        let mut rng = SeRng::from_name("s");
        for _ in 0..100 {
            let s = default_spin(Remnant::BlackHole, 10.0, &mut rng);
            assert!((0.0..=0.9999).contains(&s));
            let b = default_spin(Remnant::BlackHole, 1.0e6, &mut rng);
            assert!((0.4999..=0.9999).contains(&b));
            let q = default_spin(Remnant::NeutronStar, 1.4, &mut rng);
            assert!((0.0..=0.9999).contains(&q));
            let w = default_spin(Remnant::WhiteDwarf, 0.6, &mut rng);
            assert!((0.0..=0.04).contains(&w));
        }
    }
}
