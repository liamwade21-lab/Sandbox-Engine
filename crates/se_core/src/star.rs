//! Stellar physics helpers: spectral classification, temperatures, colors,
//! magnitudes and luminosities.

use crate::units::{SOLAR_ABS_MAG_BOL, SOLAR_ABS_MAG_V, SOLAR_TEFF};

/// Broad stellar categories that change how a star is rendered and generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StarKind {
    #[default]
    Normal,
    WolfRayet,
    WhiteDwarf,
    BrownDwarf,
    NeutronStar,
    BlackHole,
    Carbon,
    Subdwarf,
    /// A rogue planet in SE's star octree (class 12, no light): its system is a planet with moons.
    Planemo,
}

/// Luminosity class (MK system).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum LumClass {
    Hypergiant,
    Ia,
    Ib,
    II,
    III,
    IV,
    #[default]
    V,
    VI,
    VII,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpectralClass {
    pub kind: StarKind,
    /// Main letter: O B A F G K M L T Y (W for Wolf-Rayet, D for white dwarfs,
    /// C/S for carbon stars, Q neutron star, X black hole).
    pub letter: char,
    /// Subclass 0..9.9, defaults to 5 when absent.
    pub subclass: f64,
    pub lum_class: LumClass,
    pub raw: String,
}

impl SpectralClass {
    /// Parse an MK-style class like "G2V", "M5 V", "K2II", "DA2", "WN7", "sdB5",
    /// "L3", "T5", "B1.5Ia+", "Q" (neutron star) or "X" (black hole).
    pub fn parse(s: &str) -> Self {
        let raw = s.trim().to_string();
        if raw.eq_ignore_ascii_case("Planemo") {
            return Self { kind: StarKind::Planemo, letter: 'P', subclass: 0.0, lum_class: LumClass::V, raw };
        }
        let mut rest = raw.as_str();
        let mut kind = StarKind::Normal;
        let mut lum = LumClass::V;
        if let Some(r) = rest.strip_prefix("sd") {
            kind = StarKind::Subdwarf;
            lum = LumClass::VI;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("esd").or_else(|| rest.strip_prefix("usd")) {
            kind = StarKind::Subdwarf;
            lum = LumClass::VI;
            rest = r;
        }
        let mut chars = rest.chars();
        let first = chars.next().unwrap_or('G').to_ascii_uppercase();
        let wd_prefix = rest.len() >= 2 && rest[..2].eq_ignore_ascii_case("WD");
        let (letter, after) = match first {
            'W' if wd_prefix => {
                kind = StarKind::WhiteDwarf;
                lum = LumClass::VII;
                ('D', &rest[2..])
            }
            'W' => {
                kind = StarKind::WolfRayet;
                // WN / WC / WO
                let tail = chars.as_str();
                let tail = tail.trim_start_matches(['N', 'C', 'O', 'R']);
                ('W', tail)
            }
            'D' => {
                kind = StarKind::WhiteDwarf;
                lum = LumClass::VII;
                let tail = chars.as_str().trim_start_matches(|c: char| c.is_ascii_uppercase());
                ('D', tail)
            }
            'Q' => {
                kind = StarKind::NeutronStar;
                ('Q', chars.as_str())
            }
            'X' => {
                kind = StarKind::BlackHole;
                ('X', chars.as_str())
            }
            'L' | 'T' | 'Y' => {
                kind = StarKind::BrownDwarf;
                (first, chars.as_str())
            }
            'C' | 'S' | 'N' | 'R' => {
                kind = StarKind::Carbon;
                lum = LumClass::III;
                let tail = chars.as_str().trim_start_matches(['-', 'N', 'R', 'J', 'H']);
                (first, tail)
            }
            'O' | 'B' | 'A' | 'F' | 'G' | 'K' | 'M' => (first, chars.as_str()),
            _ => ('G', rest),
        };
        let after = after.trim_start();
        let num_len = after
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_digit() || *c == '.')
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        let subclass = after[..num_len].parse::<f64>().unwrap_or(5.0).clamp(0.0, 9.99);
        let lum_str = after[num_len..].trim();
        if !matches!(kind, StarKind::WhiteDwarf | StarKind::Subdwarf) {
            if let Some(l) = parse_lum_class(lum_str) {
                lum = l;
            }
        }
        Self {
            kind,
            letter,
            subclass,
            lum_class: lum,
            raw,
        }
    }

