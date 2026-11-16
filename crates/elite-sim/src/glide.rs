//! The glide (step 6.2, first cut): from supercruise into a planet's lower air, at a fixed speed along the nose, ending in a
//! deceleration to normal flight.
//!
//! Rules (grade C, A60, `docs/elite-flight-model.md`):
//! - the glide's pitch is `asin(-(d . n))` in degrees, d towards the body's centre, n the nose (negative diving);
//! - steep at or under `steepestDivePitch`, else shallow at or over `shallowestDivePitch`, else in the band;
//! - on the first frame out of the band ends it at once (too steep on entry adds the "Planet Arrival" heat); later, out of the
//!   band a timer runs (reset in the band), too steep adds heat each second, and past `alignmentGracePeriod` it ends (too
//!   steep with the heat again, too shallow plainly);
//! - the end tests: terrain along the nose nearer than 10,000 m (TerrainDanger), the surface towards the centre nearer than
//!   6,500 m (AltitudeTooLow, the normal end);
//! - every end, good or bad, starts the same deceleration: `e(t) = (1 - t/T)^p`, a tick's step `min(speedLimit dt, D (e_old -
//!   e_new))`, over once the step falls under `endSpeed dt` (or at t = T); no damage or drop is in this code;
//! - while gliding the velocity is `speedLimit` along the nose; entering it (the hand-over from supercruise) the ship is set at
//!   2,500 m/s along the nose and moved 3 x 2,500 m ahead. The body's gravity and size enter none of this.
//!
//! Numbers: the end distances, the hand-over's speed and the jump ahead are the exe's own (C). The two "Planet Arrival" heats
//! are the ship's (B: the performance record's +0x320, 72 at once, and +0x31C, 36 a second, read through 0x141B3AFF0 by
//! 0x142C761B0). The `CruiseArrivalParams` (`speedLimit`, the band's two pitches, the grace, D, T, p, `endSpeed`) are in none
//! of the records read (N1: their sender, component type 0x145F2F260's vt+0x180, is a lead): stand-ins (E) until N1's G1 run
//! measures a glide: the band is the spec's hint (-5 to -60 degrees), T and p the struct's own defaults (1.5 s, 4: the sender
//! overwrites them), D such that the deceleration starts at the speed limit (`speedLimit T / p`: F).

/// The exe's constants (C).
pub const HANDOVER_SPEED: f64 = 2500.0;
pub const HANDOVER_AHEAD_M: f64 = 3.0 * HANDOVER_SPEED;
pub const END_AHEAD_M: f64 = 10_000.0;
pub const END_BELOW_M: f64 = 6500.0;

/// The "Planet Arrival" heats (B): at a too steep entry or end, and each second too steep inside the grace, in heat units
/// (the heat capacity is 140).
pub const STEEP_HEAT: f64 = 72.0;
pub const STEEP_HEAT_PER_S: f64 = 36.0;

/// Stand-ins (E) until measured.
pub mod stand_in {
    /// `speedLimit` (m/s: the hand-over's speed), the band's pitches (degrees: the spec's hint), the grace (s).
    pub const SPEED_LIMIT: f64 = 2500.0;
    pub const STEEPEST_DEG: f64 = -60.0;
    pub const SHALLOWEST_DEG: f64 = -5.0;
    pub const GRACE_S: f64 = 3.0;
    /// The deceleration: T (s) and p (the struct's defaults), D (m: starts at the speed limit), `endSpeed` (m/s).
    pub const DECEL_S: f64 = 1.5;
    pub const POWER: f64 = 4.0;
    pub const DECEL_M: f64 = SPEED_LIMIT * DECEL_S / POWER;
    pub const END_SPEED: f64 = 150.0;
    /// The height over the ground a descending ship in supercruise enters the glide at (the spec's hint: about 25 km). The
    /// exe's safe stop sweeps a sphere of the record's 8,000 m along the path (`supercruise::SWEPT_M`, B), but against which
    /// radius of the body (its ground, its sea level, another shell) is open: kept until N1's G1 run shows the height a glide
    /// starts at.
    pub const ENTRY_M: f64 = 25_000.0;
}

