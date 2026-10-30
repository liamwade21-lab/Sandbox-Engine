//! Spacecraft aerodynamics, as decoded from the executable.
//!
//! A design picks one of SE's three aerodynamic models (`ReadSchemeScript`, FUN_1405308b0):
//! `Aerodynamics true` selects "shuttle", `Aerodynamics false` "arrow", and `AeroModel "none" |
//! "shuttle" | "arrow"` overrides both; without either key the ship has model "none". The models
//! are built once by FUN_1401a3dd0: "shuttle" and "arrow" share one table of eleven coefficients
//! over 91 angles of attack, evaluated as a cubic spline (FUN_140026010 / FUN_140025ba0).
//!
//! * "none" (FUN_14051d700): drag only, `a = 5 q (L/10)^2 / m` against the air velocity.
//! * "shuttle" / "arrow" (FUN_14051cde0): lift, drag and side force with a transonic drag rise,
//!   and pitch, yaw and roll moments. The constants are those of the Aerosonde UAV of Beard and
//!   McLain's "Small Unmanned Aircraft" (S = 0.55 m^2, b = 2.8956 m, c = 0.18994 m), scaled by 100,
//!   and do not depend on the ship's size; only the mass does. "arrow" uses the flat-plate lift
//!   `1.2 sin 2a`, "shuttle" the table's lift blended into `1.2 sin 2a + 0.01` from Mach 0.9 to 1.8.
//!
//! Frames: the ship frame is SE's (x right, y up, z back, the nose along -z). The angular velocity
//! `w_se` in [`AeroInput`] uses the executable's convention, the negated physical rate about each
//! ship axis (its quaternion update is `dq * q` with `q` mapping world to ship).

use std::sync::OnceLock;

use glam::DVec3;

/// SE's aerodynamic model of a design (`seSpacecraft + 0x154`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AeroModel {
    #[default]
    None,
    Shuttle,
    Arrow,
}

impl AeroModel {
    /// The model `ReadSchemeScript` picks from the `Aerodynamics` and `AeroModel` keys
    /// (case-insensitive names; an unknown name falls back to "none" with a warning in SE).
    pub fn from_keys(aerodynamics: Option<bool>, aero_model: Option<&str>) -> AeroModel {
        let mut m = match aerodynamics {
            Some(true) => AeroModel::Shuttle,
            Some(false) => AeroModel::Arrow,
            None => AeroModel::None,
        };
        if let Some(name) = aero_model {
            m = match name.to_ascii_lowercase().as_str() {
                "shuttle" => AeroModel::Shuttle,
                "arrow" => AeroModel::Arrow,
                _ => AeroModel::None,
            };
        }
        m
    }

    pub fn name(self) -> &'static str {
        match self {
            AeroModel::None => "none",
            AeroModel::Shuttle => "shuttle",
            AeroModel::Arrow => "arrow",
        }
    }

    /// The model's flag (DAT_1408502a0): set for "shuttle", whose designs get control surfaces
    /// (`seSpacecraft + 0xac` bit 0x100000).
    pub fn has_control_surfaces(self) -> bool {
        self == AeroModel::Shuttle
    }
}

/// Reference area factor of the dynamic pressure (DAT_1406d49a8, Aerosonde S in m^2).
pub const AERO_AREA: f64 = 0.55;
/// Mean chord and span (m) of the rate terms (DAT_1406d4888, DAT_1406d4ce8).
pub const AERO_CHORD: f64 = 0.189941;
pub const AERO_SPAN: f64 = 2.8956;
/// Coefficient scale (DAT_1406d5100).
pub const AERO_SCALE: f64 = 100.0;
/// Largest force per unit coefficient factor, `min(0.55 q, 1e5)` (DAT_1406d5448).
pub const AERO_FORCE_CAP: f64 = 1e5;
/// Moments of inertia the executable gives every ship (ctor FUN_140524610; `ReadSchemeScript`
/// scales them by mass / 18000 using the default mass, so they never change).
pub const AERO_INERTIA: [f64; 3] = [800.0, 1755.0, 130.0];
/// Limit of each aerodynamic angular acceleration (rad/s^2, DAT_1406d47e8).
pub const AERO_ANG_ACCEL_MAX: f64 = 0.1;
/// Aerodynamic acceleration (m/s^2) above which SE destroys the ship unless `God` is set
/// (squared limit 250000, DAT_1406d5480).
pub const AERO_DESTROY_ACCEL: f64 = 500.0;
/// Dynamic pressure (Pa) from which SE flags the flight as high-pressure (DAT_1406d53b8).
pub const AERO_HIGH_Q: f64 = 10_000.0;
/// Default control surface limits and rates (ctor: 20 degrees, 20 degrees/s).
pub const CONTROL_LIMIT_RAD: f64 = 0.349_065_850_398_865_9;

