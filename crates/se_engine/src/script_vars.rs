//! Script access to the engine's settings (SE's `Set`, `SetU`, `SetForce`, `Get`, `Reset`,
//! `Interpolate`, `WaitVar`, `SaveVars`, `RestoreVars`: FUN_14055ac90 .. FUN_14055a4f0).
//!
//! SE's script variables are its config variables (registered with FUN_1405a4a00), so a script
//! reads and writes the same values as the settings dialogs, and `Reset` (FUN_14055a8d0 ->
//! FUN_14054f320) puts one back to its registered default. [`Settings`] maps the names the
//! shipped scripts use (`tutorials/DefaultSettings.se`, `menu/Scene1.se` and the tutorials) onto
//! the port's own settings; names with no counterpart in the port stay script variables (kept
//! by the runner, readable by `if`/`WaitVar`).
//!
//! `LessonPassed` (written with `SetForce` at the end of each tutorial) is SE's saved tutorial
//! progress; the port keeps it in its user folder (`script-vars.cfg`), except in test runs.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::bloom::BloomSettings;
use crate::camera::{FlyCamera, MainCamera};
use crate::clusters::ClusterSettings;
use crate::color_correction::ColorCorrection;
use crate::exposure::{ExposureComp, PhotoMode};
use crate::graphics::GraphicsSettings;
use crate::lensflare::LensFlareSettings;
use crate::overlay::{MarkerKind, OverlaySettings, SkySwitches};
use crate::procgalaxies::ProcSettings;
use crate::stars::{PointStyle, StarSettings};
use crate::switches::RenderSwitches;
use crate::universe::SimTime;
use crate::waypoints::Waypoints;

/// The settings a script can reach.
#[derive(SystemParam)]
pub struct Settings<'w, 's> {
    pub stars: ResMut<'w, StarSettings>,
    pub overlay: ResMut<'w, OverlaySettings>,
    pub bloom: ResMut<'w, BloomSettings>,
    pub lens: ResMut<'w, LensFlareSettings>,
    pub gfx: ResMut<'w, GraphicsSettings>,
    pub proc: ResMut<'w, ProcSettings>,
    pub waypoints: ResMut<'w, Waypoints>,
    /// Inserted at startup by their plugins.
    pub clusters: Option<ResMut<'w, ClusterSettings>>,
    pub exposure: Option<ResMut<'w, ExposureComp>>,
    /// SE's `PhotoMode` (exposure.rs).
    pub photo: Option<ResMut<'w, PhotoMode>>,
    pub switches: Option<ResMut<'w, RenderSwitches>>,
    /// SE's `Brightness`/`Contrast`/`Saturation`/`Gamma` (FUN_140326220) live on the camera.
    pub color: Query<'w, 's, &'static mut ColorCorrection, With<MainCamera>>,
}

/// Names (lower case) of the scalar settings mapped below, for `SaveVars`.
pub const SCALARS: &[&str] = &[
    "showorbits",
    "labelsmode",
    "showmarkers",
    "starmagnlimit",
    "galaxymagnlimit",
    "nebulamagnlimit",
    "glowmagnlimit",
    "pointoverbright",
    "pointscale",
    "pointdesaturation",
    "starcolorsaturation",
    "starcolorshift",
    "sunillumsaturation",
    "illumsaturationauto",
    "starpointmethod",
    "motionblurlength",
    "timescale",
    "fov",
    "autohorizon",
    "showselectionpointer",
    "fps",
    "textmode",
    "starnamemagn",
    "enablebloom",
    "bloombright",
    "bloomthreshold",
    "enablelensflares",
    "enablelensghosts",
    "enablepseudoflares",
    "lensflarebright",
    "lensflarescale",
    "pseudoflarebright",
    "landlod",
    "landtargetres",
    "showprocgalaxies",
    "showprocclusters",
    "showprocnebulae",
    "showcatclusters",
    "exposurecomp",
    "photomode",
    "showatmospheres",
    "showclouds",
    "showaurora",
    "showcomettails",
    "showeclipsemask",
    "enableeclipses",
    "atmoringsshadow",
    "atmoextinction",
    "enableplanetshine",
    "atmominwidthpix",
    "atmodistancelimit",
    "aurorabright",
    "brightness",
    "contrast",
    "saturation",
    "gamma",
    "waypointsopacity",
    "showlabelswaypoints",
];