/// The two heats as shares of the stock Sidewinder's heat capacity (what `Tick::heat_share` carries).
fn steep_share(heat: f64) -> f64 {
    heat / crate::data::SIDEWINDER.heat_capacity
}

/// Why a glide ended (the exe's reasons that can happen here, its numbers in brackets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlideEnd {
    /// (2) The normal end: under 6,500 m.
    AltitudeTooLow,
    /// (3) Terrain within 10,000 m along the nose.
    TerrainDanger,
    /// (4), (5)
    TooSteepOnEntry,
    TooSteep,
    /// (6), (7)
    TooShallowOnEntry,
    TooShallow,
}

impl GlideEnd {
    /// The HUD's message (our words).
    pub fn message(self) -> &'static str {
        match self {
            GlideEnd::AltitudeTooLow | GlideEnd::TerrainDanger => "Glide complete",
            GlideEnd::TooSteepOnEntry | GlideEnd::TooSteep => "Glide aborted: too steep",
            GlideEnd::TooShallowOnEntry | GlideEnd::TooShallow => "Glide aborted: too shallow",
        }
    }

    /// Whether the end adds the "Planet Arrival" heat.
    pub fn heats(self) -> bool {
        matches!(self, GlideEnd::TooSteepOnEntry | GlideEnd::TooSteep)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    first: bool,
    out_of_band: f64,
    /// Decelerating: seconds into it.
    decel: Option<f64>,
    pub ended: Option<GlideEnd>,
}

impl Default for Glide {
    fn default() -> Self {
        Glide { first: true, out_of_band: 0.0, decel: None, ended: None }
    }
}

/// What a tick gives: the speed along the nose (`None` once the deceleration is over: normal flight), the end the tick it
/// happens, and the heat to add as a share of the heat capacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub speed: Option<f64>,
    pub end: Option<GlideEnd>,
    pub heat_share: f64,
}

/// The glide's pitch (degrees, negative diving) from the nose and the way down (unit vectors).
pub fn pitch_deg(nose: [f64; 3], down: [f64; 3]) -> f64 {
    let d = nose[0] * down[0] + nose[1] * down[1] + nose[2] * down[2];
    (-d).clamp(-1.0, 1.0).asin().to_degrees()
}

fn e(t: f64) -> f64 {
    (1.0 - t / stand_in::DECEL_S).max(0.0).powf(stand_in::POWER)
}

impl Glide {
    /// Decelerating (an end has happened).
    pub fn decelerating(&self) -> bool {
        self.decel.is_some()
    }

