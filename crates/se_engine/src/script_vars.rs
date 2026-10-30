//! Script access to the engine's settings (SE's `Set`, `SetU`, `SetForce`, `Get`, `Reset`,
//! `Interpolate`, `WaitVar`, `SaveVars`, `RestoreVars`: FUN_14055ac90 .. FUN_14055a4f0).
//!
//! SE's script variables are its config variables (registered with FUN_1405a4a00), so a script
//! reads and writes the same values as the settings dialogs. [`Settings`] maps the names the
//! shipped scripts use (`tutorials/DefaultSettings.se` and the tutorials) onto the port's own
//! settings resources; names with no counterpart in the port stay script variables (kept by
//! the runner, readable by `if`/`WaitVar`).
//!
//! `LessonPassed` (written with `SetForce` at the end of each tutorial) is SE's saved tutorial
//! progress; the port keeps it in its user folder (`script-vars.cfg`).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::bloom::BloomSettings;
use crate::camera::FlyCamera;
use crate::graphics::GraphicsSettings;
use crate::lensflare::LensFlareSettings;
use crate::overlay::{MarkerKind, OverlaySettings, SkySwitches};
use crate::procgalaxies::ProcSettings;
use crate::stars::{PointStyle, StarSettings};
use crate::universe::SimTime;
use crate::waypoints::Waypoints;

/// The settings a script can reach.
#[derive(SystemParam)]
pub struct Settings<'w> {
    pub stars: ResMut<'w, StarSettings>,
    pub overlay: ResMut<'w, OverlaySettings>,
    pub bloom: ResMut<'w, BloomSettings>,
    pub lens: ResMut<'w, LensFlareSettings>,
    pub gfx: ResMut<'w, GraphicsSettings>,
    pub proc: ResMut<'w, ProcSettings>,
    pub waypoints: ResMut<'w, Waypoints>,
}

/// Names (lower case) of the scalar settings mapped below, for `SaveVars`.
pub const SCALARS: &[&str] = &[
    "showorbits",
    "labelsmode",
    "showmarkers",
    "starmagnlimit",
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

impl Settings<'_> {
    /// A setting's value by its SE name (any case); `None` when the port has no such setting.
    pub fn get(&self, cam: &FlyCamera, sim: &SimTime, name: &str) -> Option<f64> {
        let k = name.to_ascii_lowercase();
        let (s, o) = (&*self.stars, &*self.overlay);
        Some(match k.as_str() {
            "showorbits" => b(o.orbits),
            "labelsmode" => o.labels_mode as f64,
            "showmarkers" => b(o.markers),
            "starmagnlimit" => s.mag_limit as f64,
            "pointoverbright" => s.overbright as f64,
            "pointscale" => s.point_scale as f64,
            "pointdesaturation" => s.desaturation as f64,
            "starcolorsaturation" => s.saturation as f64,
            "starcolorshift" => s.color_shift as f64,
            "sunillumsaturation" => s.illum_saturation as f64,
            "illumsaturationauto" => b(s.illum_saturation_auto),
            "starpointmethod" => s.point_style.method() as f64,
            "motionblurlength" => s.blur_length as f64,
            "timescale" => sim.rate,
            "fov" => cam.fov_deg as f64,
            "autohorizon" => b(cam.auto_horizon),
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
            "waypointsopacity" => self.waypoints.opacity as f64,
            "showlabelswaypoints" => b(self.waypoints.labels),
            f => match o.sky.var(f) {
                Some(on) => b(on),
                None => kind_filter_get(o, f)?,
            },
        })
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
        let names = SCALARS.iter().map(|s| s.to_string()).chain(SkySwitches::KEYS.iter().map(|k| k.to_ascii_lowercase())).chain(kind_filter_names());
        names.filter_map(|n| self.get(cam, sim, &n).map(|v| (n, v))).collect()
    }
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
/// user folder.
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
}
