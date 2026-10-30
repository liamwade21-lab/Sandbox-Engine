//! Procedural surfaces for bodies without texture maps (and fine detail for
//! textured ones): terrain height functions and colour palettes chosen from the
//! body's class, size and temperature.

use glam::DVec3;

use crate::noise::{craters, craters_from, fbm, gradient, ridged};
use crate::surface_gen::{self as sg, SurfaceGen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    /// Silicate world (with or without atmosphere).
    Rocky,
    /// Airless, heavily cratered (Moon / Mercury like).
    Cratered,
    /// Icy world.
    Icy,
    /// Molten surface.
    Lava,
    /// Dark carbon-rich world.
    Carbon,
    /// Iron-rich world.
    Iron,
    /// Hydrogen/helium gas giant (banded).
    GasGiant,
    /// Ice giant (smooth, bluish).
    IceGiant,
    /// Global ocean world.
    Ocean,
}

impl SurfaceKind {
    pub fn id(self) -> f32 {
        self as u32 as f32
    }

    pub fn is_giant(self) -> bool {
        matches!(self, SurfaceKind::GasGiant | SurfaceKind::IceGiant)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcSurface {
    pub seed: u64,
    pub kind: SurfaceKind,
    pub radius_m: f64,
    /// Characteristic relief amplitude, meters.
    pub relief_m: f64,
    /// Fraction of the surface under water (0 = no ocean).
    pub ocean_fraction: f64,
    pub crater_strength: f64,
    pub mountain_strength: f64,
    pub temperature_k: f64,
    pub has_life: bool,
    pub has_atmosphere: bool,
    /// Linear RGB palette: lowland, midland, highland, polar/peak.
    pub palette: [[f32; 3]; 4],
    /// SpaceEngine's generated surface parameters ([`crate::surface_gen`]); when present the
    /// height comes from [`ProcSurface::se_height`].
    pub se: Option<SurfaceGen>,
    /// `BumpHeight` (m): the span of the SE height field.
    pub bump_m: f64,
}

/// Seed from a body name (stable across runs).
pub fn name_seed(name: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

impl ProcSurface {
    /// Choose a surface for a body. `class` is the SE planet class (e.g. "Terra",
    /// "Selena", "Jupiter"); `temperature_k` the equilibrium/surface temperature.
    pub fn for_body(
        name: &str,
        class: Option<&str>,
        radius_m: f64,
        mass_kg: f64,
        temperature_k: f64,
        has_atmosphere: bool,
        has_ocean: bool,
        has_life: bool,
    ) -> Self {
        let seed = name_seed(name);
        let class_l = class.unwrap_or("").to_ascii_lowercase();
        let rocky_size = radius_m < 15_000_000.0;
        let kind = if class_l.contains("jupiter") || class_l.contains("gas") {
            SurfaceKind::GasGiant
        } else if class_l.contains("neptune") {
            SurfaceKind::IceGiant
        } else if !rocky_size {
            if temperature_k < 160.0 { SurfaceKind::IceGiant } else { SurfaceKind::GasGiant }
        } else if temperature_k > 900.0 {
            SurfaceKind::Lava
        } else if class_l.contains("carbon") {
            SurfaceKind::Carbon
        } else if class_l.contains("ferria") || class_l.contains("iron") {
            if has_atmosphere { SurfaceKind::Iron } else { SurfaceKind::Cratered }
        } else if class_l.contains("aquaria") && has_ocean {
            SurfaceKind::Ocean
        } else if class_l.contains("glacia") || class_l.contains("ice") || (temperature_k < 150.0 && !has_atmosphere) {
            SurfaceKind::Icy
        } else if !has_atmosphere {
            SurfaceKind::Cratered
        } else {
            SurfaceKind::Rocky
        };
        // Relief scales inversely with surface gravity.
        let g = if radius_m > 0.0 && mass_kg > 0.0 { crate::units::G * mass_kg / (radius_m * radius_m) } else { 1.0 };
        // max/min rather than clamp: tiny bodies have radius*0.04 below the 1.5 km floor.
        let relief_m = (9_000.0 * (9.81 / g.max(0.01)).sqrt()).max(1_500.0).min(radius_m * 0.04).max(10.0);
        let ocean_fraction = match kind {
            SurfaceKind::Ocean => 0.97,
            SurfaceKind::Rocky if has_ocean => 0.55 + 0.25 * ((seed >> 7) % 100) as f64 / 100.0,
            _ => 0.0,
        };
        let t = temperature_k as f32;
        let palette = match kind {
            SurfaceKind::Rocky if has_life => [
                [0.10, 0.22, 0.08],
                [0.30, 0.32, 0.18],
                [0.42, 0.37, 0.30],
                [0.90, 0.92, 0.95],
            ],
            SurfaceKind::Rocky => {
                // Deserts: warmer worlds are redder.
                let warm = ((t - 200.0) / 300.0).clamp(0.0, 1.0);
                [
                    mix([0.45, 0.42, 0.38], [0.62, 0.40, 0.24], warm),
                    mix([0.55, 0.50, 0.44], [0.70, 0.52, 0.34], warm),
                    mix([0.35, 0.33, 0.31], [0.45, 0.30, 0.22], warm),
                    if t < 270.0 { [0.92, 0.93, 0.95] } else { [0.75, 0.62, 0.48] },
                ]
            }
            SurfaceKind::Cratered => [[0.30, 0.29, 0.28], [0.42, 0.41, 0.39], [0.52, 0.51, 0.49], [0.62, 0.61, 0.60]],
            SurfaceKind::Icy => [[0.55, 0.60, 0.66], [0.72, 0.76, 0.80], [0.85, 0.87, 0.90], [0.95, 0.96, 0.98]],
            SurfaceKind::Lava => [[0.08, 0.07, 0.07], [0.14, 0.11, 0.10], [0.22, 0.18, 0.16], [0.30, 0.25, 0.22]],
            SurfaceKind::Carbon => [[0.09, 0.08, 0.08], [0.14, 0.13, 0.12], [0.20, 0.18, 0.17], [0.30, 0.28, 0.26]],
            SurfaceKind::Iron => [[0.32, 0.22, 0.18], [0.45, 0.33, 0.27], [0.55, 0.45, 0.40], [0.70, 0.66, 0.62]],
            SurfaceKind::Ocean => [[0.25, 0.30, 0.20], [0.40, 0.38, 0.30], [0.55, 0.52, 0.48], [0.92, 0.93, 0.96]],
            SurfaceKind::GasGiant => {
                let hot = ((t - 400.0) / 800.0).clamp(0.0, 1.0);
                [
                    mix([0.82, 0.72, 0.58], [0.35, 0.22, 0.18], hot),
                    mix([0.68, 0.52, 0.38], [0.25, 0.14, 0.12], hot),
                    mix([0.90, 0.86, 0.78], [0.45, 0.30, 0.24], hot),
                    mix([0.55, 0.42, 0.33], [0.20, 0.10, 0.08], hot),
                ]
            }
            SurfaceKind::IceGiant => [[0.45, 0.62, 0.85], [0.55, 0.72, 0.90], [0.62, 0.80, 0.92], [0.40, 0.55, 0.80]],
        };
        Self {
            seed,
            kind,
            radius_m,
            relief_m,
            ocean_fraction,
            crater_strength: match kind {
                SurfaceKind::Cratered => 1.0,
                SurfaceKind::Icy | SurfaceKind::Carbon | SurfaceKind::Iron => 0.45,
                // Thin-atmosphere deserts keep their craters (SE's Proxima b, Mars); wet worlds erode them.
                SurfaceKind::Rocky => if !has_atmosphere { 0.6 } else if has_ocean { 0.08 } else { 0.3 },
                _ => 0.0,
            },
            mountain_strength: match kind {
                SurfaceKind::Rocky | SurfaceKind::Iron | SurfaceKind::Ocean => 0.8,
                SurfaceKind::Icy => 0.3,
                SurfaceKind::Lava => 0.4,
                _ => 0.35,
            },
            temperature_k,
            has_life,
            has_atmosphere,
            palette,
            se: None,
            bump_m: 0.0,
        }
    }

    /// Use SE's surface parameters for the relief (`bump_height_km`: the body's `BumpHeight`).
    pub fn with_se(mut self, g: SurfaceGen, bump_height_km: f64) -> Self {
        if !self.kind.is_giant() && bump_height_km > 0.0 {
            self.bump_m = bump_height_km * 1000.0;
            self.relief_m = self.bump_m * 0.35;
            self.se = Some(g);
        }
        self
    }

    /// Height above the reference radius (sea level when there is an ocean), m.
    pub fn height_m(&self, dir: DVec3) -> f64 {
        self.height_m_lod(dir, 0.0)
    }

    /// Number of octaves of a noise whose first octave has feature size
    /// `first_feature_m` and lacunarity `lac`, keeping features above `min_feature_m`.
    fn octaves_for(first_feature_m: f64, lac: f64, min_feature_m: f64, max: u32) -> u32 {
        if min_feature_m <= 0.0 {
            return max;
        }
        let n = ((first_feature_m / min_feature_m).ln() / lac.ln()).floor() + 1.0;
        (n.max(1.0) as u32).min(max)
    }

    /// Height band-limited to features larger than `min_feature_m` (use about
    /// two vertex spacings), so coarse meshes do not alias the noise.
    pub fn height_m_lod(&self, dir: DVec3, min_feature_m: f64) -> f64 {
        if self.kind.is_giant() {
            return 0.0;
        }
        if let Some(g) = &self.se {
            return self.se_height(g, dir, min_feature_m);
        }
        let r = self.radius_m;
        let s = self.seed;
        // Large-scale continents / basins.
        let warp = DVec3::new(
            gradient(dir * 2.0, s ^ 11),
            gradient(dir * 2.0, s ^ 12),
            gradient(dir * 2.0, s ^ 13),
        ) * 0.25;
        let continents = fbm(dir * 1.6 + warp, s, Self::octaves_for(r / 1.6, 2.0, min_feature_m, 6), 2.0, 0.5);
        let mountains = ridged(dir * 5.0 + warp, s ^ 0x55, Self::octaves_for(r / 5.0, 2.1, min_feature_m, 7));
        let hill_oct = Self::octaves_for(r / 30.0, 2.1, min_feature_m, 6);
        let hills = if r / 30.0 > min_feature_m { fbm(dir * 30.0, s ^ 0x77, hill_oct, 2.1, 0.5) } else { 0.0 };
        let mut h = continents * 0.55;
        // Mountains mostly on high ground.
        let land = ((continents + 0.1) * 3.0).clamp(0.0, 1.0);
        h += mountains * self.mountain_strength * (0.25 + 0.75 * land) * 0.6;
        h += hills * 0.06;
        h += self.landforms(dir, warp, land, min_feature_m);
        if self.crater_strength > 0.0 {
            let levels = Self::octaves_for(r * 0.35 * 0.3, 2.0, min_feature_m, 5);
            h += craters(dir, s ^ 0xC4A7, 0.55, levels) * self.crater_strength * 0.35;
        }
        // Shift so the requested fraction of the surface lies below sea level.
        if self.ocean_fraction > 0.0 {
            h -= (self.ocean_fraction - 0.5) * 0.6;
        }
        h * self.relief_m
    }

    /// Mid-scale landforms as SE's terrain shows them on rocky and icy worlds (compared side by side on
    /// Proxima b, docs/procgen.md "Terrain"): terraced plateaus with steep risers, long sinuous scarps,
    /// canyons and, on dry worlds with air, dune fields. In units of `relief_m`.
    fn landforms(&self, dir: DVec3, warp: DVec3, land: f64, min_feature_m: f64) -> f64 {
        if !matches!(self.kind, SurfaceKind::Rocky | SurfaceKind::Icy | SurfaceKind::Iron | SurfaceKind::Carbon | SurfaceKind::Cratered) {
            return 0.0;
        }
        let r = self.radius_m;
        let s = self.seed;
        let dry = self.ocean_fraction == 0.0;
        let airless = matches!(self.kind, SurfaceKind::Cratered);
        let mut h = 0.0;
        // Plateaus: a mid-scale field cut into a few levels with steep risers, where a low-frequency mask
        // allows (SE's mesas and layered highlands).
        let fp = 14.0;
        if r / fp > min_feature_m {
            let w2 = DVec3::new(gradient(dir * 6.0, s ^ 21), gradient(dir * 6.0, s ^ 22), gradient(dir * 6.0, s ^ 23));
            let t = fbm(dir * fp + w2 * 0.8 + warp, s ^ 0x7E2, Self::octaves_for(r / fp, 2.0, min_feature_m, 3), 2.0, 0.5);
            let steps = 3.0;
            let x = ((t * 0.7 + 0.5).clamp(0.0, 0.999)) * steps;
            let k = x.floor();
            let riser = smoothstep(0.4, 0.6, x - k);
            let terr = (k + riser) / steps * 2.0 - 1.0;
            let mask = smoothstep(-0.15, 0.35, gradient(dir * 3.0, s ^ 0x7E3));
            let strength = if airless { 0.08 } else if dry { 0.22 } else { 0.12 };
            h += terr * strength * mask * (0.4 + 0.6 * land);
        }
        // Scarps: sinuous fault lines with a step across them and a ridge on top.
        let fs = 22.0;
        if r / fs * 0.06 > min_feature_m {
            let n = gradient(dir * fs + warp * 2.0, s ^ 0x5CA) + 0.35 * gradient(dir * fs * 3.1, s ^ 0x5CB);
            let line = 1.0 - smoothstep(0.0, 0.05, n.abs());
            let step = smoothstep(-0.05, 0.05, n) - 0.5;
            let mask = smoothstep(0.0, 0.4, gradient(dir * 4.0, s ^ 0x5CC));
            h += (step * 0.07 + line * 0.025) * mask;
        }
        // Canyons on dry worlds: narrow sinuous cuts.
        let fc = 9.0;
        if dry && !airless && r / fc * 0.03 > min_feature_m {
            let n = gradient(dir * fc + warp * 1.5, s ^ 0xCA1) + 0.3 * gradient(dir * fc * 4.3, s ^ 0xCA2);
            let cut = 1.0 - smoothstep(0.0, 0.035, n.abs());
            let mask = smoothstep(0.1, 0.5, gradient(dir * 2.5, s ^ 0xCA3));
            h -= cut * 0.12 * mask;
        }
        // Dunes: crests across a fixed wind, about 1.5 km apart, bent by noise, in dry lowlands with air.
        let lambda = 1500.0;
        if dry && self.has_atmosphere && self.kind == SurfaceKind::Rocky && lambda * 0.3 > min_feature_m {
            let wind = DVec3::new(0.8, 0.1, 0.59).normalize();
            let p = dir * r;
            let bend = gradient(p / 9000.0, s ^ 0xD0E) * 4.0;
            let ph = p.dot(wind) / lambda * std::f64::consts::TAU + bend;
            let crest = (1.0 - ph.sin().abs()).powf(1.5);
            let field = smoothstep(0.0, 0.5, gradient(p / 60000.0, s ^ 0xD0F)) * (1.0 - land);
            h += crest * field * 45.0 / self.relief_m;
        }
        h
    }

    /// Height from SE's surface parameters, m above the SE sea level (`seaLevel` of the bump span;
    /// the mean height on worlds without an ocean).
    /// SE's terrain shader is encrypted, so the composition is designed; what varies from planet to
    /// planet is SE's: the frequencies (per unit sphere), magnitudes (fractions of `BumpHeight`) and
    /// the landform shares, picked per region by cumulative thresholds as SE's shader uniforms are.
    /// docs/procgen.md "Terrain".
    pub fn se_height(&self, g: &SurfaceGen, dir: DVec3, min_feature_m: f64) -> f64 {
        let r = self.radius_m;
        let s = self.seed;
        let v = |i: usize| g.v(i) as f64;
        let oct = |freq: f64, max: u32| Self::octaves_for(r / freq.max(1e-3), 2.0, min_feature_m, max);
        // Terms fade in over a factor 3 of feature size, so neighbouring patches of different
        // resolution do not disagree by a whole landform (visible seams at the limb).
        let fade = |feature_m: f64| if min_feature_m <= 0.0 { 1.0 } else { smoothstep(min_feature_m, min_feature_m * 3.0, feature_m) };
        let on = |freq: f64| r / freq.max(1e-3) > min_feature_m;
        let rnd = DVec3::new(v(0), v(1), v(2)) * 17.0;
        let warp = DVec3::new(gradient(dir * 2.0 + rnd, s ^ 11), gradient(dir * 2.0 + rnd, s ^ 12), gradient(dir * 2.0 + rnd, s ^ 13)) * 0.25;
        // Continents.
        let mf = 1.6 * v(sg::MAIN_FREQ).max(0.05);
        let mut h = 0.5 + 0.4 * fbm(dir * mf + warp + rnd, s, oct(mf, 8), 2.0, 0.5);
        let vm = v(sg::VENUS_MAGN);
        if vm > 0.0 {
            let vf = 3.0 * v(sg::VENUS_FREQ).max(0.1);
            h += vm * 0.12 * fbm(dir * vf + DVec3::new(rnd.y, rnd.z, rnd.x), s ^ 0xE1, oct(vf, 5), 2.0, 0.5).abs();
        }
        let sea = v(sg::SEA_LEVEL);
        let land = smoothstep(sea - 0.03, sea + 0.12, h);
        // Maria: broad basins flooded flat.
        let md = v(sg::MARE_DENSITY);
        if md > 0.0 {
            let mfq = 1.5 * v(sg::MARE_FREQ).max(0.3);
            let n = fbm(dir * mfq + DVec3::new(rnd.z, rnd.x, rnd.y), s ^ 0xA4E, oct(mfq, 3), 2.0, 0.5);
            let m = smoothstep(0.0, 0.12, n + md.min(1.0) * 1.2 - 0.6);
            h += (sea + 0.05 - h).min(0.0) * m * 0.9;
        }
        // Landform shares: one type per region, softly blended at the borders.
        if let Some(t) = sg::landform_thresholds(g) {
            let sel = (fbm(dir * 3.0 + rnd * 0.37, s ^ 0x5E1, 3, 2.0, 0.5) * 1.1 + 0.5).clamp(0.0, 0.9999);
            let bw = 0.04;
            let mut lo = -1.0;
            for (k, hi) in t.iter().map(|&x| x as f64).enumerate() {
                let w = smoothstep(lo - bw, lo + bw, sel) * (1.0 - smoothstep(hi - bw, hi + bw, sel));
                lo = hi;
                if w <= 1e-3 {
                    continue;
                }
                h += w * match k {
                    // Dunes: crests across a wind, a few km apart.
                    0 => {
                        let lam = (r / (v(sg::DUNES_FREQ).max(1.0) * 30.0)).max(300.0);
                        if fade(lam * 0.3) > 0.0 {
                            let wind = DVec3::new(0.8, 0.1, 0.59).normalize();
                            let p = dir * r;
                            let ph = p.dot(wind) / lam * std::f64::consts::TAU + gradient(p / (lam * 6.0), s ^ 0xD0E) * 4.0;
                            (1.0 - ph.sin().abs()).powf(1.5) * v(sg::DUNES_MAGN) * 0.15 * (1.0 - land * 0.5) * fade(lam * 0.3)
                        } else {
                            0.0
                        }
                    }
                    // Hills, and rounded (billowy) hills.
                    1 | 2 => {
                        let f = v(sg::HILLS_FREQ).max(1.0) * if k == 2 { 0.5 } else { 1.0 };
                        if on(f) {
                            let n = fbm(dir * f + warp, s ^ (0x77 + k as u64), oct(f, 6), 2.1, 0.5);
                            v(sg::HILLS_MAGN) * (if k == 2 { n.abs() * 1.5 } else { n * 0.5 + 0.25 }) * fade(r / f)
                        } else {
                            0.0
                        }
                    }
                    // Canyons: sinuous cuts.
                    3 => {
                        let f = (v(sg::CANYONS_FREQ) * 0.1).max(2.0);
                        if fade(r / f * 0.03) > 0.0 {
                            let n = gradient(dir * f + warp * 1.5, s ^ 0xCA1) + 0.3 * gradient(dir * f * 4.3, s ^ 0xCA2);
                            -(1.0 - smoothstep(0.0, 0.035, n.abs())) * v(sg::CANYONS_MAGN) * 0.5 * fade(r / f * 0.03)
                        } else {
                            0.0
                        }
                    }
                    // Mountain ranges; `montesSpiky` sharpens the crests.
                    _ => {
                        let f = v(sg::MONTES_FREQ).max(1.0) * 0.25;
                        if on(f) {
                            let m = ridged(dir * f + warp * 2.0, s ^ 0x55, oct(f, 8));
                            m.powf(1.0 + v(sg::MONTES_SPIKY)) * v(sg::MONTES_MAGN) * (0.3 + 0.7 * land) * 1.6 * fade(r / f)
                        } else {
                            0.0
                        }
                    }
                };
            }
        }
        // Terraces on a `terraceProb` share of the land.
        let tp = v(sg::TERRACE_PROB);
        if tp > 0.0 && fade(r / 14.0) > 0.0 {
            let mask = smoothstep(0.05, -0.05, gradient(dir * 4.0 + rnd, s ^ 0x7E3) * 2.0 - (tp * 2.0 - 1.0));
            let x = h * 60.0;
            let k = x.floor();
            let terr = (k + smoothstep(0.4, 0.6, x - k)) / 60.0;
            h += (terr - h) * mask * land * 0.6 * fade(r / 14.0);
        }
        // Cracks (icy worlds): narrow grooves.
        let cm = v(sg::CRACKS_MAGN);
        if cm > 0.0 && v(sg::CRACKS_OCTAVES) > 0.0 {
            let f = v(sg::CRACKS_FREQ).max(0.1) * 20.0;
            if fade(r / f * 0.02) > 0.0 {
                let n = gradient(dir * f + warp * 3.0, s ^ 0xC7A);
                h -= (1.0 - smoothstep(0.0, 0.02, n.abs())) * cm * 0.3 * fade(r / f * 0.02);
            }
        }
        // Equatorial ridge.
        let em = v(sg::EQRIDGE_MAGN);
        if em > 0.0 {
            let w = v(sg::EQRIDGE_WIDTH).max(1e-3);
            let modu = 1.0 + 0.2 * v(sg::EQRIDGE_MOD_MAGN) * gradient(dir * v(sg::EQRIDGE_MOD_FREQ), s ^ 0xE9);
            h += em * (-(dir.y / w).powi(2)).exp() * modu.max(0.0);
        }
        // Craters: `craterOctaves` size classes, the largest about R / craterFreq across.
        let cd = v(sg::CRATER_DENSITY);
        let co = v(sg::CRATER_OCTAVES);
        if cd > 0.0 && co >= 1.0 {
            let cell0 = (1.2 / v(sg::CRATER_FREQ).max(0.5)).min(0.35);
            let levels = Self::octaves_for(r * cell0 * 0.3, 2.0, min_feature_m, (co as u32).min(12));
            h += craters_from(dir, s ^ 0xC4A7, cd.min(1.0) * 0.6, levels, cell0) * v(sg::CRATER_MAGN) * 0.5;
        }
        // Sea level is the reference radius of ocean worlds; dry ones rise from it, as in SE
        // (`BumpOffset` 0 without an ocean).
        (h - if self.ocean_fraction > 0.0 { sea } else { 0.0 }) * self.bump_m
    }

    /// Small-scale relief added below the resolution of real data or of the
    /// procedural base, scaled to the patch size `scale_m`.
    pub fn detail_height_m(&self, dir: DVec3, scale_m: f64) -> f64 {
        if self.kind.is_giant() {
            return 0.0;
        }
        let freq = self.radius_m / scale_m.max(1.0);
        fbm(dir * freq, self.seed ^ 0xDE7A, 4, 2.0, 0.5) * scale_m * 0.08
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_heights() {
        let moon = ProcSurface::for_body("Test Moon", Some("Selena"), 1_737_000.0, 7.3e22, 250.0, false, false, false);
        assert_eq!(moon.kind, SurfaceKind::Cratered);
        let earth = ProcSurface::for_body("Test Earth", Some("Terra"), 6_378_000.0, 5.97e24, 288.0, true, true, true);
        assert_eq!(earth.kind, SurfaceKind::Rocky);
        assert!(earth.ocean_fraction > 0.5);
        let jup = ProcSurface::for_body("Test Jup", Some("Jupiter"), 71_492_000.0, 1.9e27, 110.0, true, false, false);
        assert_eq!(jup.height_m(DVec3::X), 0.0);
        // Ocean fraction roughly honoured.
        let mut below = 0;
        let n = 2000;
        for i in 0..n {
            let t = i as f64 * 2.399963;
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
            let r = (1.0 - z * z).sqrt();
            let d = DVec3::new(r * t.cos(), z, r * t.sin());
            let h = earth.height_m(d);
            assert!(h.abs() < earth.relief_m * 3.0);
            if h < 0.0 {
                below += 1;
            }
        }
        let frac = below as f64 / n as f64;
        assert!(frac > 0.3 && frac < 0.95, "{frac}");
    }

    #[test]
    fn se_relief_stays_within_the_bump_span() {
        use crate::surface_gen::{self as sg, Classes, GenInput};
        for surface in [1, 4, 5, 6] {
            let inp = GenInput {
                seeds: sg::catalog_seeds("Test", "Star"),
                obj_type: sg::obj::PLANET,
                classes: Classes { bulk: sg::bulk::TERRA, temp: 4, atmo: if surface == 5 { 0 } else { 3 }, volq: 1, size: 2, surface },
                radius_km: 2448.0,
                mass_earth: 0.0388,
                t_eq: 200.0,
                t_surf: 200.0,
                t_class: 200.0,
                pressure_atm: if surface == 5 { 0.0 } else { 1.0 },
                atm_scale_height_km: 8.0,
                atm_height_km: 80.0,
                ocean_level: (surface == 1).then_some(0.5),
                bump_height_km: 57.0,
                ..Default::default()
            };
            let g = sg::generate(&inp, &sg::given_from(None, None));
            let ps = ProcSurface::for_body("Test", Some("Terra"), 2_448_000.0, 2.3e23, 200.0, surface != 5, surface == 1, false).with_se(g, 57.0);
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for i in 0..1500 {
                let t = i as f64 * 2.399963;
                let z = 1.0 - 2.0 * (i as f64 + 0.5) / 1500.0;
                let r = (1.0 - z * z).sqrt();
                // About 2 km vertex spacing, as a close terrain patch.
                let h = ps.height_m_lod(DVec3::new(r * t.cos(), z, r * t.sin()), 2000.0);
                lo = lo.min(h);
                hi = hi.max(h);
            }
            eprintln!("class {surface}: {:.1} .. {:.1} km", lo / 1000.0, hi / 1000.0);
            assert!(hi - lo < ps.bump_m && lo > -ps.bump_m && hi < ps.bump_m, "{lo} {hi}");
        }
    }
}
