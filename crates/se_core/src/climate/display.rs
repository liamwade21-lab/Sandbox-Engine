//! The temperatures SE's info panel shows (FUN_14025a220, run as the panel draws): night, average
//! and day temperatures of a body for the current distance of each of its stars and the season
//! (the latitude where the star stands overhead). The panel prints the average.
//!
//! SE's temperature model (record +0x184c, set up by FUN_14025f1c0) runs per star: a rotating body
//! under thin or no air (FUN_14025be70), a giant or crushing atmosphere (FUN_14025b430) or a body
//! locked to its star (FUN_14025aef0) each give a night, day and average value. Those add as T^4
//! over the stars, then internal heat and greenhouse warming go on top (FUN_14025e390). Catalog
//! `MinSurfaceTemp` / `MaxSurfaceTemp` rescale the result (FUN_14025aa60).

use super::{fold_angle, spread_rotating, spread_thick, tau_factor, tau_peak, ThermalIn, SIGMA_SB};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

/// What SE's model reads besides [`ThermalIn`]: the catalog's `Climate` block and the orbit.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelExtra {
    /// `GlobalWindSpeed` (m/s).
    pub wind: Option<f32>,
    /// `MaxSurfaceTemp` / `MinSurfaceTemp` (K).
    pub max_temp: Option<f32>,
    pub min_temp: Option<f32>,
    /// Height (km) of the 0.2 atm level; 0 for thinner air (atmosphere +0x4c).
    pub h02_km: f32,
    /// Periapsis, mean and apoapsis distance from the star (pc), as FUN_140513830 stores them.
    pub dist_pc: [f32; 3],
    /// The body's own orbit is eccentric.
    pub eccentric: bool,
}

/// One star as the panel sees it.
#[derive(Clone, Copy, Debug)]
pub struct StarView {
    /// Bolometric luminosity (L_sun).
    pub lum: f32,
    /// Current distance to the body (pc).
    pub dist_pc: f32,
    /// Latitude on the body where the star stands overhead (rad).
    pub decl: f32,
    /// The star the body (or its planet) orbits.
    pub main: bool,
}

/// Night, average and day temperatures (K).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelTemps {
    pub night: f32,
    pub average: f32,
    pub day: f32,
}

/// SE's temperature model of a body.
#[derive(Clone, Copy, Debug)]
pub struct Model {
    m: ThermalIn,
    x: ModelExtra,
    /// Wind speed (m/s, +0x00), substellar temperature (+0x10), optical depth (+0x18), night to
    /// day ratio (+0x1c) and vertical mixing (percent, +0x20).
    wind: f32,
    s4: f32,
    tau: f32,
    r: f32,
    mix: f32,
}

/// Running (sum of T^4)^1/4 of night, day and average over the stars.
#[derive(Default)]
struct Acc([f32; 3]);

impl Acc {
    fn add(&mut self, k: usize, t: f32) {
        self.0[k] = ((self.0[k] as f64).powf(4.0) + (t as f64).powf(4.0)).powf(0.25) as f32;
    }
    fn add_all(&mut self, t: f32) {
        (0..3).for_each(|k| self.add(k, t));
    }
}

/// Fraction of a day the star is up at latitude `lat` when it stands over latitude `decl`
/// (FUN_14025de00).
fn day_fraction(lat: f32, decl: f32) -> f32 {
    let (tl, td) = (lat.tan(), decl.tan());
    let mut v = 1.0;
    if td * tl <= 1.0 {
        v = if -1.0 <= td * tl { ((-tl * td).acos() as f64 / PI) as f32 } else { 0.0 };
    }
    if lat.abs() == std::f32::consts::FRAC_PI_2 {
        if decl == 0.0 {
            return 0.5;
        }
        if lat * decl > 0.0 {
            v = 1.0;
        }
        if lat * decl < 0.0 {
            v = 0.0;
        }
    }
    v
}