    /// Effective temperature estimate (K) from the class.
    pub fn temperature(&self) -> f64 {
        match self.kind {
            StarKind::WhiteDwarf => {
                // DA2 -> 50400 / 2 = 25200 K
                if self.subclass > 0.0 { 50_400.0 / self.subclass } else { 50_000.0 }
            }
            StarKind::WolfRayet => interpolate(&WR_TEMPS, self.subclass),
            StarKind::NeutronStar => 600_000.0,
            StarKind::BlackHole => 0.0,
            StarKind::Carbon => interpolate(&CARBON_TEMPS, self.subclass),
            _ => {
                let table: &[(f64, f64)] = match self.letter {
                    'O' => &O_TEMPS,
                    'B' => &B_TEMPS,
                    'A' => &A_TEMPS,
                    'F' => &F_TEMPS,
                    'G' => &G_TEMPS,
                    'K' => &K_TEMPS,
                    'M' => &M_TEMPS,
                    'L' => &L_TEMPS,
                    'T' => &T_TEMPS,
                    'Y' => &Y_TEMPS,
                    _ => &G_TEMPS,
                };
                let t = interpolate(table, self.subclass);
                // Giants of late types are a little cooler than dwarfs.
                match (self.letter, self.lum_class) {
                    ('G' | 'K' | 'M', l) if l <= LumClass::III => t * 0.92,
                    _ => t,
                }
            }
        }
    }

    /// Typical absolute visual magnitude for the class (rough MK calibration).
    pub fn typical_abs_mag(&self) -> f64 {
        match self.kind {
            StarKind::WhiteDwarf => 11.5 + 0.3 * self.subclass,
            StarKind::NeutronStar => 20.0,
            StarKind::BlackHole => 30.0,
            StarKind::WolfRayet => -4.5,
            StarKind::BrownDwarf => match self.letter {
                'L' => 12.0 + 0.5 * self.subclass,
                'T' => 15.0 + 0.4 * self.subclass,
                _ => 20.0,
            },
            _ => {
                let ms = main_sequence_abs_mag(self.temperature());
                match self.lum_class {
                    LumClass::Hypergiant => -9.0,
                    LumClass::Ia => -7.5,
                    LumClass::Ib => -5.0,
                    LumClass::II => -2.5,
                    LumClass::III => ms.min(0.7),
                    LumClass::IV => ms - 1.5,
                    LumClass::V => ms,
                    LumClass::VI => ms + 1.5,
                    LumClass::VII => 12.0,
                }
            }
        }
    }
}

fn parse_lum_class(s: &str) -> Option<LumClass> {
    let s = s.trim_start_matches(['(', ' ']);
    if s.is_empty() {
        return None;
    }
    let order = [
        ("0", LumClass::Hypergiant),
        ("Ia+", LumClass::Hypergiant),
        ("Iab", LumClass::Ib),
        ("Ia", LumClass::Ia),
        ("Ib", LumClass::Ib),
        ("VII", LumClass::VII),
        ("VI", LumClass::VI),
        ("IV", LumClass::IV),
        ("V", LumClass::V),
        ("III", LumClass::III),
        ("II", LumClass::II),
        ("I", LumClass::Ia),
    ];
    order.iter().find(|(p, _)| s.starts_with(p)).map(|(_, l)| *l)
}

fn interpolate(table: &[(f64, f64)], x: f64) -> f64 {
    if x <= table[0].0 {
        return table[0].1;
    }
    for w in table.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if x <= x1 {
            let t = (x - x0) / (x1 - x0);
            return y0 + (y1 - y0) * t;
        }
    }
    table[table.len() - 1].1
}

// Main sequence temperature calibration (Pecaut & Mamajek 2013, abbreviated).
const O_TEMPS: [(f64, f64); 4] = [(2.0, 54_000.0), (5.0, 41_400.0), (8.0, 34_000.0), (9.5, 31_900.0)];
const B_TEMPS: [(f64, f64); 6] = [
    (0.0, 31_400.0),
    (1.0, 26_000.0),
    (3.0, 17_000.0),
    (5.0, 15_700.0),
    (8.0, 12_500.0),
    (9.5, 10_700.0),
];
const A_TEMPS: [(f64, f64); 5] = [(0.0, 9_700.0), (2.0, 8_840.0), (5.0, 8_080.0), (7.0, 7_800.0), (9.0, 7_400.0)];
const F_TEMPS: [(f64, f64); 5] = [(0.0, 7_220.0), (2.0, 6_810.0), (5.0, 6_510.0), (8.0, 6_170.0), (9.5, 6_000.0)];
const G_TEMPS: [(f64, f64); 5] = [(0.0, 5_920.0), (2.0, 5_770.0), (5.0, 5_660.0), (8.0, 5_490.0), (9.5, 5_340.0)];
const K_TEMPS: [(f64, f64); 6] = [
    (0.0, 5_280.0),
    (2.0, 5_040.0),
    (4.0, 4_600.0),
    (5.0, 4_440.0),
    (7.0, 4_050.0),
    (9.0, 3_940.0),
];
const M_TEMPS: [(f64, f64); 7] = [
    (0.0, 3_850.0),
    (2.0, 3_550.0),
    (4.0, 3_200.0),
    (5.0, 3_050.0),
    (6.0, 2_800.0),
    (8.0, 2_500.0),
    (9.5, 2_300.0),
];
const L_TEMPS: [(f64, f64); 4] = [(0.0, 2_250.0), (3.0, 1_900.0), (6.0, 1_600.0), (9.0, 1_350.0)];
const T_TEMPS: [(f64, f64); 4] = [(0.0, 1_250.0), (3.0, 1_100.0), (6.0, 950.0), (9.0, 600.0)];
const Y_TEMPS: [(f64, f64); 3] = [(0.0, 450.0), (1.0, 400.0), (4.0, 250.0)];
const WR_TEMPS: [(f64, f64); 4] = [(2.0, 140_000.0), (5.0, 80_000.0), (7.0, 55_000.0), (9.0, 35_000.0)];
const CARBON_TEMPS: [(f64, f64); 3] = [(0.0, 4_500.0), (5.0, 3_200.0), (9.0, 2_400.0)];