include!("aero_table.rs");

/// Natural cubic spline over [`AERO_ALPHA_DEG`] of every column of [`AERO_TABLE`].
struct Spline {
    /// Second derivatives at the nodes.
    m: [[f64; 11]; 91],
}

fn spline() -> &'static Spline {
    static S: OnceLock<Spline> = OnceLock::new();
    S.get_or_init(|| {
        let x = &AERO_ALPHA_DEG;
        let n = x.len();
        let mut m = [[0.0; 11]; 91];
        for c in 0..11 {
            // Tridiagonal solve (Thomas) with natural end conditions m0 = mn = 0.
            let mut cp = vec![0.0; n];
            let mut dp = vec![0.0; n];
            for i in 1..n - 1 {
                let h0 = x[i] - x[i - 1];
                let h1 = x[i + 1] - x[i];
                let a = h0 / 6.0;
                let b = (h0 + h1) / 3.0;
                let cc = h1 / 6.0;
                let d = (AERO_TABLE[i + 1][c] - AERO_TABLE[i][c]) / h1 - (AERO_TABLE[i][c] - AERO_TABLE[i - 1][c]) / h0;
                let den = b - a * cp[i - 1];
                cp[i] = cc / den;
                dp[i] = (d - a * dp[i - 1]) / den;
            }
            for i in (1..n - 1).rev() {
                m[i][c] = dp[i] - cp[i] * m[i + 1][c];
            }
        }
        Spline { m }
    })
}

/// The eleven table coefficients at angle of attack `alpha_deg` (clamped to the table's range).
pub fn coefficients(alpha_deg: f64) -> [f64; 11] {
    let x = &AERO_ALPHA_DEG;
    let n = x.len();
    if !alpha_deg.is_finite() || alpha_deg <= x[0] {
        return AERO_TABLE[0];
    }
    if alpha_deg >= x[n - 1] {
        return AERO_TABLE[n - 1];
    }
    let i = x.partition_point(|v| *v <= alpha_deg).clamp(1, n - 1) - 1;
    let h = x[i + 1] - x[i];
    let t = (alpha_deg - x[i]) / h;
    let s = spline();
    let mut out = [0.0; 11];
    for c in 0..11 {
        let y0 = AERO_TABLE[i][c];
        let y1 = AERO_TABLE[i + 1][c];
        let (m0, m1) = (s.m[i][c], s.m[i + 1][c]);
        out[c] = (1.0 - t) * y0 + t * y1 + h * h / 6.0 * (((1.0 - t).powi(3) - (1.0 - t)) * m0 + (t.powi(3) - t) * m1);
    }
    out
}

/// Inputs of one aerodynamic step.
#[derive(Debug, Clone, Copy)]
pub struct AeroInput {
    pub model: AeroModel,
    /// `Length` (m) of the design (model "none" only).
    pub length_m: f64,
    pub mass_kg: f64,
    /// Ship velocity relative to the air, in the ship frame (m/s).
    pub v_air: DVec3,
    /// Angular velocity, the executable's convention (see the module docs), rad/s.
    pub w_se: DVec3,
    /// Air density (kg/m^3) and speed of sound (m/s, 0 when unknown).
    pub density: f64,
    pub sound_speed: f64,
    /// Control surface deflections (rad): elevator, rudder, ailerons.
    pub controls: DVec3,
}

/// Result of one aerodynamic step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AeroOutput {
    /// Aerodynamic acceleration in the ship frame (m/s^2).
    pub accel: DVec3,
    /// Angular acceleration in the executable's convention (rad/s^2), each within
    /// [`AERO_ANG_ACCEL_MAX`]; the physical one is its negation.
    pub ang_accel_se: DVec3,
    /// Angle of attack and sideslip (rad), Mach number and dynamic pressure (Pa).
    pub alpha: f64,
    pub beta: f64,
    pub mach: f64,
    pub q: f64,
}

/// Angle of attack and sideslip of an air velocity in the ship frame (FUN_14051cde0):
/// `-atan2(vy, -vz)` and `-atan2(vx, -vz)`.
pub fn attack_angles(v_air: DVec3) -> (f64, f64) {
    (-(v_air.y.atan2(-v_air.z)), -(v_air.x.atan2(-v_air.z)))
}

