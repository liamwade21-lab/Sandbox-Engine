//! Planetary ring parameters from SpaceEngine `Rings { ... }` blocks.
//!
//! Format and key meanings are documented in `docs/formats/rings.md`. Defaults
//! here are port choices for absent keys, not engine values.

use crate::sc::Block;

/// Parameters of one `Rings` block. Radii are in km; `Option` fields mean
/// "not set in the catalog".
#[derive(Debug, Clone, PartialEq)]
pub struct RingsParams {
    /// Texture stem as written in the catalog, e.g. `Saturn-rings.*`.
    pub texture: Option<String>,
    pub inner_radius_km: f64,
    pub outer_radius_km: f64,
    pub mean_radius_km: Option<f64>,
    pub edge_radius_km: Option<f64>,
    pub chart_radius_km: Option<f64>,
    pub thickness_km: f64,
    pub rocks_max_size_km: f64,
    pub rocks_spacing: f64,
    /// `VolumDrawDist` (km): the 3D rings are drawn within this distance of the ring volume.
    pub volum_draw_dist_km: f64,
    pub density: f64,
    pub opacity: f64,
    pub self_shadow: f64,
    pub planet_shadow: f64,
    pub brightness: f64,
    pub front_bright: f64,
    pub back_bright: f64,
    pub hapke: f64,
    pub spot_bright: f64,
    pub spot_width: f64,
    pub spot_bright_cb: f64,
    pub spot_width_cb: f64,
    pub rotation_period: Option<f64>,
    pub frequency: f64,
    pub density_scale: f64,
    pub density_offset: f64,
    pub density_power: f64,
    pub color_contrast: f64,
    pub front_color: Option<[f64; 3]>,
    pub back_thick_color: Option<[f64; 3]>,
    pub back_ice_color: Option<[f64; 3]>,
    pub back_dust_color: Option<[f64; 3]>,
}

impl Default for RingsParams {
    fn default() -> Self {
        Self {
            texture: None,
            inner_radius_km: 0.0,
            outer_radius_km: 0.0,
            mean_radius_km: None,
            edge_radius_km: None,
            chart_radius_km: None,
            thickness_km: 0.0,
            rocks_max_size_km: 0.0,
            rocks_spacing: 0.0,
            volum_draw_dist_km: 0.0,
            density: 1.0,
            opacity: 1.0,
            self_shadow: 1.0,
            planet_shadow: 1.0,
            brightness: 1.0,
            front_bright: 1.0,
            back_bright: 1.0,
            hapke: 1.0,
            spot_bright: 0.0,
            spot_width: 0.0,
            spot_bright_cb: 0.0,
            spot_width_cb: 0.0,
            rotation_period: None,
            frequency: 0.0,
            density_scale: 1.0,
            density_offset: 0.0,
            density_power: 1.0,
            color_contrast: 0.0,
            front_color: None,
            back_thick_color: None,
            back_ice_color: None,
            back_dust_color: None,
        }
    }
}