/// The per-kind filters `ShowOrbits<Kind>`, `ShowLabels<Kind>`, `ShowMarkers<Kind>` (lower case).
pub fn kind_filter_names() -> impl Iterator<Item = String> {
    ["showorbits", "showlabels", "showmarkers"]
        .into_iter()
        .flat_map(|p| MarkerKind::ALL.into_iter().map(move |k| format!("{p}{}", k.key().to_ascii_lowercase())))
}

fn b(v: bool) -> f64 {
    v as u8 as f64
}

/// Read-only access to every mapped setting: the live ones or the port's defaults.
struct View<'a> {
    stars: &'a StarSettings,
    overlay: &'a OverlaySettings,
    bloom: &'a BloomSettings,
    lens: &'a LensFlareSettings,
    gfx: &'a GraphicsSettings,
    proc: &'a ProcSettings,
    wps: &'a Waypoints,
    clusters: Option<&'a ClusterSettings>,
    exposure: Option<&'a ExposureComp>,
    photo: Option<&'a PhotoMode>,
    switches: Option<&'a RenderSwitches>,
    color: Option<&'a ColorCorrection>,
    cam: &'a FlyCamera,
    sim: &'a SimTime,
}

impl View<'_> {
    fn get(&self, k: &str) -> Option<f64> {
        let (s, o) = (self.stars, self.overlay);
        // SE's `bcsg`: brightness - 1, contrast, saturation - 1, 1 / gamma.
        let cc = |f: fn(Vec4) -> f32| self.color.map(|c| f(c.bcsg) as f64);
        Some(match k {
            "showorbits" => b(o.orbits),
            "labelsmode" => o.labels_mode as f64,
            "showmarkers" => b(o.markers),
            "starmagnlimit" => s.mag_limit as f64,
            "galaxymagnlimit" => s.galaxy_mag_limit as f64,
            "nebulamagnlimit" => s.nebula_mag_limit as f64,
            "glowmagnlimit" => s.glow_mag_limit as f64,
            "pointoverbright" => s.overbright as f64,
            "pointscale" => s.point_scale as f64,
            "pointdesaturation" => s.desaturation as f64,
            "starcolorsaturation" => s.saturation as f64,
            "starcolorshift" => s.color_shift as f64,
            "sunillumsaturation" => s.illum_saturation as f64,
            "illumsaturationauto" => b(s.illum_saturation_auto),
            "starpointmethod" => s.point_style.method() as f64,
            "motionblurlength" => s.blur_length as f64,
            "timescale" => self.sim.rate,
            "fov" => self.cam.fov_deg as f64,
            "autohorizon" => b(self.cam.auto_horizon),
            "showselectionpointer" => b(o.selection_pointer),
            "fps" => b(o.show_fps),
            "textmode" => if o.info_full { 2.0 } else { 1.0 },
            "starnamemagn" => o.star_label_mag as f64,
            "enablebloom" => b(self.bloom.enabled),
            "bloombright" => self.bloom.bright as f64,
            "bloomthreshold" => self.bloom.threshold as f64,
            "enablelensflares" => b(self.lens.flares),
            "enablelensghosts" => b(self.lens.ghosts),
            "enablepseudoflares" => b(self.lens.pseudo),
            "lensflarebright" => self.lens.bright as f64,
            "lensflarescale" => self.lens.scale as f64,
            "pseudoflarebright" => self.lens.pseudo_bright as f64,
            "landlod" => self.gfx.quality.land_lod as f64,
            "landtargetres" => self.gfx.quality.land_target_res as f64,
            "showprocgalaxies" => b(self.proc.galaxies),
            "showprocclusters" => b(self.proc.clusters),
            "showprocnebulae" => b(self.proc.nebulae),
            "showcatclusters" => b(self.clusters?.show_cat),
            // SE keeps ExposureComp negated: +1 shown (twice as bright) is -1 (exposure.rs).
            "exposurecomp" => -self.exposure?.stops as f64,
            "photomode" => self.photo?.0 as f64,
            "showatmospheres" => b(self.switches?.atmospheres),
            "showclouds" => b(self.switches?.clouds),
            "showaurora" => b(self.switches?.aurora),
            "showcomettails" => b(self.switches?.comet_tails),
            "showeclipsemask" => b(self.switches?.eclipse_mask),
            "enableeclipses" => b(self.switches?.eclipses),
            "atmoringsshadow" => b(self.switches?.sky_rings_shadows),
            "atmoextinction" => b(self.switches?.atmo_extinction),
            "enableplanetshine" => b(self.switches?.planet_shine),
            "atmominwidthpix" => self.switches?.atmo_min_width_px as f64,
            "atmodistancelimit" => self.switches?.atmo_distance_limit as f64,
            "aurorabright" => self.switches?.aurora_bright as f64,
            "brightness" => cc(|v| v.x + 1.0)?,
            "contrast" => cc(|v| v.y)?,
            "saturation" => cc(|v| v.z + 1.0)?,
            "gamma" => cc(|v| 1.0 / v.w.max(1e-6))?,
            "waypointsopacity" => self.wps.opacity as f64,
            "showlabelswaypoints" => b(self.wps.labels),
            f => match o.sky.var(f) {
                Some(on) => b(on),
                None => kind_filter_get(o, f)?,
            },
        })
    }
}

