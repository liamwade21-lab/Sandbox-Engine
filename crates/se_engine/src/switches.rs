//! SE's switches for what is drawn around planets: the object filters `ShowAtmospheres`,
//! `ShowClouds`, `ShowAurora`, `ShowCometTails` and `ShowEclipseMask` (main-def.cfg ObjectFilters), and
//! the settings `EnableEclipses`, `EnablePlanetShine` (PlanetRendering), `AtmoRingsShadow` and
//! `AtmoExtinction` (AtmosphereRendering), with SE's default keys from its key table FUN_1405ba8f0:
//! ToggleAtmospheres Shift+A, ToggleClouds Shift+C, ToggleAurora Shift+Ctrl+A, ToggleCometTails
//! Shift+Ctrl+C, ToggleEclipses Ctrl+L, TogglePlanetShine Shift+L (ToggleSkyRingsShadows and
//! ToggleEclipseMask have no default key). Also SE's atmosphere-at-a-distance and aurora keys
//! (`AtmoMinWidthPix`, `AtmoDistanceLimit`, `AuroraBright`).

use bevy::prelude::*;

use crate::config::EngineConfig;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct RenderSwitches {
    /// `ShowAtmospheres`: the sky shells.
    pub atmospheres: bool,
    /// `ShowClouds`.
    pub clouds: bool,
    /// `ShowAurora`.
    pub aurora: bool,
    /// `ShowCometTails`.
    pub comet_tails: bool,
    /// `ShowEclipseMask`: SE's "Eclipse totality mask" overlay.
    pub eclipse_mask: bool,
    /// `EnableEclipses`: SE's eclipse intensity (AmbientColor.a, its DAT_1419fbb04 squared).
    pub eclipses: bool,
    /// `AtmoRingsShadow`: "Ring shadows on sky".
    pub sky_rings_shadows: bool,
    /// `AtmoExtinction`: the sky dims and reddens what lies behind it.
    pub atmo_extinction: bool,
    /// `EnablePlanetShine` (mirrored into `system::LightingFade::planet_shine`).
    pub planet_shine: bool,
    /// `AtmoMinWidthPix`: the least thickness of an atmosphere on screen (px).
    pub atmo_min_width_px: f32,
    /// `AtmoDistanceLimit`: the farthest eye distance (planet radii) the atmosphere is computed from.
    pub atmo_distance_limit: f32,
    /// `AuroraBright`.
    pub aurora_bright: f32,
}

impl Default for RenderSwitches {
    fn default() -> Self {
        // main-def.cfg's values.
        Self {
            atmospheres: true,
            clouds: true,
            aurora: true,
            comet_tails: true,
            eclipse_mask: false,
            eclipses: true,
            sky_rings_shadows: true,
            atmo_extinction: true,
            planet_shine: true,
            atmo_min_width_px: 2.0,
            atmo_distance_limit: 30.0,
            aurora_bright: 1.0,
        }
    }
}

impl RenderSwitches {
    /// The persisted switches by SE's keys (settings.rs).
    pub const KEYS: [&'static str; 9] = [
        "ShowAtmospheres",
        "ShowClouds",
        "ShowAurora",
        "ShowCometTails",
        "ShowEclipseMask",
        "EnableEclipses",
        "AtmoRingsShadow",
        "AtmoExtinction",
        "EnablePlanetShine",
    ];

    pub fn saved(&self) -> [bool; 9] {
        [
            self.atmospheres,
            self.clouds,
            self.aurora,
            self.comet_tails,
            self.eclipse_mask,
            self.eclipses,
            self.sky_rings_shadows,
            self.atmo_extinction,
            self.planet_shine,
        ]
    }

    pub fn set_var(&mut self, key: &str, on: bool) {
        let v = match key {
            "ShowAtmospheres" => &mut self.atmospheres,
            "ShowClouds" => &mut self.clouds,
            "ShowAurora" => &mut self.aurora,
            "ShowCometTails" => &mut self.comet_tails,
            "ShowEclipseMask" => &mut self.eclipse_mask,
            "EnableEclipses" => &mut self.eclipses,
            "AtmoRingsShadow" => &mut self.sky_rings_shadows,
            "AtmoExtinction" => &mut self.atmo_extinction,
            "EnablePlanetShine" => &mut self.planet_shine,
            _ => return,
        };
        *v = on;
    }