impl RingsParams {
    /// Build from the body of a `Rings` block (the `{ ... }` contents, i.e.
    /// `root.sub("Rings")`). Keys are case-insensitive; missing keys take defaults.
    pub fn from_block(b: &Block) -> Self {
        let d = Self::default();
        Self {
            texture: b.str("Texture").map(str::to_owned),
            inner_radius_km: b.f64("InnerRadius").unwrap_or(d.inner_radius_km),
            outer_radius_km: b.f64("OuterRadius").unwrap_or(d.outer_radius_km),
            mean_radius_km: b.f64("MeanRadius"),
            edge_radius_km: b.f64("EdgeRadius"),
            chart_radius_km: b.f64("ChartRadius"),
            thickness_km: b.f64("Thickness").unwrap_or(d.thickness_km),
            rocks_max_size_km: b.f64("RocksMaxSize").unwrap_or(d.rocks_max_size_km),
            rocks_spacing: b.f64("RocksSpacing").unwrap_or(d.rocks_spacing),
            volum_draw_dist_km: b.f64("VolumDrawDist").unwrap_or(d.volum_draw_dist_km),
            density: b.f64("Density").unwrap_or(d.density),
            opacity: b.f64("Opacity").unwrap_or(d.opacity),
            self_shadow: b.f64("SelfShadow").unwrap_or(d.self_shadow),
            planet_shadow: b.f64("PlanetShadow").unwrap_or(d.planet_shadow),
            brightness: b.f64("Brightness").unwrap_or(d.brightness),
            front_bright: b.f64("FrontBright").unwrap_or(d.front_bright),
            back_bright: b.f64("BackBright").unwrap_or(d.back_bright),
            hapke: b.f64("Hapke").unwrap_or(d.hapke),
            spot_bright: b.f64("SpotBright").unwrap_or(d.spot_bright),
            spot_width: b.f64("SpotWidth").unwrap_or(d.spot_width),
            spot_bright_cb: b.f64("SpotBrightCB").unwrap_or(d.spot_bright_cb),
            spot_width_cb: b.f64("SpotWidthCB").unwrap_or(d.spot_width_cb),
            rotation_period: b.f64("RotationPeriod"),
            frequency: b.f64("frequency").unwrap_or(d.frequency),
            density_scale: b.f64("densityScale").unwrap_or(d.density_scale),
            density_offset: b.f64("densityOffset").unwrap_or(d.density_offset),
            density_power: b.f64("densityPower").unwrap_or(d.density_power),
            color_contrast: b.f64("colorContrast").unwrap_or(d.color_contrast),
            front_color: b.vec3("FrontColor"),
            back_thick_color: b.vec3("BackThickColor"),
            back_ice_color: b.vec3("BackIceColor"),
            back_dust_color: b.vec3("BackDustColor"),
        }
    }

    /// The ring volume as SE fills the keys a block leaves out (FUN_140508600, the same rules the
    /// generated rings follow, `procgen::se_rings`): `Thickness` (half the slab, km) `5e-6 U(0.25, 1)`
    /// of the mean radius (0.0125-25 km), `RocksMaxSize` 0.04 of it, `RocksSpacing` 1, `VolumDrawDist`
    /// the least of 20000 km, the inner radius, 3000 thicknesses and twice the ring's width, and the
    /// `RotationPeriod` (hours in the catalog, days here) the Kepler period halfway to the edge.
    /// `u` is the draw in [0, 1) SE takes for the thickness; `mass_earth` the planet's mass.
    pub fn volume(&self, mass_earth: f64, u: f64) -> RingVolume {
        let (inner, outer) = (self.inner_radius_km, self.outer_radius_km);
        let edge = self.edge_radius_km.unwrap_or(outer);
        let mean = self.mean_radius_km.unwrap_or((edge - inner) * 0.333 + inner);
        let thickness = if self.thickness_km > 0.0 { self.thickness_km } else { ((u * 0.75 + 0.25) * 5e-6 * mean).clamp(0.0125, 25.0) };
        let rocks = if self.rocks_max_size_km > 0.0 { self.rocks_max_size_km } else { thickness * 0.04 };
        let spacing = if self.rocks_spacing > 0.0 { self.rocks_spacing } else { 1.0 };
        let draw = if self.volum_draw_dist_km > 0.0 {
            self.volum_draw_dist_km
        } else {
            (thickness * 3000.0).min(inner.min(20_000.0)).min((outer - inner) * 2.0)
        };
        let period = match self.rotation_period {
            Some(h) if h != 0.0 => h / 24.0,
            _ => kepler_period_days((edge + inner) * 0.5, mass_earth),
        };
        RingVolume { thickness_km: thickness, rocks_max_size_km: rocks, rocks_spacing: spacing, volum_draw_dist_km: draw, rotation_period_days: period }
    }

