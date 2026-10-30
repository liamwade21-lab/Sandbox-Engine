//! SE's rings of generated bodies (FUN_140508600), run for every planet, dwarf planet, moon and
//! asteroid the system generator makes (FUN_140501620 / FUN_140502710 / FUN_140502eb0 /
//! FUN_140503780).
//!
//! - **Roll**: one stream-A draw against 0.6, or 0.05 for a tidally locked body (record +0x165c
//!   bit 0, `TidalLocked`). A body of more than 1e-7 Earth masses then has rings when its
//!   temperature is below `80 + 220 (log10 M + 5) / 4.4` K (80 K below 1e-5 Earth masses, 300 K
//!   above 0.25), so cold terrestrial planets, dwarf planets, moons and asteroids are ringed too.
//! - **Shape**: inner edge `U(1.1, 1.6) R + 20 km`, main edge `U(0.1, 2.3) R` beyond it; below
//!   300 K a ProbRingsElike (0.63) ring reaches out to `U(1.5, 3.5)` times that (like Saturn's E
//!   ring). Thickness `5e-6 U(0.25, 1)` of the mean radius (0.0125-25 km), rotation period at the
//!   ring's middle.
//! - **Look**: ProbRingsThin (0.2) thin, darker rings; ProbRingsSparse (0.3) sparse ones (100x the
//!   band frequency); narrow banding (x5 `densityPower`) with ProbRingsNarrow (0.2) for bodies of
//!   an Earth mass and up, always below 0.001 Earth masses; ProbRingsContrast (0.2) high contrast.
//!
//! The draws come from a stream of the body's own seeds; their place in SE's body stream (after the
//! body's other draws) is not replicated. SE's "debris" rings (ProbRingsDebris, bodies on circular
//! orbits made by its moon generator FUN_1404fdc90) are not generated.

use crate::surface_gen::SeRng;

/// main-def.cfg `PlanetGenerator`, matched to the exe globals through the config reader
/// (FUN_140280ec0): ProbRingsElike 0x14075f870, Thin 0x14075f86c, Narrow 0x14075f868, Sparse
/// 0x14075f864, Contrast 0x14075f860.
const PROB_RINGS_ELIKE: f32 = 0.63;
const PROB_RINGS_THIN: f32 = 0.2;
const PROB_RINGS_NARROW: f32 = 0.2;
const PROB_RINGS_SPARSE: f32 = 0.3;
const PROB_RINGS_CONTRAST: f32 = 0.2;
/// km to AU and the Earth/Sun mass ratio as SE stores them.
const AU_PER_KM: f32 = 6.684_587e-9;
const EARTH_SUN: f32 = 3.003_489_4e-6;

/// A generated body's `Rings` block, every key SE's generator fills.
#[derive(Debug, Clone, Default)]
pub(super) struct SeRings {
    pub inner_km: f32,
    pub outer_km: f32,
    pub edge_km: f32,
    pub mean_km: f32,
    pub chart_km: f32,
    pub thickness_km: f32,
    pub rocks_max_size_km: f32,
    pub rocks_spacing: f32,
    pub volum_draw_dist_km: f32,
    pub rotation_period_days: f32,
    pub brightness: f32,
    pub front_bright: f32,
    pub back_bright: f32,
    pub density: f32,
    pub opacity: f32,
    pub shadow: f32,
    pub self_shadow: f32,
    pub hapke: f32,
    pub spot_bright: f32,
    pub spot_width: f32,
    pub spot_bright_cb: f32,
    pub spot_width_cb: f32,
    pub frequency: f32,
    pub density_scale: f32,
    pub density_offset: f32,
    pub density_power: f32,
    pub color_contrast: f32,
}

/// SE's colours for rings that give none (FUN_140508600).
pub(super) const FRONT_COLOR: [f32; 3] = [0.7, 0.7, 0.7];
pub(super) const BACK_THICK_COLOR: [f32; 3] = [0.8, 0.6, 0.4];
pub(super) const BACK_ICE_COLOR: [f32; 3] = [0.3, 0.7, 1.0];
pub(super) const BACK_DUST_COLOR: [f32; 3] = [1.0, 0.98, 0.88];

/// Temperature (K) below which a body of `mass_me` Earth masses can have rings.
pub(super) fn ring_temp_limit(mass_me: f32) -> f32 {
    let lm = mass_me.log10();
    if lm < -5.0 {
        80.0
    } else if lm > -0.6 {
        300.0
    } else {
        (lm + 5.0) * 220.0 / 4.4 + 80.0
    }
}

