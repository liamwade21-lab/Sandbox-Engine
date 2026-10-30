//! Inputs of SE's terrain generator shaders (`tg_terra_height.glsl`, `tg_selena_height.glsl`, ported
//! to `tg_height.wesl`): the 64^3 noise texture and the uniform block, as the exe builds them.
//! docs/formats/se_shaders.md "Terrain generator".

use crate::surface_gen::{self as sg, SurfaceGen};

/// Side of SE's `Noise3DSampler` (`NOISE_TEX_3D_SIZE`).
pub const NOISE3D_SIZE: usize = 64;

/// SE's `Noise3DSampler`: 64^3 RGBA8 texels (x fastest), each byte `u / 255 + u` (low 8 bits) of
/// the next state `u` of SE's generator A (`u = u * 0x19660d + 0x3c6ef35f`, thread slot +0x24).
/// FUN_1402f0880 seeds it with 0x7178d, then fills, in order: a 512^2 RGBA noise map, a 512^2 R
/// map, a 64^2 R map, a 256^2 and a 16^2 map with two draws per texel, then this texture (which
/// the height generator FUN_1402fab70 binds to unit 0, nearest).
pub fn noise3d() -> Vec<u8> {
    let mut u: u32 = 0x7178d;
    let mut next = || {
        u = u.wrapping_mul(0x19660d).wrapping_add(0x3c6e_f35f);
        ((u / 255) as u8).wrapping_add(u as u8)
    };
    let skipped = 512 * 512 * 4 + 512 * 512 + 64 * 64 + 256 * 256 * 2 + 16 * 16 * 2;
    for _ in 0..skipped {
        next();
    }
    (0..NOISE3D_SIZE * NOISE3D_SIZE * NOISE3D_SIZE * 4).map(|_| next()).collect()
}

/// Which of SE's height generators a body gets (FUN_1402ef870): `Some(0)` `tg_terra`, `Some(1)`
/// `tg_selena` (rocky bulk classes, by whether the atmosphere class is below 2), `Some(2)`
/// `tg_asteroid`, `None` for the others (giants, stars), which have their own shaders.
pub fn generator(g: &SurfaceGen) -> Option<u32> {
    match g.classes.bulk {
        sg::bulk::ASTEROID => Some(2),
        1..=4 => Some(if g.classes.atmo < 2 { 1 } else { 0 }),
        _ => None,
    }
}

/// SE's generator uniforms in its block order (FUN_1402fab70 copies them from the body record):
/// Randomize, mainParams, colorParams, climateParams, plantsParams1, plantsParams2, mareParams,
/// montesParams, dunesParams, hillsParams, canyonsParams, riversParams, riftsParams,
/// eqridgeParams, cracksParams, craterParams, volcanoParams1, volcanoParams2, lavaParams,
/// textureParams. `tidal_lock`: the record's "locked to a star" flag (+0x165c bit 2).
pub fn uniforms(g: &SurfaceGen, tidal_lock: bool) -> [[f32; 4]; 20] {
    let v = |i: usize| g.v(i);
    let [dunes, hills, hills2, canyons, montes] = sg::landform_thresholds(g).unwrap_or([0.0; 5]);
    // 1 for a global ocean (Superoceanic), 0.5 for the other volatile classes past Arid.
    let ocean_type = match g.classes.volq {
        5 => 1.0,
        q if q > 1 => 0.5,
        _ => 0.0,
    };
    [
        [v(sg::RANDOMIZE), v(sg::RANDOMIZE + 1), v(sg::RANDOMIZE + 2), 0.0],
        [v(sg::MAIN_FREQ), v(sg::TERRACE_PROB), ocean_type, if tidal_lock { 1.0 } else { -1.0 }],
        [v(sg::COLOR_DIST_MAGN), v(sg::COLOR_DIST_FREQ), v(sg::ICECAP_LATITUDE), v(sg::TROPIC_LATITUDE)],
        [v(sg::CLIMATE_POLE), v(sg::CLIMATE_TROPIC), v(sg::CLIMATE_EQUATOR), v(sg::TROPIC_WIDTH)],
        [v(sg::CLIMATE_GRASS_MIN), v(sg::CLIMATE_GRASS_MAX), v(sg::CLIMATE_FOREST_MIN), v(sg::CLIMATE_FOREST_MAX)],
        [v(sg::CLIMATE_STEPPE_MIN), v(sg::CLIMATE_STEPPE_MAX), v(sg::HUMIDITY), 0.0],
        [v(sg::SEA_LEVEL), v(sg::MARE_FREQ), v(sg::MARE_DENSITY).max(0.0).sqrt(), v(sg::ICECAP_HEIGHT)],
        [v(sg::MONTES_MAGN), v(sg::MONTES_FREQ), montes, v(sg::MONTES_SPIKY)],
        [v(sg::DUNES_MAGN), v(sg::DUNES_FREQ), dunes, v(sg::DRIVEN_DARKENING)],
        [v(sg::HILLS_MAGN), v(sg::HILLS_FREQ), hills, hills2],
        [v(sg::CANYONS_MAGN), v(sg::CANYONS_FREQ), canyons, v(sg::EROSION)],
        [v(sg::RIVERS_MAGN), v(sg::RIVERS_FREQ), v(sg::RIVERS_SIN), v(sg::BEACH_WIDTH)],
        [v(sg::RIFTS_MAGN), v(sg::RIFTS_FREQ), v(sg::RIFTS_SIN), 0.0],
        [v(sg::EQRIDGE_MAGN), v(sg::EQRIDGE_WIDTH), v(sg::EQRIDGE_MOD_MAGN), v(sg::EQRIDGE_MOD_FREQ)],
        [v(sg::CRACKS_MAGN), v(sg::CRACKS_FREQ), v(sg::CRACKS_OCTAVES), v(sg::CRATER_RAYED_FACTOR)],
        [v(sg::CRATER_MAGN), v(sg::CRATER_FREQ), v(sg::CRATER_DENSITY).max(0.0).sqrt(), v(sg::CRATER_OCTAVES)],
        [v(sg::VOLCANO_MAGN), v(sg::VOLCANO_FREQ), v(sg::VOLCANO_DENSITY), v(sg::VOLCANO_OCTAVES)],
        [v(sg::VOLCANO_ACTIVITY), v(sg::VOLCANO_FLOWS), v(sg::VOLCANO_RADIUS), v(sg::VOLCANO_TEMP) * 0.001],
        [v(sg::LAVA_LEVEL).clamp(0.0, 1.0), v(sg::SNOW_LEVEL), 0.0, v(sg::HEIGHT_TEMP_GRAD)],
        [v(sg::DETAIL_SCALE), 0.0, v(sg::VENUS_MAGN), v(sg::VENUS_FREQ)],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_texture_is_fixed() {
        let a = noise3d();
        assert_eq!(a.len(), 64 * 64 * 64 * 4);
        assert_eq!(a, noise3d());
        // Uniform bytes: the mean is near 127.5.
        let mean = a.iter().map(|&b| b as f64).sum::<f64>() / a.len() as f64;
        assert!((mean - 127.5).abs() < 1.0, "{mean}");
    }
}