    /// VFS path of the ring texture, relative to the data dir, e.g.
    /// `textures/planets/Saturn-rings.png`. The catalog's `.*` wildcard becomes
    /// `.png`, the only extension in Rings.pak. Returns `None` without a `Texture` key.
    pub fn texture_path(&self) -> Option<String> {
        let stem = self.texture.as_deref()?;
        let file = match stem.strip_suffix(".*") {
            Some(base) => format!("{base}.png"),
            None => stem.to_owned(),
        };
        Some(format!("textures/planets/{file}"))
    }
}

/// The ring volume's values ([`RingsParams::volume`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingVolume {
    pub thickness_km: f64,
    pub rocks_max_size_km: f64,
    pub rocks_spacing: f64,
    pub volum_draw_dist_km: f64,
    pub rotation_period_days: f64,
}

/// SE's orbital period at `r_km` around a planet of `mass_earth` (FUN_140508600 and the subring
/// rates of FUN_1402e9c50): `sqrt(a^3 x 1.000003 / M)` years, `a` in AU, `M` in solar masses
/// (Earth masses x 3.0034894e-6).
pub fn kepler_period_days(r_km: f64, mass_earth: f64) -> f64 {
    let a = r_km * 3.240_779_289_444_365e-14 / 4.848_136_811_095_36e-6;
    (a * a * a * 1.000_003_003_489_382_5 / (mass_earth.max(1e-12) * 3.003_489_382_535_917_7e-6)).sqrt() * 365.242_189_85
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc::parse;

    fn rings_block(src: &str) -> Block {
        parse(src).sub("Rings").expect("Rings block").clone()
    }

    #[test]
    fn saturn_block_parses_all_keys() {
        let b = rings_block(
            r#"Rings
{
    // Rings texture author: Sean Young
    Texture "Saturn-rings.*"
    InnerRadius 64535
    OuterRadius 318120
    MeanRadius 106000 // radius of the most denser part
    EdgeRadius 137000
    ChartRadius 137000
    Thickness 0.15 // km
    RocksMaxSize 0.008 // km
    Brightness 1.0
    FrontBright 2.0
    BackBright 5.0 // 0.15
    Density 1.0
    Opacity 1.0
    SelfShadow 1.0
    PlanetShadow 1.0
    Hapke 1.0
    SpotBright 0.45
    SpotWidth 0.015
    SpotBrightCB 1.95
    SpotWidthCB 0.00245
}"#,
        );
        let r = RingsParams::from_block(&b);
        assert_eq!(r.texture.as_deref(), Some("Saturn-rings.*"));
        assert_eq!(r.inner_radius_km, 64535.0);
        assert_eq!(r.outer_radius_km, 318120.0);
        assert_eq!(r.mean_radius_km, Some(106000.0));
        assert_eq!(r.edge_radius_km, Some(137000.0));
        assert_eq!(r.chart_radius_km, Some(137000.0));
        assert_eq!(r.thickness_km, 0.15);
        assert_eq!(r.rocks_max_size_km, 0.008);
        assert_eq!(r.back_bright, 5.0);
        assert_eq!(r.front_bright, 2.0);
        assert_eq!(r.spot_bright_cb, 1.95);
        assert_eq!(r.spot_width_cb, 0.00245);
        assert_eq!(r.rotation_period, None);
        assert_eq!(r.front_color, None);
        // Saturn's volume: its own thickness and rocks, SE's spacing and draw distance (3000 x 0.15 km),
        // and the Kepler period halfway to the edge (100,770 km: 2 pi sqrt(r^3 / GM) = 9.1 hours).
        let v = r.volume(95.16, 0.5);
        assert_eq!((v.thickness_km, v.rocks_max_size_km, v.rocks_spacing), (0.15, 0.008, 1.0));
        assert!((v.volum_draw_dist_km - 450.0).abs() < 1e-9);
        assert!((v.rotation_period_days * 24.0 - 9.06).abs() < 0.05, "{}", v.rotation_period_days);
    }

    #[test]
    fn missing_volume_keys_follow_se() {
        let b = rings_block("Rings {
 InnerRadius 1000
 OuterRadius 1200
 VolumDrawDist 50
 RotationPeriod 12
}
");
        let v = RingsParams::from_block(&b).volume(1.0, 1.0);
        // Mean radius 1000 + 0.333 x 200 = 1066.6 km: thickness 5e-6 x mean, at least 0.0125 km.
        assert_eq!(v.thickness_km, 0.0125);
        assert_eq!(v.rocks_max_size_km, 0.0125 * 0.04);
        assert_eq!((v.volum_draw_dist_km, v.rotation_period_days), (50.0, 0.5));
        // The Moon's distance around the Earth: about 27.3 days.
        assert!((kepler_period_days(384_400.0, 1.0 + 1.0 / 81.3) - 27.3).abs() < 0.2);
    }

    #[test]
    fn exponent_numbers_and_defaults() {
        // Jupiter-style: only some keys present, exponent notation.
        let b = rings_block(
            "Rings {\n InnerRadius 9e4\n OuterRadius 4e6\n Density 1e-4\n Opacity 1e-4\n}\n",
        );
        let r = RingsParams::from_block(&b);
        assert_eq!(r.inner_radius_km, 9e4);
        assert_eq!(r.outer_radius_km, 4e6);
        assert_eq!(r.density, 1e-4);
        assert_eq!(r.opacity, 1e-4);
        assert_eq!(r.texture, None);
        assert_eq!(r.texture_path(), None);
        // Absent keys fall back to the documented defaults.
        assert_eq!(r.self_shadow, 1.0);
        assert_eq!(r.hapke, 1.0);
        assert_eq!(r.spot_bright, 0.0);
        assert_eq!(r.edge_radius_km, None);
    }

    #[test]
    fn procedural_keys_are_case_insensitive() {
        let b = rings_block(
            "Rings {\n frequency 157.93\n densityScale 3.2061\n densityOffset -1.4504\n densityPower 2.5385\n colorContrast 0\n}\n",
        );
        let r = RingsParams::from_block(&b);
        assert_eq!(r.frequency, 157.93);
        assert_eq!(r.density_scale, 3.2061);
        assert_eq!(r.density_offset, -1.4504);
        assert_eq!(r.density_power, 2.5385);
        assert_eq!(r.color_contrast, 0.0);
    }

    #[test]
    fn colour_tuples_with_and_without_commas() {
        let b = rings_block(
            "Rings {\n FrontColor (0.700 0.700 0.700)\n BackThickColor ( 0.800 , 0.600 , 0.400 )\n BackIceColor (0.300 0.700 1.000)\n BackDustColor (1.000 0.980 0.880)\n}\n",
        );
        let r = RingsParams::from_block(&b);
        assert_eq!(r.front_color, Some([0.7, 0.7, 0.7]));
        assert_eq!(r.back_thick_color, Some([0.8, 0.6, 0.4]));
        assert_eq!(r.back_ice_color, Some([0.3, 0.7, 1.0]));
        assert_eq!(r.back_dust_color, Some([1.0, 0.98, 0.88]));
    }

    #[test]
    fn texture_path_maps_wildcard_to_png() {
        let b = rings_block("Rings {\n Texture \"Chariklo-rings.*\"\n InnerRadius 387.1\n}\n");
        let r = RingsParams::from_block(&b);
        assert_eq!(
            r.texture_path().as_deref(),
            Some("textures/planets/Chariklo-rings.png")
        );

        let explicit = RingsParams {
            texture: Some("Custom-rings.dds".to_owned()),
            ..RingsParams::default()
        };
        assert_eq!(
            explicit.texture_path().as_deref(),
            Some("textures/planets/Custom-rings.dds")
        );
    }

    #[test]
    fn commented_out_texture_is_ignored() {
        let b = rings_block("Rings {\n //Texture \"Haumea-rings.*\"\n InnerRadius 2252\n OuterRadius 2322\n}\n");
        let r = RingsParams::from_block(&b);
        assert_eq!(r.texture, None);
        assert_eq!(r.inner_radius_km, 2252.0);
        assert_eq!(r.outer_radius_km, 2322.0);
    }
}