/// One aerodynamic step (FUN_14051d700 for model "none", FUN_14051cde0 otherwise).
pub fn aero_step(i: &AeroInput) -> AeroOutput {
    let v = i.v_air.length();
    let q = 0.5 * i.density * v * v;
    let mach = if i.sound_speed > 0.0 { v / i.sound_speed } else { 0.0 };
    let (alpha, beta) = attack_angles(i.v_air);
    let mut out = AeroOutput { alpha, beta, mach, q, ..Default::default() };
    if q <= 0.0 || v <= 0.0 || i.mass_kg <= 0.0 {
        return out;
    }
    if i.model == AeroModel::None {
        // Length in km squared times 1e4: (L/10 m)^2 of reference area, five times the pressure.
        let l_km = i.length_m * 1e-3;
        let a = 5.0 * q * l_km * l_km * 1e4 / i.mass_kg;
        out.accel = -i.v_air / v * a;
        return out;
    }
    let c = coefficients(alpha.to_degrees());
    let c2v = AERO_CHORD / (2.0 * v) * AERO_SCALE;
    let b2v = AERO_SPAN / (2.0 * v) * AERO_SCALE;
    let lim = CONTROL_LIMIT_RAD;
    let de = i.controls.x.clamp(-lim, lim);
    let dr = i.controls.y.clamp(-lim, lim);
    let da = i.controls.z.clamp(-lim, lim);
    let [w0, w1, w2] = i.w_se.to_array();
    let s2a = (2.0 * alpha).sin();
    let mut cl = 1.2 * s2a;
    if i.model == AeroModel::Shuttle {
        let hyper = 1.2 * s2a + 0.01;
        let t = (mach - 0.9) / 0.9;
        cl = if t < 0.0 {
            c[0]
        } else if t > 1.0 {
            hyper
        } else {
            (hyper - c[0]) * t + c[0]
        };
    }
    let cl = cl - c2v * 4.231 * w0 - de * 0.039;
    let lift = cl * AERO_SCALE;
    let rise = (-0.5 * ((mach - 1.0) / 0.1).powi(2)).exp();
    let drag = (rise * 4.0 + 1.0) * c[1] * AERO_SCALE;
    let cl_b = cl * 0.139_404_998_894_049_95 * b2v;
    let cm = ((c[2] + 18.8 * c2v * w0) + c[3] * 0.3 * de) * 0.01;
    let c_w2 = ((c[6] * -3.260_118_800_939_356 - cl * cl * 0.3) * b2v * 10.0 * w2
        + c[6] * beta
        + cl_b * 50.0 * -w1
        + c[10] * 0.01 * dr
        + c[8] * 50.0 * da)
        * 0.01;
    let c_w1 = (((w2 * cl_b + c[5] * beta + b2v * 0.782_145_686_794_956_7 * -w1) - c[9] * 0.1 * dr) + c[7] * 0.01 * da) * 0.01;
    let f = (q * AERO_AREA).min(AERO_FORCE_CAP);
    let (sa, ca) = alpha.sin_cos();
    out.accel = DVec3::new(
        (dr * 0.1914 - beta * c[4] - da * 0.0225) * AERO_SCALE * f / i.mass_kg,
        (drag * sa + lift * ca) * f / i.mass_kg,
        -(sa * lift - drag * ca) * f / i.mass_kg,
    );
    let [i0, i1, i2] = AERO_INERTIA;
    let m = AERO_ANG_ACCEL_MAX;
    let m_a = (((i1 - i2) * w0 * w2 + c_w1 * f) / i0).clamp(-m, m);
    let m_b = (((i2 - i0) * w2 * -w1 + cm * f) / i1).clamp(-m, m);
    let m_c = (((i0 - i1) * -w1 * w0 + c_w2 * f) / i2).clamp(-m, m);
    out.ang_accel_se = DVec3::new(-m_b, m_a, m_c);
    out
}

/// Gas constant (J/(mol K)) of SE's atmosphere code (DAT_1406d5460).
pub const R_GAS: f64 = 8.31447;
/// Pascal per atmosphere (DAT_1406d5c38).
pub const PA_PER_ATM: f64 = 101_325.0;