impl Model {
    /// Set up the model (FUN_14025f1c0, night ratio FUN_14025e9b0).
    pub fn new(m: &ThermalIn, x: &ModelExtra) -> Model {
        let (p, rho) = (m.pressure_atm, m.density_kg_m3);
        let thick = m.thick();
        let f = if p >= 0.2 { 4.0 } else { 1.0 };
        let s4 = x.max_temp.unwrap_or_else(|| (((1.0 - m.bond_albedo) * m.flux * 1370.0) / (f * SIGMA_SB)).powf(0.25));
        let wind = x.wind.unwrap_or_else(|| {
            let mut w = if !thick {
                if p < 0.01 || rho == 0.0 { 0.0 } else { (1.0 / rho).powf(0.333_333_3) * 10.0 }
            } else if p == 0.0 || rho == 0.0 {
                0.0
            } else {
                (1.0 / rho).powf(0.25) * 100.0
            };
            if w.is_infinite() {
                w = 0.0;
            }
            let lim = if thick { 2000.0 } else { 200.0 };
            while w > lim {
                w /= 10.0;
            }
            let g = m.mass_earth / ((m.radius_km / 6371.0) * (m.radius_km / 6371.0)) * 9.806_65;
            let esc = ((g + g) * m.radius_km * 1000.0).sqrt() * 0.001;
            if esc * 1000.0 <= w {
                w = ((esc * 1000.0) as f64 * 0.75) as f32;
            }
            w
        });
        let tau = if !thick {
            if p >= 0.01 {
                let s3 = (s4 as f64).powf(3.0);
                let t = ((rho * 1000.0 * x.h02_km * wind) as f64 / ((s3 * 5.670_374_037e-8 + s3 * 5.670_374_037e-8) * PI * m.radius_km as f64)) as f32;
                if t.is_nan() { 0.1 } else { t.max(0.1).min(20.0) }
            } else {
                0.1
            }
        } else {
            let t = ((wind * 0.001).exp() - 1.0) * 1.788_51;
            if t >= 0.1 { t } else { 0.1 }
        };
        let mut mix = 20.0f32;
        if p >= 0.01 {
            mix = (10.0 - 2.0 * rho.ln()).abs();
            if thick {
                mix = 5.0;
            }
        }
        if m.locked_to_star {
            mix = 66.666_67;
        }
        let mut model = Model { m: *m, x: *x, wind, s4, tau, r: 0.0, mix };
        model.r = model.night_ratio();
        model
    }

    fn thick(&self) -> bool {
        self.m.thick()
    }

    /// Internal heat and greenhouse on top of a radiative T^4 (FUN_14025e390).
    fn surface(&self, t4: f32) -> f32 {
        self.m.surface(t4)
    }

    /// The radiative part of a temperature (FUN_14025e2e0), 20 K when heat alone exceeds it.
    fn strip(&self, t: f32) -> f32 {
        let t = (t - self.m.greenhouse_k).max(0.0);
        let x = ((t as f64).powf(4.0) - (self.m.heat * 1_048_576.0) as f64) as f32;
        if x < 0.0 { 20.0 } else { (x as f64).powf(0.25) as f32 }
    }

    /// Night to day ratio (FUN_14025e9b0), from the catalog's extremes where it gives them.
    fn night_ratio(&self) -> f32 {
        let (lo, hi) = (self.x.min_temp, self.x.max_temp);
        let p = self.m.pressure_atm;
        let (night, day) = match (lo, hi) {
            (Some(lo), Some(hi)) => (lo, hi),
            _ if self.thick() => match (lo, hi) {
                (Some(lo), None) => {
                    let n = self.strip(lo);
                    (n, n / 0.75)
                }
                _ => {
                    let d = hi.map_or(self.s4, |t| self.strip(t));
                    (d * 0.75, d)
                }
            },
            _ if p <= 0.001 => (lo.map_or_else(|| self.surface(0.0), |t| self.strip(t)), hi.map_or(self.s4, |t| self.strip(t))),
            _ => {
                let mut dt = ((25.0 - ((p as f64) * 1.01325).log10() * 11.0) * (2.0 / self.m.rotation_days)) as f32;
                if self.s4 < dt {
                    dt = self.s4 * 0.99;
                }
                let dt = if dt >= 5.0 { dt } else { 5.0 };
                let (night, day) = match (lo, hi) {
                    (Some(lo), _) => {
                        let n = self.strip(lo);
                        (n, dt + dt + n)
                    }
                    (None, Some(hi)) => {
                        let d = self.strip(hi);
                        (d - (dt + dt), d)
                    }
                    _ => (self.s4 - dt, dt + self.s4),
                };
                let floor = self.surface(0.0);
                (if floor <= night { night } else { floor }, day)
            }
        };
        let ratio = (night.max(20.0) / day.max(20.0)).abs() / (1.0 - self.mix / 100.0);
        if ratio > 1.0 { 0.99 } else { ratio }
    }