impl Settings<'_, '_> {
    fn view<'a>(&'a self, cam: &'a FlyCamera, sim: &'a SimTime) -> View<'a> {
        View {
            stars: &self.stars,
            overlay: &self.overlay,
            bloom: &self.bloom,
            lens: &self.lens,
            gfx: &self.gfx,
            proc: &self.proc,
            wps: &self.waypoints,
            clusters: self.clusters.as_deref(),
            exposure: self.exposure.as_deref(),
            photo: self.photo.as_deref(),
            switches: self.switches.as_deref(),
            color: self.color.single().ok(),
            cam,
            sim,
        }
    }

    /// A setting's value by its SE name (any case); `None` when the port has no such setting.
    pub fn get(&self, cam: &FlyCamera, sim: &SimTime, name: &str) -> Option<f64> {
        self.view(cam, sim).get(&name.to_ascii_lowercase())
    }

    /// Sets a setting by its SE name; false when the port has no such setting.
    pub fn set(&mut self, cam: &mut FlyCamera, sim: &mut SimTime, name: &str, v: f64) -> bool {
        let k = name.to_ascii_lowercase();
        let on = v != 0.0;
        let (s, o) = (&mut *self.stars, &mut *self.overlay);
        match k.as_str() {
            "showorbits" => o.orbits = on,
            "labelsmode" => o.labels_mode = v.clamp(0.0, 2.0) as u8,
            "showmarkers" => o.markers = on,
            "starmagnlimit" => s.mag_limit = v as f32,
            "galaxymagnlimit" => s.galaxy_mag_limit = v as f32,
            "nebulamagnlimit" => s.nebula_mag_limit = v as f32,
            "glowmagnlimit" => s.glow_mag_limit = v as f32,
            "pointoverbright" => s.overbright = v as f32,
            "pointscale" => s.point_scale = v as f32,
            "pointdesaturation" => s.desaturation = v as f32,
            "starcolorsaturation" => s.saturation = v as f32,
            "starcolorshift" => s.color_shift = v as f32,
            "sunillumsaturation" => s.illum_saturation = v as f32,
            "illumsaturationauto" => s.illum_saturation_auto = on,
            "starpointmethod" => s.point_style = PointStyle::from_method(v.max(0.0) as u32),
            "motionblurlength" => s.blur_length = v as f32,
            "timescale" => sim.rate = v,
            "fov" => cam.fov_deg = (v as f32).clamp(crate::camera::MIN_FOV_DEG, crate::camera::MAX_FOV_DEG),
            "autohorizon" => cam.auto_horizon = on,
            "showselectionpointer" => o.selection_pointer = on,
            "fps" => o.show_fps = on,
            "textmode" => o.info_full = v >= 2.0,
            "starnamemagn" => o.star_label_mag = v as f32,
            "enablebloom" => self.bloom.enabled = on,
            "bloombright" => self.bloom.bright = v as f32,
            "bloomthreshold" => self.bloom.threshold = v as f32,
            "enablelensflares" => self.lens.flares = on,
            "enablelensghosts" => self.lens.ghosts = on,
            "enablepseudoflares" => self.lens.pseudo = on,
            "lensflarebright" => self.lens.bright = v as f32,
            "lensflarescale" => self.lens.scale = v as f32,
            "pseudoflarebright" => self.lens.pseudo_bright = v as f32,
            "landlod" => self.gfx.quality.land_lod = v as f32,
            "landtargetres" => self.gfx.quality.land_target_res = v as f32,
            "showprocgalaxies" => self.proc.galaxies = on,
            "showprocclusters" => self.proc.clusters = on,
            "showprocnebulae" => self.proc.nebulae = on,
            "showcatclusters" => match self.clusters.as_mut() {
                Some(c) => c.show_cat = on,
                None => return false,
            },
            "exposurecomp" => match self.exposure.as_mut() {
                Some(e) => e.set(-v as f32),
                None => return false,
            },
            "photomode" => match self.photo.as_mut() {
                Some(m) => **m = PhotoMode::new(v.clamp(0.0, 2.0) as u8),
                None => return false,
            },
            // switches.rs (rings, skies and eclipses): the planet drawing switches.
            "showatmospheres" | "showclouds" | "showaurora" | "showcomettails" | "showeclipsemask" | "enableeclipses" | "atmoringsshadow"
            | "atmoextinction" | "enableplanetshine" | "atmominwidthpix" | "atmodistancelimit" | "aurorabright" => {
                let Some(sw) = self.switches.as_mut() else { return false };
                match k.as_str() {
                    "showatmospheres" => sw.atmospheres = on,
                    "showclouds" => sw.clouds = on,
                    "showaurora" => sw.aurora = on,
                    "showcomettails" => sw.comet_tails = on,
                    "showeclipsemask" => sw.eclipse_mask = on,
                    "enableeclipses" => sw.eclipses = on,
                    "atmoringsshadow" => sw.sky_rings_shadows = on,
                    "atmoextinction" => sw.atmo_extinction = on,
                    "enableplanetshine" => sw.planet_shine = on,
                    "atmominwidthpix" => sw.atmo_min_width_px = v as f32,
                    "atmodistancelimit" => sw.atmo_distance_limit = v as f32,
                    _ => sw.aurora_bright = v as f32,
                }
            }
            "brightness" | "contrast" | "saturation" | "gamma" => {
                let Ok(mut c) = self.color.single_mut() else { return false };
                let v4 = &mut c.bcsg;
                match k.as_str() {
                    "brightness" => v4.x = v as f32 - 1.0,
                    "contrast" => v4.y = v as f32,
                    "saturation" => v4.z = v as f32 - 1.0,
                    _ => v4.w = 1.0 / (v as f32).max(1e-3),
                }
            }
            // SE's WaypointsOpacity runs 0..1 (FUN_140014d10).
            "waypointsopacity" => self.waypoints.opacity = v.clamp(0.0, 1.0) as f32,
            "showlabelswaypoints" => self.waypoints.labels = on,
            f => {
                if o.sky.set_var(f, on) {
                    return true;
                }
                match crate::overlay::kind_filter(o, f) {
                    Some((set, kind)) => set.set(kind, on),
                    None => return false,
                }
            }
        }
        true
    }

    /// Every mapped setting's value, for `SaveVars`.
    pub fn snapshot(&self, cam: &FlyCamera, sim: &SimTime) -> Vec<(String, f64)> {
        let view = self.view(cam, sim);
        all_names().filter_map(|n| view.get(&n).map(|v| (n, v))).collect()
    }
}

