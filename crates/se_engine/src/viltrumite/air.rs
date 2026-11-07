//! The air he flies through (spec F9): the one source for drag, the cap in air, the readout and, from steps 6 and 7,
//! the boom, the heat and the sounds (`Flow`, one frame's reading: q, Mach, the temperature of F9).
//!
//! It takes `ship/air.rs`'s parameters and differs in three ways. The air reaches 25 scale heights, because
//! `AtmoParams::state` gives zeros above `height_m` and the drawn shell's top is only a drawing height (Earth 60 km,
//! Mars 132, Venus 200: at Earth's 60 km, 30 km/s would jump from nothing to 10,300 K). The temperature never falls
//! under a quarter of the surface's (Venus lacks `Adiabat` and floors at 20 K above 60 km). Giants without `Pressure`
//! get 1 atm at their radius. Stars and bodies with no shell have no air. One mean temperature per body, no day and night.

use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::ship::aero::AtmoParams;

use crate::data::SeData;
use crate::ship::air::AirCache;
use crate::terrain::Terrain;

use super::tune;

const R_GAS: f64 = 8.314_462;
const PA_PER_ATM: f64 = 101_325.0;
/// A giant without `Pressure`: 1 atm at the radius, at about Jupiter's temperature and hydrogen and helium.
const GIANT_T0_K: f64 = 165.0;
const GIANT_MOLAR_MASS: f64 = 2.3;
/// The share of the surface temperature the air never falls under.
const COLDEST_SHARE: f64 = 0.25;
/// F9's heating: Sutton-Graves `k sqrt(rho / NOSE_RADIUS_M) v^3`, radiated at `EMISSIVITY`.
const NOSE_RADIUS_M: f64 = 0.15;
const EMISSIVITY: f64 = 0.8;
const STEFAN_BOLTZMANN: f64 = 5.670e-8;
/// F9's density fade of the heating, log10 rho: nothing under the first, all of it over the second.
const HEAT_LOG_RHO: (f64, f64) = (-7.0, -5.0);

/// The air at a point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AirSample {
    pub pressure_pa: f64,
    pub density: f64,
    pub temperature_k: f64,
    pub sound_speed: f64,
}

/// What he flies through, one frame: the one reading the sounds, the readout and (step 7) the boom and the heat's look go by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flow {
    pub air: AirSample,
    /// His speed through the air (m/s): over the ground, the body's spin and motion taken out.
    pub speed: f64,
    /// Dynamic pressure, `rho v^2 / 2` (Pa).
    pub q: f64,
    /// 0 out of the air.
    pub mach: f64,
    /// What his skin reaches here at this speed, before the low-pass `ease_heat` (K). 0 out of the air.
    pub skin_k: f64,
}

impl Flow {
    /// Out of any air.
    pub fn vacuum(speed: f64) -> Flow {
        Flow { speed, ..Flow::default() }
    }

    pub fn in_air(&self) -> bool {
        self.air.density > 0.0 && self.air.sound_speed > 0.0
    }

    /// `log10 q`, minus infinity at none: every curve by it starts from its low end.
    pub fn log_q(&self) -> f64 {
        if self.q > 0.0 { self.q.log10() } else { f64::NEG_INFINITY }
    }
}

/// F9's three states of the air's sound barrier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoomState {
    #[default]
    NoAir,
    Sub,
    Super,
}

/// What a frame's reading did to the barrier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoomEvent {
    /// Sub to Super: the boom, loud by this dynamic pressure (Pa).
    Boom(f64),
    /// Super to Sub: `supersonicEnd`, a pulse of the cone, no boom.
    End,
}

/// F9's boom: the air counts over 150 Pa and stops under 100, Sub to Super at Mach 1 (a boom, at most one in 3 s), Super to Sub at Mach
/// 0.92. Coming out of `NoAir` is always silent, whatever the Mach number: an entry from orbit doesn't boom (Mach reads 0 above the air),
/// and neither does turning on at speed. Default (`NoAir`) is the state on turning on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Boom {
    pub state: BoomState,
    /// `Hero.clock` at the last boom.
    last: Option<f64>,
}