    /// The defaults from main-def.cfg's text.
    pub fn from_main_def(text: &str) -> Self {
        let doc = se_core::sc::parse(text);
        let mut s = Self::default();
        for key in Self::KEYS {
            if let Some(on) = doc.find_deep(key).and_then(|i| i.bool()) {
                s.set_var(key, on);
            }
        }
        let num = |k: &str, d: f32| doc.find_deep(k).and_then(|i| i.f64()).map_or(d, |v| v as f32);
        s.atmo_min_width_px = num("AtmoMinWidthPix", s.atmo_min_width_px);
        s.atmo_distance_limit = num("AtmoDistanceLimit", s.atmo_distance_limit);
        s.aurora_bright = num("AuroraBright", s.aurora_bright);
        s
    }

    /// SE's eclipse intensity (`AmbientColor.a`).
    pub fn eclipse_intensity(&self) -> f32 {
        if self.eclipses { 1.0 } else { 0.0 }
    }
}

pub struct SwitchesPlugin;

impl Plugin for SwitchesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderSwitches>()
            .add_systems(Startup, load_switches)
            .add_systems(Update, (switch_keys, sync_planet_shine).chain());
    }
}

fn load_switches(mut switches: ResMut<RenderSwitches>, cfg: Res<EngineConfig>) {
    if let Ok(text) = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")) {
        *switches = RenderSwitches::from_main_def(&text);
    }
}

/// SE's keys for the switches (FUN_1405ba8f0: key, then Shift 1 / Ctrl 2 / Alt 4).
fn switch_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut switches: ResMut<RenderSwitches>,
    mut msg: ResMut<crate::messages::Messages>,
    i18n: Res<crate::i18n::I18n>,
    mut egui: bevy_egui::EguiContexts,
) {
    // Not while typing into a text field.
    if egui.ctx_mut().is_ok_and(|c| c.egui_wants_keyboard_input()) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let s = &mut *switches;
    let hit = match (shift, ctrl, alt) {
        (true, false, false) if keys.just_pressed(KeyCode::KeyA) => Some((&mut s.atmospheres, "Atmospheres")),
        (true, false, false) if keys.just_pressed(KeyCode::KeyC) => Some((&mut s.clouds, "Clouds")),
        (true, true, false) if keys.just_pressed(KeyCode::KeyA) => Some((&mut s.aurora, "Aurora")),
        (true, true, false) if keys.just_pressed(KeyCode::KeyC) => Some((&mut s.comet_tails, "Comet tails")),
        (false, true, false) if keys.just_pressed(KeyCode::KeyL) => Some((&mut s.eclipses, "Eclipse shadows")),
        (true, false, false) if keys.just_pressed(KeyCode::KeyL) => Some((&mut s.planet_shine, "Planet shine")),
        _ => None,
    };
    if let Some((v, name)) = hit {
        *v = !*v;
        msg.switch(&i18n, name, *v);
    }
}

/// `EnablePlanetShine` drives the lighting's planet shine (system.rs).
fn sync_planet_shine(switches: Res<RenderSwitches>, fade: Option<ResMut<crate::system::LightingFade>>) {
    if let Some(mut fade) = fade
        && fade.planet_shine != switches.planet_shine
    {
        fade.planet_shine = switches.planet_shine;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_main_def_keys() {
        let s = RenderSwitches::from_main_def(
            "ObjectFilters\n{\n\tShowAtmospheres true\n\tShowClouds false\n\tShowEclipseMask false\n}\nAtmosphereRendering\n{\n\tAtmoRingsShadow false\n\tAtmoDistanceLimit 30\n\tAtmoMinWidthPix 2\n}\nPlanetRendering\n{\n\tEnableEclipses true\n\tAuroraBright 1.5\n}\n",
        );
        assert!(s.atmospheres && !s.clouds && !s.sky_rings_shadows && s.eclipses);
        assert_eq!((s.atmo_distance_limit, s.atmo_min_width_px, s.aurora_bright), (30.0, 2.0, 1.5));
        let mut t = s.clone();
        for (k, on) in RenderSwitches::KEYS.iter().zip(s.saved()) {
            t.set_var(k, !on);
        }
        assert_eq!(t.saved().map(|b| !b), s.saved());
    }
}