/// Every mapped setting's name (lower case).
fn all_names() -> impl Iterator<Item = String> {
    SCALARS.iter().map(|s| s.to_string()).chain(SkySwitches::KEYS.iter().map(|k| k.to_ascii_lowercase())).chain(kind_filter_names())
}

/// The value `Reset` puts a setting back to: the port's default for it (SE resets a variable to
/// its registered default, FUN_14054f320). `None` for names the port has no setting for.
pub fn default_value(name: &str) -> Option<f64> {
    let (stars, overlay, bloom, lens) = (StarSettings::default(), OverlaySettings::default(), BloomSettings::default(), LensFlareSettings::default());
    let (gfx, proc, wps) = (GraphicsSettings::default(), ProcSettings::default(), Waypoints::default());
    let (exposure, color, clusters, switches) = (ExposureComp::default(), ColorCorrection::default(), ClusterSettings { show_cat: true }, RenderSwitches::default());
    let (cam, sim) = (FlyCamera::default(), SimTime { rate: 1.0, ..SimTime::default() });
    let view = View {
        stars: &stars,
        overlay: &overlay,
        bloom: &bloom,
        lens: &lens,
        gfx: &gfx,
        proc: &proc,
        wps: &wps,
        clusters: Some(&clusters),
        exposure: Some(&exposure),
        photo: Some(&PhotoMode::default()),
        switches: Some(&switches),
        color: Some(&color),
        cam: &cam,
        sim: &sim,
    };
    view.get(&name.to_ascii_lowercase())
}