/// Approximate main-sequence absolute V magnitude for a temperature.
pub fn main_sequence_abs_mag(teff: f64) -> f64 {
    // Piecewise fit over the main sequence (Pecaut & Mamajek): log T -> Mv.
    const TABLE: [(f64, f64); 12] = [
        (2_300.0, 19.0),
        (2_800.0, 16.0),
        (3_200.0, 12.0),
        (3_850.0, 8.9),
        (4_440.0, 7.3),
        (5_280.0, 5.9),
        (5_770.0, 4.8),
        (6_510.0, 3.4),
        (8_080.0, 1.9),
        (9_700.0, 1.1),
        (15_700.0, -1.1),
        (31_400.0, -3.9),
    ];
    if teff >= 31_400.0 {
        return -3.9 - 2.0 * (teff / 31_400.0).log10() * 3.0;
    }
    interpolate(&TABLE, teff.max(2_300.0))
}

/// Absolute magnitude from apparent magnitude and distance in parsecs.
pub fn abs_mag_from_app(app_mag: f64, dist_pc: f64) -> f64 {
    app_mag - 5.0 * (dist_pc.max(1e-12) / 10.0).log10()
}

/// Apparent magnitude from absolute magnitude and distance in parsecs.
pub fn app_mag_from_abs(abs_mag: f64, dist_pc: f64) -> f64 {
    abs_mag + 5.0 * (dist_pc.max(1e-12) / 10.0).log10()
}

/// Visual luminosity (solar units) from absolute visual magnitude.
pub fn lum_from_abs_mag(abs_mag: f64) -> f64 {
    10f64.powf(0.4 * (SOLAR_ABS_MAG_V - abs_mag))
}

pub fn abs_mag_from_lum(lum: f64) -> f64 {
    SOLAR_ABS_MAG_V - 2.5 * lum.max(1e-30).log10()
}

pub fn abs_bol_mag_from_lum(lum_bol: f64) -> f64 {
    SOLAR_ABS_MAG_BOL - 2.5 * lum_bol.max(1e-30).log10()
}

/// Bolometric correction BC_V(T) (Torres 2010 / Flower 1996 polynomial).
pub fn bolometric_correction(teff: f64) -> f64 {
    let lt = teff.max(1_000.0).log10();
    if teff < 3_000.0 {
        // The Torres polynomial diverges below ~3000 K (it gives -74 at 1300 K); continue
        // linearly in log T to about -8 at 1000 K, as observed for L and T dwarfs.
        let lt0 = 3_000f64.log10();
        return bolometric_correction(3_000.0) + 5.45 * (lt - lt0);
    }
    if lt < 3.70 {
        (-0.190_537_291_496_456e5 + 0.155_144_866_764_412e5 * lt - 0.421_278_819_301_717e4 * lt * lt
            + 0.381_476_328_422_343e3 * lt.powi(3))
        .min(0.0)
    } else if lt < 3.90 {
        (-0.370_510_203_809_015e5 + 0.385_672_629_965_804e5 * lt - 0.150_651_486_316_025e5 * lt * lt
            + 0.261_724_637_119_416e4 * lt.powi(3)
            - 0.170_623_810_323_864e3 * lt.powi(4))
        .min(0.0)
    } else {
        (-0.118_115_450_538_963e6 + 0.137_145_973_583_929e6 * lt - 0.636_233_812_100_225e5 * lt * lt
            + 0.147_412_923_562_646e5 * lt.powi(3)
            - 0.170_587_278_406_872e4 * lt.powi(4)
            + 0.788_731_721_804_990e2 * lt.powi(5))
        .min(0.0)
    }
}

/// Stellar radius (solar radii) from bolometric luminosity (solar) and Teff.
pub fn radius_from_lum_teff(lum_bol: f64, teff: f64) -> f64 {
    lum_bol.max(0.0).sqrt() / (teff / SOLAR_TEFF).powi(2)
}