impl Boom {
    /// One frame's reading, `clock` in real seconds.
    pub fn step(&mut self, flow: &Flow, clock: f64) -> Option<BoomEvent> {
        let p = if flow.in_air() { flow.air.pressure_pa } else { 0.0 };
        let (air_on, air_off) = tune::BOOM_AIR_PA;
        let (up, down) = tune::BOOM_MACH;
        match self.state {
            BoomState::NoAir => {
                if p >= air_on {
                    self.state = if flow.mach >= up { BoomState::Super } else { BoomState::Sub };
                }
                None
            }
            _ if p < air_off => {
                self.state = BoomState::NoAir;
                None
            }
            BoomState::Sub => {
                if flow.mach < up {
                    return None;
                }
                self.state = BoomState::Super;
                if self.last.is_some_and(|t| clock - t < tune::BOOM_COOLDOWN_S) {
                    return None;
                }
                self.last = Some(clock);
                Some(BoomEvent::Boom(flow.q))
            }
            BoomState::Super => {
                if flow.mach > down {
                    return None;
                }
                self.state = BoomState::Sub;
                Some(BoomEvent::End)
            }
        }
    }
}

/// F9's low-pass of the temperature: it rises faster than it falls.
pub fn ease_heat(heat_k: f64, skin_k: f64, dt: f64) -> f64 {
    let tau = if skin_k > heat_k { tune::HEAT_RISE_S } else { tune::HEAT_FALL_S };
    heat_k + (skin_k - heat_k) * (1.0 - (-dt / tau).exp())
}

/// A body's air: where it ends, and how it changes with height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirModel {
    pub params: AtmoParams,
}

impl AirModel {
    /// The air of `obj`, `None` with no atmosphere shell (stars, airless bodies).
    pub fn of(cache: &mut AirCache, data: &SeData, eph: &Ephemeris, jd: f64, obj: ObjectRef, radius_m: f64, terrain: Option<&Terrain>) -> Option<AirModel> {
        let terrain = terrain?;
        let top = terrain.atmosphere.as_ref().map(|a| a.top_radius_m(radius_m) - radius_m)?;
        // (A giant without `Pressure`, as all four of the solar system's are: 1 atm at the radius.)
        let params = cache.get(data, eph, jd, obj, radius_m, Some(top)).or_else(|| {
            terrain.proc.kind.is_giant().then_some(AtmoParams { p0_atm: 1.0, t0_k: GIANT_T0_K, molar_mass: GIANT_MOLAR_MASS, gravity: 24.8, adiabat: 0.25, height_m: 0.0 })
        })?;
        // The body's own gravity: the climate's completion says 9.81 for some moons (Titan's air would end 8 times too low).
        let gravity = se_core::units::G * crate::ship::body_mass(data, obj, radius_m) / (radius_m * radius_m);
        Some(Self::from_params(AtmoParams { gravity: if gravity > 0.0 { gravity } else { params.gravity }, ..params }))
    }

    pub fn from_params(params: AtmoParams) -> AirModel {
        AirModel { params: AtmoParams { height_m: tune::CAP_AIR_SCALE_HEIGHTS * params.scale_height(), ..params } }
    }

    /// Where the air (and the cap in it) ends, over the ground (m).
    pub fn top_m(&self) -> f64 {
        self.params.height_m
    }

    /// `height_m` over the ground (the surface's pressure is at the ground, as SE's).
    pub fn at(&self, height_m: f64) -> AirSample {
        let st = self.params.state(height_m);
        if st.density <= 0.0 {
            return AirSample::default();
        }
        let t = st.temperature_k.max(self.params.t0_k * COLDEST_SHARE);
        let mu = self.params.molar_mass * 1e-3;
        let gamma = 1.0 / (1.0 - self.params.adiabat.clamp(0.01, 0.9));
        let pressure_pa = st.pressure_atm * PA_PER_ATM;
        AirSample { pressure_pa, density: pressure_pa * mu / (R_GAS * t), temperature_k: t, sound_speed: (gamma * R_GAS * t / mu).sqrt() }
    }

    /// `gamma` of the gas, from the polytrope as `AtmoParams::state`.
    fn gamma(&self) -> f64 {
        1.0 / (1.0 - self.params.adiabat.clamp(0.01, 0.9))
    }

    /// F9's `k`, by molar mass.
    fn heat_k(&self) -> f64 {
        if self.params.molar_mass >= 40.0 {
            1.90e-4
        } else if self.params.molar_mass >= 10.0 {
            1.74e-4
        } else {
            6.6e-5
        }
    }