/// `ShowOrbits<Kind>` / `ShowLabels<Kind>` / `ShowMarkers<Kind>` (lower case).
fn kind_filter_get(o: &OverlaySettings, key: &str) -> Option<f64> {
    let (set, rest) = if let Some(r) = key.strip_prefix("showorbits") {
        (o.orbit_kinds, r)
    } else if let Some(r) = key.strip_prefix("showlabels") {
        (o.label_kinds, r)
    } else {
        (o.marker_kinds, key.strip_prefix("showmarkers")?)
    };
    let kind = MarkerKind::ALL.into_iter().find(|k| k.key().eq_ignore_ascii_case(rest))?;
    Some(b(set.has(kind)))
}

// ---- saved tutorial progress -------------------------------------------------------

/// Script variables SE saves with its config; the port writes them to `script-vars.cfg` in its
/// user folder. Test runs (`--script`, `--run`: `settings::scripted`) neither read nor write it.
pub const PERSISTENT: &[&str] = &["lessonpassed"];

fn vars_path() -> std::path::PathBuf {
    crate::settings::user_file("script-vars.cfg")
}

/// The saved values of [`PERSISTENT`] variables (lower-case names).
pub fn load_persistent() -> Vec<(String, f64)> {
    let Ok(text) = std::fs::read_to_string(vars_path()) else { return Vec::new() };
    parse_persistent(&text)
}

fn parse_persistent(text: &str) -> Vec<(String, f64)> {
    let doc = se_core::sc::parse(text);
    doc.items
        .iter()
        .filter_map(|i| {
            let k = i.key.to_ascii_lowercase();
            PERSISTENT.contains(&k.as_str()).then_some(())?;
            Some((k, i.f64()?))
        })
        .collect()
}

pub fn save_persistent(vars: &[(String, f64)]) {
    let mut out = String::from("// SpaceEngine-rs saved script variables (tutorial progress)\n");
    for (k, v) in vars {
        let name = if k == "lessonpassed" { "LessonPassed" } else { k.as_str() };
        out += &format!("{name} {v}\n");
    }
    let path = vars_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, out) {
        warn!("cannot save {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_vars_round_trip() {
        let v = parse_persistent("// x\nLessonPassed 3\nOther 5\n");
        assert_eq!(v, vec![("lessonpassed".to_string(), 3.0)]);
    }

    #[test]
    fn kind_filter_names_cover_every_kind() {
        let names: Vec<String> = kind_filter_names().collect();
        assert_eq!(names.len(), 3 * MarkerKind::ALL.len());
        assert!(names.iter().any(|n| n == "showlabelsplanets"), "{names:?}");
    }

    #[test]
    fn kind_filters_read_back() {
        let mut o = OverlaySettings::default();
        crate::overlay::kind_filter(&mut o, "showorbitscomets").unwrap().0.set(MarkerKind::Comet, false);
        assert_eq!(kind_filter_get(&o, "showorbitscomets"), Some(0.0));
        assert!(kind_filter_get(&o, "showsomething").is_none());
    }

    #[test]
    fn reset_values_are_the_defaults() {
        assert_eq!(default_value("ShowOrbits"), default_value("showorbits"));
        assert_eq!(default_value("TimeScale"), Some(1.0));
        assert_eq!(default_value("WaypointsOpacity"), Some(1.0));
        // ColorCorrection's default saturation is main-def's 1.5; exposure 0 stops.
        assert!((default_value("Saturation").unwrap() - 1.5).abs() < 1e-6);
        assert_eq!(default_value("ExposureComp"), Some(0.0));
        assert_eq!(default_value("NoSuchSetting"), None);
        // Every name SaveVars lists has a default.
        assert!(all_names().all(|n| default_value(&n).is_some()), "{:?}", all_names().find(|n| default_value(n).is_none()));
    }
}