    /// |cos| of SE's night-side latitude, blended with the night ratio.
    fn night_side(&self) -> f32 {
        let fold = fold_angle(self.m.obliquity_rad as f32).abs();
        let a = ((fold as f64 + FRAC_PI_2) as f32 as f64 / ((fold + fold) as f64 / PI + 1.0)).abs() as f32;
        (a.cos() * (1.0 - self.r) + self.r).abs()
    }

    /// Radiative temperature from one star at `d_pc` with heat spread factor `f` (FUN_14025e400).
    /// For the main star the catalog extremes stand in, scaled as 1/sqrt(d) between periapsis
    /// and apoapsis.
    fn radiative(&self, f: f32, main: bool, lum: f32, d_pc: f32) -> f32 {
        let (hi, lo) = (self.x.max_temp, self.x.min_temp);
        let [peri, _, apo] = self.x.dist_pc;
        let h1 = 1.0 - self.mix / 100.0;
        let t = if main && (hi.is_some() || lo.is_some()) {
            let fin = |v: f32, or: f32| if v.is_finite() { v } else { or };
            match (hi, lo) {
                (Some(hi), None) => (peri.sqrt() * fin(self.strip(hi), hi)) / d_pc.sqrt(),
                (Some(hi), Some(lo)) => {
                    let (a, b) = (fin(self.strip(hi), hi), fin(self.strip(lo), 20.0));
                    let (small, big) = if a < b { (a, b) } else { (b, a) };
                    if self.x.eccentric {
                        let x = small / (self.night_side() * h1);
                        let (sa, sp) = (apo.sqrt(), peri.sqrt());
                        let base = (sp * big - x * sa) / (sp - sa);
                        ((x - base) * sa) / d_pc.sqrt() + base
                    } else {
                        big
                    }
                }
                _ => {
                    let lo = fin(self.strip(lo.unwrap_or(20.0)), 20.0);
                    if self.m.locked_to_star { lo * 3.0 } else { ((lo / (self.night_side() * h1)) * apo.sqrt()) / d_pc.sqrt() }
                }
            }
        } else {
            let d_m = ((d_pc * 1000.0) as f64 / 3.240_779_289_444_365e-14) as f32 as f64;
            let flux = ((lum * 3.826e26) as f64 / (d_m * 12.566_370_614_359_172 * d_m)) as f32;
            let mut t = ((flux * (1.0 - self.m.bond_albedo)) / (f * SIGMA_SB)).powf(0.25);
            if f == 4.0 {
                t /= ((0.994_837_701_321f64 as f32).cos() * (1.0 - self.r) + self.r).abs();
            }
            t
        };
        if t == 0.0 { 20.0 } else { t }
    }

    /// Temperature after cooling from `t` at angle `start` (rad) to `end` through the night
    /// (FUN_14025d830): T^-3 grows by 6 pi / tau per radian.
    fn cooled(&self, end: f64, start: f32, t: f32) -> f32 {
        let span = (end - (start as f64 - FRAC_PI_2)) as f32 as f64 - FRAC_PI_2;
        ((t.powf(-3.0) as f64) + span * (18.849_555_921_538_759 / self.tau as f64)).powf(-0.333_333_343_267_441) as f32
    }

    /// Locked to the star (FUN_14025cc40): day and night sides with night at `q` of the day.
    fn spread_locked(&self, t: f32, q: f32) -> f32 {
        let d3 = (self.wind * 0.000_15 + self.wind * 0.000_15) as f64;
        let d1 = q as f64 * PI;
        let a = (d3 + PI).sin() + TAU;
        let b = (d3 - PI).sin();
        let s = ((a - b) * ((1.0 - q) * 0.25) as f64 + d1) as f32 + d1 as f32;
        (s as f64 / TAU) as f32 * t
    }