/// SE's roll and, when it succeeds, the rings of a body of `mass_me` Earth masses, `radius_km`
/// and temperature `t_k`.
pub(super) fn generate(r: &mut SeRng, mass_me: f32, radius_km: f32, t_k: f32, locked: bool) -> Option<SeRings> {
    let p = if locked { 0.05 } else { 0.6 };
    if r.uniform(1.0) >= p || mass_me <= 1e-7 || t_k >= ring_temp_limit(mass_me) {
        return None;
    }
    let lm = mass_me.log10();
    let narrow = if lm < -3.0 {
        1.0
    } else if lm > 0.0 {
        PROB_RINGS_NARROW
    } else {
        (lm + 3.0) * (PROB_RINGS_NARROW - 1.0) / 3.0 + 1.0
    };
    let mut g = SeRings { inner_km: (r.uniform(0.5) + 1.1) * radius_km + 20.0, ..Default::default() };
    g.edge_km = (r.uniform(2.2) + 0.1) * radius_km + g.inner_km;
    g.outer_km = g.edge_km;
    if t_k < 300.0 && r.uniform(1.0) < PROB_RINGS_ELIKE {
        g.outer_km = (r.uniform(2.0) + 1.5) * g.edge_km;
    }
    g.mean_km = (g.edge_km - g.inner_km) * 0.333 + g.inner_km;
    g.thickness_km = ((r.uniform(0.75) + 0.25) * 5e-6 * g.mean_km).clamp(0.0125, 25.0);
    g.rocks_max_size_km = g.thickness_km * 0.04;
    g.rocks_spacing = 1.0;
    g.volum_draw_dist_km = (g.thickness_km * 3000.0).min(g.inner_km.min(20_000.0)).min((g.outer_km - g.inner_km) * 2.0);
    let a_au = (g.edge_km + g.inner_km) * 0.5 * AU_PER_KM;
    g.rotation_period_days = (a_au * a_au * a_au * 1.000_003 / mass_me / EARTH_SUN).sqrt() * 365.242_2;
    let thin = r.uniform(1.0) < PROB_RINGS_THIN;
    g.front_bright = (r.uniform(0.2) + 0.8) * 3.0;
    g.back_bright = if thin { r.uniform(0.3) + 0.3 } else { r.uniform(0.2) + 0.8 } * 5.0;
    g.density = if thin { r.uniform(0.35) + 0.05 } else { r.uniform(0.05) + 0.95 };
    g.opacity = g.density;
    g.shadow = g.density * if thin { 0.5 } else { 1.0 };
    g.self_shadow = g.shadow;
    g.brightness = 1.0;
    g.hapke = 1.0;
    g.spot_bright = if thin { r.uniform(0.3) + 0.7 } else { r.uniform(0.5) + 2.2 };
    g.spot_width = r.uniform(0.03) + 0.01;
    g.spot_width_cb = 0.001;
    g.chart_km = g.edge_km;
    let u_narrow = r.uniform(1.0);
    let sparse = r.uniform(1.0) < PROB_RINGS_SPARSE;
    let contrast = r.uniform(1.0) < PROB_RINGS_CONTRAST;
    g.frequency = r.tnormal(7.14, 0.2, 3.0, 10.0) * if sparse { 100.0 } else { 1.0 };
    g.density_scale = r.tnormal(1.5, 0.1, 1.0, 2.0) * if sparse { 1.5 } else { 1.0 };
    g.density_offset = -r.uniform(0.5) - if sparse { 1.4 } else { 0.0 };
    g.density_power = r.tnormal(1.0, 0.02, 0.5, 1.5) * if u_narrow < narrow { 5.0 } else { 1.0 };
    g.color_contrast = (r.uniform(0.05) + 0.05) * if contrast { 1.5 } else { 1.0 };
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_limit_follows_ses_line() {
        assert_eq!(ring_temp_limit(1e-6), 80.0);
        assert_eq!(ring_temp_limit(1.0), 300.0);
        // Earth's Moon (0.0123 Me): 80 + 220 (log10 0.0123 + 5) / 4.4 = 234 K.
        assert!((ring_temp_limit(0.0123) - 234.5).abs() < 0.5, "{}", ring_temp_limit(0.0123));
        assert!((ring_temp_limit(0.2512) - 300.0).abs() < 0.1);
    }

    #[test]
    fn roll_rates_and_shapes() {
        let (mut n, mut locked_n, mut elike, mut thin_dense) = (0, 0, 0, 0);
        for i in 0..4000u32 {
            let mut r = SeRng::new(i.wrapping_mul(0x9e37_79b9), i ^ 0x1234_5678);
            if let Some(g) = generate(&mut r, 1.0, 6371.0, 200.0, false) {
                n += 1;
                assert!(g.inner_km >= 1.1 * 6371.0 + 20.0 && g.inner_km < 1.6 * 6371.0 + 20.0);
                assert!(g.edge_km > g.inner_km && g.outer_km >= g.edge_km);
                assert!((0.0125..=25.0).contains(&g.thickness_km));
                assert!(g.volum_draw_dist_km <= 2.0 * (g.outer_km - g.inner_km) + 1e-3);
                elike += (g.outer_km > g.edge_km) as usize;
                thin_dense += (g.density < 0.5) as usize;
            }
            let mut r = SeRng::new(i.wrapping_mul(0x9e37_79b9), i ^ 0x1234_5678);
            locked_n += generate(&mut r, 1.0, 6371.0, 200.0, true).is_some() as usize;
        }
        let f = n as f64 / 4000.0;
        assert!((0.57..0.63).contains(&f), "ring share {f}");
        assert!((0.03..0.07).contains(&(locked_n as f64 / 4000.0)), "locked {locked_n}");
        let fe = elike as f64 / n as f64;
        assert!((0.58..0.68).contains(&fe), "E-like share {fe}");
        let ft = thin_dense as f64 / n as f64;
        assert!((0.15..0.25).contains(&ft), "thin share {ft}");
        // Too warm, or too small: none.
        let mut r = SeRng::new(1, 2);
        assert!((0..100).all(|_| generate(&mut r, 1.0, 6371.0, 310.0, false).is_none()));
        assert!((0..100).all(|_| generate(&mut r, 1e-8, 1.0, 20.0, false).is_none()));
    }
}