    /// The reading at `height_m` over the ground, going `speed` through the air. The temperature is F9's: Sutton-Graves against
    /// radiation, never over the air's stagnation temperature (Sutton-Graves alone gives 800 K at sea-level Mach 1).
    pub fn flow(&self, height_m: f64, speed: f64) -> Flow {
        let air = self.at(height_m);
        if air.density <= 0.0 || air.sound_speed <= 0.0 {
            return Flow::vacuum(speed);
        }
        let mach = speed / air.sound_speed;
        let k = ((air.density.log10() - HEAT_LOG_RHO.0) / (HEAT_LOG_RHO.1 - HEAT_LOG_RHO.0)).clamp(0.0, 1.0);
        let flux = self.heat_k() * (air.density / NOSE_RADIUS_M).sqrt() * speed.powi(3) * (k * k * (3.0 - 2.0 * k));
        let radiating = (flux / (EMISSIVITY * STEFAN_BOLTZMANN)).powf(0.25);
        let stagnation = air.temperature_k * (1.0 + (self.gamma() - 1.0) / 2.0 * mach * mach);
        Flow { air, speed, q: 0.5 * air.density * speed * speed, mach, skin_k: radiating.min(stagnation) }
    }

    /// Drag per metre, `0.5 rho CdA/m`: `dv/dt = -k |v| v`.
    pub fn drag(&self, height_m: f64) -> f64 {
        0.5 * self.at(height_m).density * tune::DRAG_CDA_M
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth() -> AirModel {
        AirModel::from_params(AtmoParams { p0_atm: 1.0, t0_k: 288.0, molar_mass: 29.0, gravity: 9.81, adiabat: 0.25, height_m: 0.0 })
    }

    #[test]
    fn the_air_reaches_25_scale_heights_and_stops() {
        let air = earth();
        let h = air.params.scale_height();
        assert!((air.top_m() - 25.0 * h).abs() < 1.0);
        assert!(air.at(60e3).density > 0.0, "Earth's drawn shell ends at 60 km: the air doesn't");
        assert!(air.at(24.0 * h).pressure_pa > 0.0);
        assert_eq!(air.at(26.0 * h), AirSample::default());
    }

    #[test]
    fn earth_at_sea_level_is_what_it_should_be() {
        let s = earth().at(0.0);
        assert!((s.density - 1.2).abs() < 0.1, "{s:?}");
        assert!((s.pressure_pa - 101_325.0).abs() < 1.0);
        // SE's polytrope gives gamma 4/3.
        assert!((270.0..345.0).contains(&s.sound_speed), "{s:?}");
    }

    #[test]
    fn it_is_never_colder_than_a_quarter_of_the_surface() {
        // Venus: thick, hot, no Adiabat (the default), floors at 20 K above 60 km in `state` alone.
        let v = AirModel::from_params(AtmoParams { p0_atm: 92.0, t0_k: 737.0, molar_mass: 44.0, gravity: 8.87, adiabat: 0.25, height_m: 0.0 });
        let s = v.at(100e3);
        assert!(s.temperature_k >= 737.0 * 0.25 - 1e-6, "{s:?}");
        assert!(s.density > 0.0);
    }

    #[test]
    fn q_and_mach_are_what_they_should_be() {
        let air = earth();
        let f = air.flow(0.0, 20.0);
        assert!((f.q - 0.5 * f.air.density * 400.0).abs() < 1e-6);
        assert!((f.q - 240.0).abs() < 20.0, "{f:?}");
        assert!((f.mach - 20.0 / f.air.sound_speed).abs() < 1e-9);
        // The same reading, past the air's end: nothing.
        let top = air.flow(26.0 * air.params.scale_height(), 3000.0);
        assert_eq!(top, Flow::vacuum(3000.0));
        assert!(!top.in_air() && top.log_q() == f64::NEG_INFINITY);
        // Standing still: no heat.
        assert_eq!(air.flow(0.0, 0.0).skin_k, 0.0);
    }

    /// F9's table, within 10 % of the temperature (C16), with the worlds the table was worked out for.
    #[test]
    fn the_heat_matches_the_table() {
        let table = |p0_atm, t0_k, molar_mass, gravity, adiabat, h: f64, speeds: [f64; 5]| {
            let air = AirModel::from_params(AtmoParams { p0_atm, t0_k, molar_mass, gravity, adiabat, height_m: 0.0 });
            for (v, want) in speeds.iter().zip([800.0, 2000.0, 3500.0, 6000.0, 10000.0]) {
                let got = air.flow(h, v * 1e3).skin_k;
                assert!((got / want - 1.0).abs() < 0.1, "{p0_atm} atm, {h} m, {v} km/s: {got} K, table {want} K");
            }
        };
        table(1.0, 287.0, 28.97, 9.81, 0.28, 0.0, [1.0, 1.9, 2.6, 4.9, 9.7]);
        table(1.0, 287.0, 28.97, 9.81, 0.28, 60e3, [1.1, 3.4, 7.1, 14.6, 28.9]);
        table(0.00638, 214.0, 43.4, 3.71, 0.25, 0.0, [0.9, 2.3, 4.8, 9.8, 19.5]);
        table(92.0, 760.0, 43.5, 8.87, 0.25, 0.0, [0.2, 1.4, 2.0, 2.8, 4.9]);
        table(0.001, 250.0, 28.0, 5.0, 0.25, 0.0, [1.1, 3.5, 7.4, 15.2, 30.1]);
        // The same speeds in the air the game really has (`hero air`, 2026-11-07).
        table(1.0, 281.57, 28.92, 9.798, 0.28, 0.0, [1.0, 1.9, 2.6, 4.9, 9.7]);
        table(1.0, 281.57, 28.92, 9.798, 0.28, 60e3, [1.1, 3.4, 7.1, 14.6, 28.9]);
        table(0.00638, 214.13, 43.6, 3.714, 0.25, 0.0, [0.9, 2.3, 4.8, 9.8, 19.5]);
        table(92.0, 788.79, 43.45, 8.869, 0.25, 0.0, [0.2, 1.4, 2.0, 2.8, 4.9]);
    }

    /// What a test flies through: the air `air` at this height and speed.
    fn boom_along(path: &[(f64, f64)], air: &AirModel) -> (Vec<BoomEvent>, BoomState) {
        let mut b = Boom::default();
        let mut events = Vec::new();
        for (i, &(h, v)) in path.iter().enumerate() {
            events.extend(b.step(&air.flow(h, v), i as f64 / 60.0));
        }
        (events, b.state)
    }

    /// C15: from orbit into the air at 7.8 km/s: no boom on the way in, whatever the Mach number; slowing down it's the end of the
    /// barrier, no boom; speeding up again, one.
    #[test]
    fn re_entry_does_not_boom() {
        let air = earth();
        let mut path: Vec<(f64, f64)> = (0..2000).map(|i| (300e3 - 150.0 * i as f64, 7800.0)).collect();
        let (e, s) = boom_along(&path, &air);
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(s, BoomState::Super);
        path.extend((0..600).map(|i| (0.0, 7800.0 * (1.0 - i as f64 / 600.0))));
        let (e, s) = boom_along(&path, &air);
        assert_eq!(e, [BoomEvent::End]);
        assert_eq!(s, BoomState::Sub);
        path.extend((0..600).map(|i| (0.0, 2.0 * i as f64)));
        let (e, s) = boom_along(&path, &air);
        assert!(matches!(e[..], [BoomEvent::End, BoomEvent::Boom(q)] if q > 5e4), "{e:?}");
        assert_eq!(s, BoomState::Super);
    }

    /// Turning on at speed in the air (the first frame from `NoAir`) is silent too.
    #[test]
    fn turning_on_at_speed_does_not_boom() {
        let (e, s) = boom_along(&[(1000.0, 2000.0); 30], &earth());
        assert!(e.is_empty());
        assert_eq!(s, BoomState::Super);
    }

    #[test]
    fn the_barrier_has_its_hysteresis_and_its_cooldown() {
        let air = earth();
        let a = air.at(0.0).sound_speed;
        let mut b = Boom::default();
        let at = |m: f64| air.flow(0.0, m * a);
        assert_eq!(b.step(&at(0.5), 0.0), None);
        assert_eq!(b.state, BoomState::Sub);
        assert_eq!(b.step(&at(0.999), 1.0), None);
        assert!(matches!(b.step(&at(1.001), 2.0), Some(BoomEvent::Boom(_))));
        // Back under Mach 1 it stays Super, down to 0.92; then up to 0.99 it stays Sub.
        assert_eq!(b.step(&at(0.95), 3.0), None);
        assert_eq!(b.state, BoomState::Super);
        assert_eq!(b.step(&at(0.91), 3.5), Some(BoomEvent::End));
        assert_eq!(b.state, BoomState::Sub);
        assert_eq!(b.step(&at(0.99), 4.0), None);
        assert_eq!(b.state, BoomState::Sub);
        // Past Mach 1 again 2.5 s after the boom: Super, but silent. It ends, and 3 s on booms again.
        assert_eq!(b.step(&at(1.1), 4.5), None);
        assert_eq!(b.state, BoomState::Super);
        assert_eq!(b.step(&at(0.5), 4.9), Some(BoomEvent::End));
        assert!(matches!(b.step(&at(1.1), 5.0), Some(BoomEvent::Boom(_))));
        assert_eq!(b.step(&at(0.5), 5.1), Some(BoomEvent::End));
        assert_eq!(b.step(&at(1.1), 7.9), None);
    }

    #[test]
    fn the_air_counts_between_100_and_150_pa() {
        let air = earth();
        let h_at = |p: f64| (air.params.p0_atm * PA_PER_ATM / p).ln() * air.params.scale_height();
        let mut b = Boom::default();
        // Coming down: not at 120 Pa, at 160. Going up: still at 120, gone at 90.
        assert_eq!(b.step(&air.flow(h_at(120.0), 100.0), 0.0), None);
        assert_eq!(b.state, BoomState::NoAir);
        assert_eq!(b.step(&air.flow(h_at(160.0), 100.0), 0.1), None);
        assert_eq!(b.state, BoomState::Sub);
        assert_eq!(b.step(&air.flow(h_at(120.0), 100.0), 0.2), None);
        assert_eq!(b.state, BoomState::Sub);
        assert_eq!(b.step(&air.flow(h_at(90.0), 100.0), 0.3), None);
        assert_eq!(b.state, BoomState::NoAir);
        // Out of the air altogether.
        let mut b = Boom { state: BoomState::Super, last: None };
        assert_eq!(b.step(&Flow::vacuum(3000.0), 0.0), None);
        assert_eq!(b.state, BoomState::NoAir);
    }

    /// The table's other line: the height of 150 Pa on the worlds it was worked out for. A 1 mbar world never has a boom, however fast.
    #[test]
    fn the_top_of_the_boom_is_where_the_table_says() {
        let top = |p0_atm: f64, t0_k, molar_mass, gravity, adiabat| {
            let air = AirModel::from_params(AtmoParams { p0_atm, t0_k, molar_mass, gravity, adiabat, height_m: 0.0 });
            // The real air, by steps: its temperature falls with height, so no closed form.
            (0..400_000).map(|i| i as f64).find(|&h| air.at(h).pressure_pa < tune::BOOM_AIR_PA.0).unwrap_or(0.0) / 1e3
        };
        for (got, want) in [(top(1.0, 281.57, 28.92, 9.798, 0.28), 55.0), (top(0.00638, 214.13, 43.6, 3.714, 0.25), 16.0), (top(92.0, 788.79, 43.45, 8.869, 0.25), 181.0)] {
            assert!((got / want - 1.0).abs() < 0.1, "{got} km, table {want} km");
        }
        let thin = AirModel::from_params(AtmoParams { p0_atm: 0.001, t0_k: 250.0, molar_mass: 28.0, gravity: 5.0, adiabat: 0.25, height_m: 0.0 });
        let (e, s) = boom_along(&(0..600).map(|i| (0.0, 50.0 * i as f64)).collect::<Vec<_>>(), &thin);
        assert!(e.is_empty());
        assert_eq!(s, BoomState::NoAir);
        // Its heat goes by density all the same: the table's 1 mbar line is covered above.
        assert!(thin.flow(0.0, 3500.0).skin_k > 1800.0);
    }

    #[test]
    fn the_heat_rises_faster_than_it_falls() {
        let mut t = 0.0;
        for _ in 0..24 {
            t = ease_heat(t, 1000.0, 1.0 / 60.0);
        }
        assert!((t - 1000.0 * (1.0 - (-1.0f64).exp())).abs() < 1.0, "one rise time: {t}");
        let mut t = 1000.0;
        for _ in 0..90 {
            t = ease_heat(t, 0.0, 1.0 / 60.0);
        }
        assert!((t - 1000.0 * (-1.0f64).exp()).abs() < 1.0, "one fall time: {t}");
    }

    #[test]
    fn terminal_speed_on_earth_is_about_45_m_s() {
        let k = earth().drag(0.0);
        let terminal = (9.81 / k).sqrt();
        assert!((terminal - 45.0).abs() < 2.0, "{terminal}");
    }
}