    /// Rotating body under thin or no air (FUN_14025be70).
    fn rotating(&self, s: &StarView, acc: &mut Acc) {
        let p = self.m.pressure_atm;
        let t = self.radiative(if p >= 0.2 { 4.0 } else { 1.0 }, s.main, s.lum, s.dist_pc);
        if s.lum == 0.0 {
            acc.add_all(if s.main { 20.0 } else { 0.0 });
            return;
        }
        let tau = self.tau;
        let x29 = (tau as f64 / 25.132_741_228_718_345) as f32;
        let x37 = std::f32::consts::PI - x29;
        let peak = tau_peak(tau);
        let k = (0.0 - peak) / (x37 * x37);
        let curve = |a: f32| k * (a - x29) * (a - x29) + peak;
        let r = self.r;
        let fold = fold_angle(self.m.obliquity_rad as f32).abs();
        let d4 = (fold + fold) as f64 / PI + 1.0;
        let ad = s.decl.abs();
        // The pole turned away: dark through a season, half lit at an equinox.
        let f13 = day_fraction(-std::f32::consts::FRAC_PI_2, ad);
        let f14 = (f13 as f64 * PI) as f32;
        let mut pole = curve(f14).max(0.0);
        if f13 == 0.0 {
            pole = self.cooled(6.283_185_482_025_146_5, f14, pole);
        } else if f13 < 1.0 {
            pole = self.cooled((TAU - f13 as f64 * PI) as f32 as f64, f14, pole);
        }
        let c26 = ((((ad as f64 + FRAC_PI_2) as f32 as f64) / d4).abs() as f32).cos();
        let mut lat_night = (c26 * (1.0 - r) + r).abs();
        if lat_night < r || 1.0 < lat_night {
            lat_night = r;
        }
        let x0 = (0.0 - x29) * k * (0.0 - x29) + peak;
        let floor = ((x0.powf(-3.0) as f64) + (18.849_555_921_538_759 / tau as f64) * 6.283_185_644_960_403).powf(-0.333_333_343_267_441) as f32;
        let h = self.mix / 100.0;
        let span = 1.0 - floor;
        let keep = 1.0 - h;
        let maxf = ((peak - floor) / span) * h + keep;
        let mid = (1.0 - maxf) * 0.5 + maxf;
        let f9 = ((1.0 - mid) * c26 + mid).abs();
        let x = (day_fraction(s.decl, s.decl) as f64 * PI) as f32 - x29;
        let set = (((x * k * x + peak) - floor) / span) * h + keep;
        let avg = spread_rotating(p, self.m.obliquity_rad, r, h, t);
        let mut corr = 1.0;
        if (maxf + set) * 0.5 < avg / t && (ad as f64) < FRAC_PI_4 && s.main {
            corr = (avg / t) / set;
        }
        acc.add(0, (((pole - floor) / span) * h + keep) * lat_night * t * corr * (2.0 - f9));
        acc.add(1, maxf * t * corr);
        acc.add(2, avg);
    }

    /// Giant or crushing atmosphere (FUN_14025b430).
    fn enveloped(&self, s: &StarView, acc: &mut Acc) {
        let t4 = self.radiative(4.0, s.main, s.lum, s.dist_pc);
        if s.lum == 0.0 {
            acc.add_all(if s.main { 20.0 } else { 0.0 });
            return;
        }
        let tau = self.tau;
        let x25 = (tau as f64 / 25.132_741_228_718_345) as f32;
        let x22 = std::f32::consts::PI - x25;
        let c = tau_factor(tau);
        let k = (0.0 - c) / (x22 * x22);
        let r = self.r;
        let fold = fold_angle(self.m.obliquity_rad as f32).abs();
        let d6 = (fold + fold) as f64 / PI + 1.0;
        let ad = s.decl.abs();
        let f15 = day_fraction(-std::f32::consts::FRAC_PI_2, ad);
        let f16 = (f15 as f64 * PI) as f32;
        let mut pole = (f16 - x25) * k * (f16 - x25) + c;
        if f15 == 0.0 {
            pole = self.cooled(6.283_185_482_025_146_5, f16, pole);
        } else if f15 < 1.0 {
            pole = self.cooled((TAU - f15 as f64 * PI) as f32 as f64, f16, pole);
        }
        let d2 = ad as f64 + FRAC_PI_2;
        let mut lat_night = ((((d2 as f32) as f64 / d6).abs() as f32).cos() * (1.0 - r) + r).abs();
        if lat_night < r || 1.0 < lat_night {
            lat_night = r;
        }
        let x0 = k * (0.0 - x25) * (0.0 - x25) + c;
        let floor = ((x0.powf(-3.0) as f64) + (18.849_555_921_538_759 / tau as f64) * 6.283_185_644_960_403).powf(-0.333_333_343_267_441) as f32;
        let h = self.mix / 100.0;
        let span = 1.0 - floor;
        let keep = 1.0 - h;
        let peak = ((c - floor) / span) * h + keep;
        // FUN_14025d170 at the night-side latitude.
        let lift = if 0.0 < ad { peak + 1.0 } else { 1.0 };
        let a = ((d2 as f32) as f64 / ((ad + ad) as f64 / PI + 1.0)).abs() as f32;
        let side = (a.sin() * (1.0 - peak) * lift + peak).abs();
        acc.add(0, side * (((pole - floor) / span) * h + keep) * lat_night * t4);
        acc.add(1, peak.abs() * t4);
        acc.add(2, spread_thick(self.m.obliquity_rad, self.wind, r, h, t4));
    }

