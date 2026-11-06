//! The air he flies through (spec F9): the one source for drag, the cap in air, the readout and, from steps 6 and 7,
//! the boom, the heat and the sounds.
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

/// The air at a point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AirSample {
    pub pressure_pa: f64,
    pub density: f64,
    pub temperature_k: f64,
    pub sound_speed: f64,
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
        let params = cache.get(data, eph, jd, obj, radius_m, Some(top)).or_else(|| {
            terrain.proc.kind.is_giant().then(|| {
                let mass = if let ObjectRef::Body(i) = obj { data.catalog.bodies.get(i as usize).map_or(0.0, |b| b.mass_kg) } else { 0.0 };
                let gravity = if mass > 0.0 { se_core::units::G * mass / (radius_m * radius_m) } else { 24.8 };
                AtmoParams { p0_atm: 1.0, t0_k: GIANT_T0_K, molar_mass: GIANT_MOLAR_MASS, gravity, adiabat: 0.25, height_m: 0.0 }
            })
        })?;
        Some(Self::from_params(params))
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
    fn terminal_speed_on_earth_is_about_45_m_s() {
        let k = earth().drag(0.0);
        let terminal = (9.81 / k).sqrt();
        assert!((terminal - 45.0).abs() < 2.0, "{terminal}");
    }
}