    /// A tick: the pitch now, the terrain's distance along the nose (m, `f64::MAX` if none within reach) and the height over
    /// the surface towards the centre (m).
    pub fn tick(&mut self, dt: f64, pitch: f64, ahead_m: f64, below_m: f64) -> Tick {
        use stand_in::*;
        if let Some(t) = self.decel.as_mut() {
            let t0 = *t;
            *t = (t0 + dt).min(DECEL_S);
            let step = (SPEED_LIMIT * dt).min(DECEL_M * (e(t0) - e(*t)));
            let floor = END_SPEED * dt;
            let speed = if step < floor || t0 >= DECEL_S { None } else { Some(step / dt) };
            return Tick { speed, end: None, heat_share: 0.0 };
        }
        let first = std::mem::take(&mut self.first);
        let steep = pitch <= STEEPEST_DEG;
        let shallow = !steep && pitch >= SHALLOWEST_DEG;
        let mut heat_share = 0.0;
        let end = if ahead_m < END_AHEAD_M {
            Some(GlideEnd::TerrainDanger)
        } else if below_m < END_BELOW_M {
            Some(GlideEnd::AltitudeTooLow)
        } else if first && steep {
            Some(GlideEnd::TooSteepOnEntry)
        } else if first && shallow {
            Some(GlideEnd::TooShallowOnEntry)
        } else if steep || shallow {
            self.out_of_band += dt;
            if steep {
                heat_share += steep_share(STEEP_HEAT_PER_S) * dt;
            }
            (self.out_of_band >= GRACE_S).then_some(if steep { GlideEnd::TooSteep } else { GlideEnd::TooShallow })
        } else {
            self.out_of_band = 0.0;
            None
        };
        if let Some(e) = end {
            if e.heats() {
                heat_share += steep_share(STEEP_HEAT);
            }
            self.decel = Some(0.0);
            self.ended = Some(e);
        }
        Tick { speed: Some(SPEED_LIMIT), end, heat_share }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In the band it glides at the limit; under 6,500 m it ends (AltitudeTooLow) and decelerates, the steps' sum D or less,
    /// then hands over under the end speed; out of the band on the first frame it ends at once; later only past the grace,
    /// heating each second while too steep.
    #[test]
    fn glide_band_ends_and_deceleration() {
        let dt = 1.0 / 60.0;
        assert!((pitch_deg([0.0, 0.0, -1.0], [0.0, -1.0, 0.0])).abs() < 1e-9);
        assert!((pitch_deg([0.0, -1.0, 0.0], [0.0, -1.0, 0.0]) + 90.0).abs() < 1e-9);
        let mut g = Glide::default();
        assert_eq!(g.tick(dt, -30.0, f64::MAX, 20_000.0), Tick { speed: Some(stand_in::SPEED_LIMIT), end: None, heat_share: 0.0 });
        assert_eq!(g.tick(dt, -30.0, f64::MAX, 6000.0).end, Some(GlideEnd::AltitudeTooLow));
        let (mut dist, mut last, mut n) = (0.0, f64::MAX, 0);
        while let Some(v) = g.tick(dt, -30.0, f64::MAX, 3000.0).speed {
            assert!(v <= last + 1e-9 && v <= stand_in::SPEED_LIMIT + 1e-9);
            dist += v * dt;
            last = v;
            n += 1;
        }
        assert!(last >= stand_in::END_SPEED && dist <= stand_in::DECEL_M + 1e-6 && (n as f64) * dt <= stand_in::DECEL_S + 1e-9);
        assert_eq!(Glide::default().tick(dt, -75.0, f64::MAX, 20_000.0).end, Some(GlideEnd::TooSteepOnEntry));
        assert_eq!(Glide::default().tick(dt, 2.0, f64::MAX, 20_000.0).end, Some(GlideEnd::TooShallowOnEntry));
        assert_eq!(Glide::default().tick(dt, -30.0, 9000.0, 20_000.0).end, Some(GlideEnd::TerrainDanger));
        for (pitch, want) in [(-75.0, GlideEnd::TooSteep), (2.0, GlideEnd::TooShallow)] {
            let mut s = Glide::default();
            s.tick(dt, -30.0, f64::MAX, 20_000.0);
            let (mut end, mut heat, mut t) = (None, 0.0, 0.0);
            while end.is_none() && t < 10.0 {
                let k = s.tick(dt, pitch, f64::MAX, 20_000.0);
                end = k.end;
                heat += k.heat_share;
                t += dt;
            }
            assert_eq!(end, Some(want));
            assert!((t - stand_in::GRACE_S).abs() < 2.0 * dt, "{t}");
            // (72 at the end and 36 a second through the grace, of 140.)
            let want_heat = if want.heats() { (STEEP_HEAT_PER_S * stand_in::GRACE_S + STEEP_HEAT) / 140.0 } else { 0.0 };
            assert!((heat - want_heat).abs() < 0.01, "{heat} {want_heat}");
        }
    }
}