    /// Locked to its star (FUN_14025aef0).
    fn locked(&self, s: &StarView, acc: &mut Acc) {
        let t2 = self.radiative(2.0, s.main, s.lum, s.dist_pc);
        if t2 <= 20.0 {
            acc.add_all(if s.main { 20.0 } else { 0.0 });
            return;
        }
        let c = tau_factor(self.tau);
        let lo = self.x.min_temp.unwrap_or(t2 / 3.0);
        let q = (lo / t2) * 0.1 + lo / t2;
        let ct = c * t2;
        let f19 = (1.0 - c) + (1.0 - c);
        let f22 = 1.0 - f19;
        let day = (0f32.cos() * 0.1 + 0.9).abs() * ct * (2.0 - (0f32.cos() * f19 + f22));
        let n = ((1.570_796_370_506_286_6f64 as f32).cos() * 0.1 + 0.9).abs();
        let night = n * ct * q * (2.0 - (FRAC_PI_2.cos() * f19 as f64 + f22 as f64)) as f32;
        acc.add(0, night);
        acc.add(1, day);
        acc.add(2, self.spread_locked(ct, q));
    }

    /// Night, average and day temperatures for `stars`.
    pub fn panel(&self, stars: &[StarView]) -> PanelTemps {
        if stars.is_empty() {
            let t = self.surface((self.x.max_temp.unwrap_or(20.0) as f64).powf(4.0) as f32);
            return PanelTemps { night: t, average: t, day: t };
        }
        let mut acc = Acc::default();
        for s in stars {
            if self.m.locked_to_star {
                self.locked(s, &mut acc);
            } else if !self.thick() {
                self.rotating(s, &mut acc);
            } else {
                self.enveloped(s, &mut acc);
            }
        }
        let fin = |t: f32| self.surface((t as f64).powf(4.0) as f32);
        let mut out = PanelTemps { night: fin(acc.0[0]), average: fin(acc.0[2]), day: fin(acc.0[1]) };
        if self.x.max_temp.is_some() || self.x.min_temp.is_some() {
            let d = stars.iter().find(|s| s.main).map_or(stars[0].dist_pc, |s| s.dist_pc);
            self.rescale(&mut out, d);
        }
        if out.day < out.average {
            out.average = out.day;
            out.night = out.day;
        }
        if out.average < out.night {
            out.night = out.average;
            out.average = (out.average + out.day) * 0.5;
        }
        out
    }

