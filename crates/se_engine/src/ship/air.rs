//! The air a ship flies through: each body's atmosphere parameters for
//! [`se_core::ship::aero::AtmoParams`], worked out once per body and cached.
//!
//! SE reads them from the planet's atmosphere model (FUN_14025a220 / FUN_140259df0): surface
//! pressure, temperature, molar mass, gravity and the polytropic exponent. The port takes them
//! from the climate completion SE gives bodies the catalog leaves without an atmosphere, or from
//! the catalog's `Atmosphere` block (pressure, `Adiabat`, `Composition`) with the equilibrium
//! temperature plus `Greenhouse` (as `system.rs` does for the surface).

use std::collections::HashMap;

use bevy::prelude::*;
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::ship::aero::{AtmoParams, composition_molar_mass};
use se_core::units::G;

use crate::data::SeData;

/// Polytropic exponent SE's climate code uses when nothing sets one (`height_at`'s 0.25).
const DEFAULT_ADIABAT: f64 = 0.25;
/// Molar mass (g/mol) of air, for catalog atmospheres without a known composition.
const AIR_MOLAR_MASS: f64 = 29.0;

/// Per-body atmosphere parameters (`None`: no air), keyed by object.
#[derive(Resource, Default)]
pub struct AirCache(HashMap<ObjectRef, Option<AtmoParams>>);

impl AirCache {
    /// The atmosphere of `obj`, whose visual atmosphere reaches `height_m` above its surface
    /// (`None` without an atmosphere shell: SE applies no aerodynamics there).
    pub fn get(&mut self, data: &SeData, eph: &Ephemeris, jd: f64, obj: ObjectRef, radius_m: f64, height_m: Option<f64>) -> Option<AtmoParams> {
        let height_m = height_m.filter(|h| *h > 0.0)?;
        let p = *self.0.entry(obj).or_insert_with(|| body_air(data, eph, jd, obj, radius_m));
        p.map(|p| AtmoParams { height_m, ..p })
    }
}

fn body_air(data: &SeData, eph: &Ephemeris, jd: f64, obj: ObjectRef, radius_m: f64) -> Option<AtmoParams> {
    let ObjectRef::Body(i) = obj else { return None };
    let cat = &data.catalog;
    let b = cat.bodies.get(i as usize)?;
    let root = cat.system_root(obj)?;
    let lit = se_core::climate::system_stars(cat, root);
    let light = se_core::climate::star_light_from(eph, cat, obj, jd, &lit);
    let gravity = if radius_m > 0.0 && b.mass_kg > 0.0 { G * b.mass_kg / (radius_m * radius_m) } else { 9.81 };
    let completion = cat
        .climate
        .as_ref()
        .and_then(|t| se_core::climate::complete(t, &se_core::climate::catalog_body_in(cat, i as usize, &light)?));
    if let Some(c) = completion.filter(|c| c.pressure_atm > 0.0) {
        return Some(AtmoParams {
            p0_atm: c.pressure_atm as f64,
            t0_k: c.t_surf as f64,
            molar_mass: if c.molar_mass > 0.0 { c.molar_mass as f64 } else { AIR_MOLAR_MASS },
            gravity: if c.gravity > 0.0 { c.gravity as f64 } else { gravity },
            adiabat: DEFAULT_ADIABAT,
            height_m: 0.0,
        });
    }
    let extras = se_core::body_params::BodyExtras::from_body_block(&b.block);
    let atm = extras.atmosphere.filter(|a| !extras.no_atmosphere && a.pressure_atm > 0.0)?;
    let t = se_core::climate::equilibrium_temperature(light.flux_mean, b);
    Some(AtmoParams {
        p0_atm: atm.pressure_atm,
        t0_k: t.max(10.0),
        molar_mass: composition_molar_mass(atm.composition.iter().map(|(k, v)| (k.as_str(), *v))).unwrap_or(AIR_MOLAR_MASS),
        gravity,
        adiabat: if atm.adiabat > 0.0 { atm.adiabat } else { DEFAULT_ADIABAT },
        height_m: 0.0,
    })
}