/// What the atmosphere state at an altitude needs (the parameters FUN_140259df0 reads).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtmoParams {
    /// Surface pressure (atm), temperature (K), molar mass (g/mol), surface gravity (m/s^2).
    pub p0_atm: f64,
    pub t0_k: f64,
    pub molar_mass: f64,
    pub gravity: f64,
    /// Polytropic exponent `a` of the lower atmosphere (`Adiabat`; T ~ P^a), so gamma = 1/(1-a).
    pub adiabat: f64,
    /// Top of the atmosphere above the surface (m): no aerodynamics above it.
    pub height_m: f64,
}

/// Air density (kg/m^3) and speed of sound (m/s) at an altitude.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AtmoState {
    pub density: f64,
    pub sound_speed: f64,
    pub pressure_atm: f64,
    pub temperature_k: f64,
}

impl AtmoParams {
    /// Scale height (m) at temperature `t`.
    fn scale_height(&self, t: f64) -> f64 {
        R_GAS * t / (self.molar_mass * 1e-3 * self.gravity.max(1e-6))
    }

    /// State at `altitude_m` (FUN_140259df0's profile): a polytropic layer from the surface up to
    /// the 0.2 atm level when the surface pressure exceeds 0.2 atm, isothermal above it (and
    /// throughout a thinner atmosphere); density from the ideal gas law, the speed of sound
    /// `sqrt(gamma R T / mu)` with `gamma = 1 / (1 - a)`.
    pub fn state(&self, altitude_m: f64) -> AtmoState {
        if self.p0_atm <= 0.0 || self.molar_mass <= 0.0 || self.t0_k <= 0.0 || altitude_m > self.height_m {
            return AtmoState::default();
        }
        let h = altitude_m.max(0.0);
        let a = self.adiabat.clamp(0.01, 0.9);
        let h0 = self.scale_height(self.t0_k);
        let (p, t) = if self.p0_atm < 0.2 {
            (self.p0_atm * (-h / h0).exp(), self.t0_k)
        } else {
            let h02 = (1.0 - (0.2 / self.p0_atm).powf(a)) * h0 / a;
            if h <= h02 {
                let tr = (1.0 - a * h / h0).max(1e-6);
                (self.p0_atm * tr.powf(1.0 / a), self.t0_k * tr)
            } else {
                let t02 = self.t0_k * (0.2 / self.p0_atm).powf(a);
                (0.2 * (-(h - h02) / self.scale_height(t02)).exp(), t02)
            }
        };
        let mu = self.molar_mass * 1e-3;
        let gamma = 1.0 / (1.0 - a);
        AtmoState {
            density: p * PA_PER_ATM * mu / (R_GAS * t),
            sound_speed: (gamma * R_GAS * t / mu).sqrt(),
            pressure_atm: p,
            temperature_k: t,
        }
    }
}