    /// Map the model's night .. day range onto the catalog's extremes at distance `d_pc` from the
    /// main star (FUN_14025aa60).
    fn rescale(&self, o: &mut PanelTemps, d_pc: f32) {
        let (hi, lo) = (self.x.max_temp, self.x.min_temp);
        let [peri, _, apo] = self.x.dist_pc;
        let sd = d_pc.sqrt();
        let night_lo = |lo: f32| {
            let c = self.night_side();
            let h1 = 1.0 - self.mix / 100.0;
            ((apo.sqrt() * (lo / (c * h1))) / sd) * c * h1
        };
        match (hi, lo) {
            (Some(hi), Some(lo)) => {
                let top = ((peri.sqrt() * hi) / sd).min(hi);
                let mut bottom = lo;
                if !self.m.locked_to_star {
                    let v = night_lo(lo);
                    if lo <= v {
                        bottom = v;
                    }
                }
                let (n, dmax) = (o.night, o.day);
                o.day = dmax * (top / dmax);
                o.average = ((o.average - n) / (dmax - n)) * (top - bottom) + bottom;
                o.night = (bottom / n) * n;
            }
            _ => {
                let k = match (hi, lo) {
                    (Some(hi), None) => ((peri.sqrt() * hi) / sd) / o.day,
                    (None, Some(lo)) if !self.m.locked_to_star => night_lo(lo) / o.night,
                    (None, Some(lo)) => lo / o.night,
                    _ => return,
                };
                o.day *= k;
                o.average *= k;
                o.night *= k;
            }
        }
        if let Some(lo) = lo {
            o.night = if lo <= o.night { o.night } else { lo };
        }
        if let Some(hi) = hi {
            o.day = if o.day <= hi { o.day } else { hi };
        }
    }
}

/// SE's temperature model for catalog body `i` with its system's `light`: from the completion
/// when SE generated the atmosphere, else from the catalog as for the class word.
pub fn catalog_model(cat: &crate::catalog::Catalog, i: usize, light: &super::StarLight, completion: Option<&super::Completion>) -> Model {
    use crate::catalog::ObjectRef;
    let b = &cat.bodies[i];
    let blk = &b.block;
    let atm = blk.sub("Atmosphere").filter(|_| !blk.bool("NoAtmosphere").unwrap_or(false));
    let g = |m: f32, r: f32| m / ((r / 6371.0) * (r / 6371.0)) * 9.806_65;
    let (thermal, h02_km) = match completion.and_then(|c| c.thermal.map(|t| (c, t))) {
        Some((c, t)) => {
            let p = c.pressure_atm;
            (t, if p >= 0.2 { ((1.0 - (0.2 / p).powf(0.25)) * c.scale_height_km) / 0.25 } else { 0.0 })
        }
        None => {
            let t_surf = super::equilibrium_temperature(light.flux_mean, b) as f32;
            let greenhouse = atm.and_then(|a| a.f64("Greenhouse")).unwrap_or(0.0);
            let p = atm.and_then(|a| a.f64("Pressure")).unwrap_or(if atm.is_some() { 1.0 } else { 0.0 }) as f32;
            let bulk = crate::surface_gen::catalog_bulk(b);
            let t = crate::surface_gen::catalog_thermal(cat, i, light.flux_mean, greenhouse, p, t_surf, bulk);
            let h = (t_surf * super::R_GAS) / (29.0 * g(t.mass_earth, t.radius_km));
            (t, if p >= 0.2 { ((1.0 - (0.2 / p).powf(0.25)) * h) / 0.25 } else { 0.0 })
        }
    };
    // Periapsis, mean and apoapsis distance (FUN_140513830): of the body's own orbit around a
    // star, else its parent's orbit widened by its own.
    let pc = |o: &crate::orbit::KeplerOrbit| {
        let (a, e) = (o.semi_major_axis / crate::units::METERS_PER_PC, o.eccentricity);
        [a * (1.0 - e), a * (e * e * 0.5 + 1.0), a * (1.0 + e)]
    };
    let own = b.orbit.as_ref().map(pc);
    let parent = match cat.find(&b.parent) {
        Some(ObjectRef::Star(s)) if !cat.stars[s as usize].barycenter => None,
        Some(ObjectRef::Star(s)) => cat.stars[s as usize].orbit.as_ref().map(pc),
        Some(ObjectRef::Body(j)) => cat.bodies[j as usize].orbit.as_ref().map(pc),
        _ => None,
    };
    let d = match (own, parent) {
        (Some(s), Some(p)) => [p[0] - s[2], p[1] + s[1], s[2] + p[2]],
        (Some(s), None) => s,
        (None, Some(p)) => p,
        (None, None) => [0.0; 3],
    };
    let climate = blk.sub("Climate");
    let get = |k: &str| climate.and_then(|c| c.f64(k)).map(|v| v as f32);
    let x = ModelExtra {
        wind: get("GlobalWindSpeed"),
        max_temp: get("MaxSurfaceTemp"),
        min_temp: get("MinSurfaceTemp"),
        h02_km,
        dist_pc: d.map(|v| v as f32),
        eccentric: b.orbit.as_ref().is_some_and(|o| o.eccentricity != 0.0),
    };
    Model::new(&thermal, &x)
}