/// Linear sRGB color (max channel = 1) of a blackbody at `teff` kelvin, using the
/// Wyman/Sloan/Shirley analytic fit of the CIE 1931 colour matching functions.
pub fn blackbody_rgb(teff: f64) -> [f32; 3] {
    if teff <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let t = teff.clamp(500.0, 1.0e6);
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    let mut lambda = 380.0;
    while lambda <= 780.0 {
        let b = planck(lambda * 1e-9, t);
        let (cx, cy, cz) = cie_cmf(lambda);
        x += b * cx;
        y += b * cy;
        z += b * cz;
        lambda += 5.0;
    }
    let r = 3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z;
    let g = -0.969_266_0 * x + 1.876_010_8 * y + 0.041_556_0 * z;
    let bl = 0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z;
    let (r, g, bl) = (r.max(0.0), g.max(0.0), bl.max(0.0));
    let m = r.max(g).max(bl).max(1e-30);
    [(r / m) as f32, (g / m) as f32, (bl / m) as f32]
}

fn planck(lambda_m: f64, t: f64) -> f64 {
    const H: f64 = 6.626_070_15e-34;
    const C: f64 = 2.997_924_58e8;
    const K: f64 = 1.380_649e-23;
    let a = 2.0 * H * C * C / lambda_m.powi(5);
    let e = (H * C / (lambda_m * K * t)).min(700.0);
    a / (e.exp() - 1.0)
}

fn cie_cmf(l: f64) -> (f64, f64, f64) {
    fn g(x: f64, mu: f64, s1: f64, s2: f64) -> f64 {
        let s = if x < mu { s1 } else { s2 };
        (-0.5 * ((x - mu) / s).powi(2)).exp()
    }
    let x = 1.056 * g(l, 599.8, 37.9, 31.0) + 0.362 * g(l, 442.0, 16.0, 26.7) - 0.065 * g(l, 501.1, 20.4, 26.2);
    let y = 0.821 * g(l, 568.8, 46.9, 40.5) + 0.286 * g(l, 530.9, 16.3, 31.1);
    let z = 1.217 * g(l, 437.0, 11.8, 36.0) + 0.681 * g(l, 459.0, 26.0, 13.8);
    (x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_classes() {
        let s = SpectralClass::parse("G2V");
        assert_eq!((s.letter, s.subclass, s.lum_class), ('G', 2.0, LumClass::V));
        assert!((s.temperature() - 5770.0).abs() < 1.0);
        let s = SpectralClass::parse("M5 V");
        assert_eq!((s.letter, s.subclass), ('M', 5.0));
        let s = SpectralClass::parse("K2II");
        assert_eq!(s.lum_class, LumClass::II);
        let s = SpectralClass::parse("DA2");
        assert_eq!(s.kind, StarKind::WhiteDwarf);
        assert!((s.temperature() - 25_200.0).abs() < 1.0);
        let s = SpectralClass::parse("B1.5Ia+");
        assert_eq!((s.letter, s.subclass, s.lum_class), ('B', 1.5, LumClass::Hypergiant));
        assert_eq!(SpectralClass::parse("WN7").kind, StarKind::WolfRayet);
        assert_eq!(SpectralClass::parse("WD").kind, StarKind::WhiteDwarf);
        assert_eq!(SpectralClass::parse("WD1856+534").kind, StarKind::WhiteDwarf);
        assert_eq!(SpectralClass::parse("T5").kind, StarKind::BrownDwarf);
        assert_eq!(SpectralClass::parse("sdB5").kind, StarKind::Subdwarf);
        assert_eq!(SpectralClass::parse("X").kind, StarKind::BlackHole);
        assert_eq!(SpectralClass::parse("Q").kind, StarKind::NeutronStar);
    }

    #[test]
    fn magnitudes() {
        let m = abs_mag_from_app(-26.74, 1.0 / 206_264.806);
        assert!((m - 4.83).abs() < 0.02, "{m}");
        assert!((lum_from_abs_mag(SOLAR_ABS_MAG_V) - 1.0).abs() < 1e-12);
        assert!((radius_from_lum_teff(1.0, SOLAR_TEFF) - 1.0).abs() < 1e-12);
        assert!(bolometric_correction(5778.0).abs() < 0.2);
    }

    #[test]
    fn blackbody_colors() {
        let sun = blackbody_rgb(5778.0);
        assert!(sun[0] > 0.9 && sun[1] > 0.8 && sun[2] > 0.7, "{sun:?}");
        let red = blackbody_rgb(3000.0);
        assert!(red[0] == 1.0 && red[2] < 0.5, "{red:?}");
        let blue = blackbody_rgb(30_000.0);
        assert!(blue[2] == 1.0 && blue[0] < 0.8, "{blue:?}");
    }
}