/// Molar mass (g/mol) of the gases named in a catalog `Composition` block, weighted by their
/// shares; `None` when none is known.
pub fn composition_molar_mass<'a>(gases: impl IntoIterator<Item = (&'a str, f64)>) -> Option<f64> {
    let mut sum = 0.0;
    let mut w = 0.0;
    for (name, share) in gases {
        let m = match name.to_ascii_uppercase().as_str() {
            "H2" => 2.016,
            "HE" => 4.003,
            "CH4" => 16.04,
            "NH3" => 17.03,
            "H2O" => 18.02,
            "NE" => 20.18,
            "N2" => 28.01,
            "CO" => 28.01,
            "O2" => 32.0,
            "H2S" => 34.08,
            "AR" => 39.95,
            "CO2" => 44.01,
            "SO2" => 64.07,
            _ => continue,
        };
        if share > 0.0 {
            sum += m * share;
            w += share;
        }
    }
    (w > 0.0).then(|| sum / w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_from_keys_follows_read_scheme_script() {
        assert_eq!(AeroModel::from_keys(None, None), AeroModel::None);
        assert_eq!(AeroModel::from_keys(Some(true), None), AeroModel::Shuttle);
        assert_eq!(AeroModel::from_keys(Some(false), None), AeroModel::Arrow);
        assert_eq!(AeroModel::from_keys(Some(true), Some("Arrow")), AeroModel::Arrow);
        assert_eq!(AeroModel::from_keys(None, Some("shuttle")), AeroModel::Shuttle);
        assert_eq!(AeroModel::from_keys(Some(true), Some("bogus")), AeroModel::None);
        assert!(AeroModel::Shuttle.has_control_surfaces() && !AeroModel::Arrow.has_control_surfaces());
    }

    #[test]
    fn spline_passes_through_the_nodes() {
        let c = coefficients(0.0);
        assert!((c[0] - 0.1101).abs() < 1e-12 && (c[1] - 0.0254).abs() < 1e-12, "{c:?}");
        let c = coefficients(90.0);
        assert!((c[1] - 3.0).abs() < 1e-12 && c[0].abs() < 1e-12);
        // Between nodes the spline stays close to the neighbours: lift peaks near 15 degrees.
        let c = coefficients(14.5);
        assert!(c[0] > 1.2174 && c[0] < 1.24, "{c:?}");
        assert_eq!(coefficients(-400.0), AERO_TABLE[0]);
    }

    fn input(model: AeroModel, v_air: DVec3) -> AeroInput {
        AeroInput {
            model,
            length_m: 37.0,
            mass_kg: 100_000.0,
            v_air,
            w_se: DVec3::ZERO,
            density: 1.2,
            sound_speed: 340.0,
            controls: DVec3::ZERO,
        }
    }

    #[test]
    fn none_model_is_drag_along_the_air_velocity() {
        let v = DVec3::new(0.0, 0.0, -100.0);
        let o = aero_step(&input(AeroModel::None, v));
        // 5 q (L/10)^2 / m, pointing backwards (+z).
        let want = 5.0 * 0.5 * 1.2 * 100.0 * 100.0 * 3.7 * 3.7 / 100_000.0;
        assert!((o.accel - DVec3::new(0.0, 0.0, want)).length() < 1e-9, "{o:?}");
        assert!((o.mach - 100.0 / 340.0).abs() < 1e-12);
    }

    #[test]
    fn shuttle_lifts_at_positive_attack_and_weathervanes() {
        // Flying forward and sinking: the air comes from below, alpha = +10 degrees.
        let a = 10f64.to_radians();
        let v = DVec3::new(0.0, -a.sin(), -a.cos()) * 150.0;
        let o = aero_step(&input(AeroModel::Shuttle, v));
        assert!((o.alpha - a).abs() < 1e-12 && o.beta.abs() < 1e-12);
        assert!(o.accel.y > 0.0, "lift {o:?}");
        assert!(o.accel.dot(v) < 0.0, "drag opposes the motion {o:?}");
        // Nose-down moment (positive in SE's convention: the physical rate is its negation).
        assert!(o.ang_accel_se.x > 0.0, "{o:?}");
        // Sliding right: the nose turns right (physical yaw negative, SE's positive).
        let o = aero_step(&input(AeroModel::Shuttle, DVec3::new(20.0, 0.0, -150.0)));
        assert!(o.accel.x < 0.0 && o.ang_accel_se.y > 0.0, "{o:?}");
    }

    #[test]
    fn transonic_drag_rise_and_force_cap() {
        let v = |m: f64| DVec3::new(0.0, 0.0, -340.0 * m);
        let d = |m: f64| {
            let mut i = input(AeroModel::Arrow, v(m));
            i.density = 1e-4;
            aero_step(&i).accel.z / (m * m)
        };
        // Five times the drag coefficient at Mach 1.
        assert!((d(1.0) / d(0.5) - 5.0).abs() < 0.01, "{} {}", d(1.0), d(0.5));
        // At sea level and Mach 5 the force factor is capped at 1e5.
        let o = aero_step(&input(AeroModel::Arrow, v(5.0)));
        let want = 0.0254 * (4.0 * (-0.5f64 * 1600.0).exp() + 1.0) * 100.0 * AERO_FORCE_CAP / 100_000.0;
        assert!((o.accel.z - want).abs() < 1e-6, "{o:?}");
    }

    #[test]
    fn earth_like_atmosphere() {
        let p = AtmoParams { p0_atm: 1.0, t0_k: 288.0, molar_mass: 29.0, gravity: 9.81, adiabat: 0.25, height_m: 100_000.0 };
        let s = p.state(0.0);
        assert!((s.density - 1.23).abs() < 0.02, "{s:?}");
        assert!((s.sound_speed - 333.0).abs() < 5.0, "{s:?}");
        let hi = p.state(30_000.0);
        assert!(hi.density < s.density * 0.05 && hi.density > 0.0, "{hi:?}");
        assert_eq!(p.state(200_000.0), AtmoState::default());
        assert!((composition_molar_mass([("N2", 78.0), ("O2", 21.0), ("Ar", 1.0)]).unwrap() - 28.96).abs() < 0.05);
    }
}