/// The star body `i` goes around (through the planet it circles and any barycentre in between,
/// such as Earth-Moon) and the object whose orbit around that star makes the body's year.
fn year_orbit(cat: &crate::catalog::Catalog, i: usize) -> Option<(crate::catalog::ObjectRef, u32)> {
    use crate::catalog::ObjectRef;
    let mut cur = ObjectRef::Body(i as u32);
    for _ in 0..16 {
        let parent = match cur {
            ObjectRef::Body(j) => cat.bodies[j as usize].parent.as_str(),
            ObjectRef::Star(s) => cat.stars[s as usize].parent.as_deref()?,
            _ => return None,
        };
        match cat.find(parent)? {
            ObjectRef::Star(s) if !cat.stars[s as usize].barycenter => return Some((cur, s)),
            p @ (ObjectRef::Star(_) | ObjectRef::Body(_)) => cur = p,
            _ => return None,
        }
    }
    None
}

/// The stars of body `i`'s system as the panel sees them at `jd`: luminosity, distance and the
/// latitude under each star (from the port's spin axis). The main star is the first star up the
/// parent chain; black holes are left out.
pub fn catalog_view(eph: &crate::ephemeris::Ephemeris, cat: &crate::catalog::Catalog, i: usize, jd: f64) -> Vec<StarView> {
    use crate::catalog::ObjectRef;
    let obj = ObjectRef::Body(i as u32);
    let Some(root) = cat.system_root(obj) else { return Vec::new() };
    let main = year_orbit(cat, i).map(|(_, s)| s);
    let pos = eph.position(obj, jd);
    let pole = eph.orientation(obj, jd) * glam::DVec3::Y;
    let mut stars = cat.system_members(&cat.stars[root as usize].names[0]);
    stars.push(ObjectRef::Star(root));
    stars.dedup();
    stars
        .into_iter()
        .filter_map(|s| {
            let ObjectRef::Star(k) = s else { return None };
            let st = &cat.stars[k as usize];
            if st.barycenter || matches!(st.class.kind, crate::star::StarKind::BlackHole | crate::star::StarKind::Planemo) {
                return None;
            }
            let d = eph.position(s, jd).delta_m(pos);
            Some(StarView {
                lum: st.lum_bol() as f32,
                dist_pc: (d.length() / crate::units::METERS_PER_PC) as f32,
                decl: pole.dot(d.normalize_or_zero()).clamp(-1.0, 1.0).asin() as f32,
                main: main == Some(k),
            })
        })
        .collect()
}

/// SE's day lines of the panel: "Solar day", "Daylight hours at equator/current latitude" and
/// "Annual fraction of polar day at current latitude".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayLines {
    /// Solar day (days): the star's return to the meridian.
    pub solar_day: f64,
    /// Share of the solar day in daylight at the equator and at the observer's latitude.
    pub daylight_equator: f32,
    pub daylight_here: f32,
    /// Percent of the year in polar day (or night) at the observer's latitude.
    pub polar_day_percent: f32,
}

/// Share of a day the star is up at latitude `lat` with the star overhead at `decl` (FUN_14025de00).
pub fn daylight_share(lat: f32, decl: f32) -> f32 {
    let p = lat.tan() * decl.tan();
    let share = if p > 1.0 {
        1.0
    } else if p < -1.0 {
        0.0
    } else {
        (-p).acos() / std::f32::consts::PI
    };
    if lat.abs() == std::f32::consts::FRAC_PI_2 && decl != 0.0 {
        return if lat * decl > 0.0 { 1.0 } else { 0.0 };
    }
    share
}

/// Percent of the year in polar day at `lat` for a spin axis `tilt` from the orbit's pole
/// (FUN_14025db80): the star's latitude runs `tilt sin t` over 6283 steps of a year.
pub fn polar_day_percent(lat: f32, tilt: f32) -> f32 {
    let tilt = if tilt > FRAC_PI_2 as f32 { (PI - tilt as f64) as f32 } else { tilt };
    let tilt = if tilt < -FRAC_PI_2 as f32 { (tilt as f64 + PI) as f32 } else { tilt };
    let t = lat.tan();
    let n = (0..6283).filter(|&k| (1.0 - ((k as f32 * 0.001).sin() * tilt).tan() * t).clamp(0.0, 2.0) >= 2.0).count();
    (n as f64 * 100.0 / 6283.0) as f32
}

/// The day lines of catalog body `i` at `jd` for an observer at latitude `lat` (rad), or `None`
/// for a body without a rotation, without a star to orbit, or turning once per orbit
/// (FUN_140515cf0). The year is the orbit around the star of the body or of the planet it circles.
pub fn day_lines(eph: &crate::ephemeris::Ephemeris, cat: &crate::catalog::Catalog, i: usize, jd: f64, lat: f64) -> Option<DayLines> {
    use crate::catalog::ObjectRef;
    let b = &cat.bodies[i];
    // Without a rotation of its own a moon turns once per orbit (SE locks catalog moons).
    let p_rot = b.rotation.period_seconds().or_else(|| b.orbit.as_ref().map(|o| o.period).filter(|p| *p > 0.0))? / 86_400.0;
    let (anc, s) = year_orbit(cat, i)?;
    let star = ObjectRef::Star(s);
    let o = match anc {
        ObjectRef::Body(j) => cat.bodies[j as usize].orbit.as_ref()?,
        ObjectRef::Star(k) => cat.stars[k as usize].orbit.as_ref()?,
        _ => return None,
    };
    let p_orb = o.period / 86_400.0;
    if p_orb <= 0.0 || (p_orb - p_rot).abs() < p_orb * 1e-6 {
        return None;
    }
    // Spin and orbit in the same sense (SE: obliquity and the orbit's inclination both under or
    // both over 90 degrees) take the difference of the rates, else their sum.
    let obj = ObjectRef::Body(i as u32);
    let pole = eph.orientation(obj, jd) * glam::DVec3::Y;
    let r1 = eph.position(anc, jd).delta_m(eph.position(star, jd));
    let t2 = jd + p_orb / 8.0;
    let r2 = eph.position(anc, t2).delta_m(eph.position(star, t2));
    let normal = r1.cross(r2).normalize_or_zero();
    let cos_tilt = pole.dot(normal).clamp(-1.0, 1.0);
    // A negative rate (IAU's Venus) turns the body backwards about its north pole.
    let backwards = match &b.rotation {
        crate::rotation::RotationModel::Iau(r) => r.rotation_rate < 0.0,
        crate::rotation::RotationModel::Simple(r) => r.period_hours < 0.0,
    };
    let denom = if (cos_tilt >= 0.0) != backwards { p_orb - p_rot } else { p_orb + p_rot };
    let solar_day = ((p_rot * p_orb / denom).abs() / 1e-6 + 0.5).floor() * 1e-6;
    let decl = catalog_view(eph, cat, i, jd).iter().find(|s| s.main).map_or(0.0, |s| s.decl);
    Some(DayLines {
        solar_day,
        daylight_equator: daylight_share(0.0, decl),
        daylight_here: daylight_share(lat as f32, decl),
        polar_day_percent: polar_day_percent(lat as f32, cos_tilt.acos() as f32),
    })
}

#[cfg(test)]
mod day_tests {
    use super::*;

    #[test]
    fn daylight_and_polar_day() {
        // Equinox: half a day everywhere but the poles; solstice at 60 N with 23.44 deg tilt.
        assert!((daylight_share(1.0, 0.0) - 0.5).abs() < 1e-6);
        let s = daylight_share(60f32.to_radians(), 23.44f32.to_radians());
        assert!((s * 24.0 - 18.5).abs() < 0.2, "{}", s * 24.0);
        assert_eq!(daylight_share(80f32.to_radians(), 23.44f32.to_radians()), 1.0);
        // Earth: nothing at 60 N; at 80 N while the sun stands over 10 deg N or S
        // (polar day or night), (pi - 2 asin(10 / 23.44)) / 2 pi of the year.
        assert_eq!(polar_day_percent(60f32.to_radians(), 23.44f32.to_radians()), 0.0);
        let p80 = polar_day_percent(80f32.to_radians(), 23.44f32.to_radians());
        assert!((p80 - 36.0).abs() < 0.5, "{p80}");
    }
}
