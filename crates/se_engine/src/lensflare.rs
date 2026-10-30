//! Lens flares of the suns of the current system (SE's `seLensFlareEffect`, docs/formats/lens_flares.md).
//!
//! Two sprite layers come from the presets in `textures/flares/Flares.pak`: the flare (central glow and
//! diffraction spikes, `LensFlarePreset`) and the ghosts (`LensGhostPreset`). Each sprite sits on the
//! line from the sun through the screen centre and interpolates its brightness, size and angle
//! between the "centre" and "edge" values by how far the sun is from the centre. A third layer, the
//! "pseudo lens flare" (`PseudoFlarePreset`), is screen-space: ghosts and a halo of the bloom, the
//! starburst and lens dirt (`hdr_combine.glsl`, drawn by bloom.rs's combine from [`SeBloom`]).
//!
//! A flare fades with the sun's apparent magnitude (`LensFlareFadeBeginMagn` -30 .. `EndMagn` -14) and is
//! scaled by the frame itself at the sun (`lens_flare.glsl`): bloom.rs reads the frame (small suns) or
//! the bloom (large ones, `LensFlareSmallMethodRadius` 5 .. `LargeMethodRadius` 8 px) at its centre
//! before drawing the sprites, so whatever is drawn over the sun (bodies, terrain, clouds, rings,
//! ships) hides the flare and the atmosphere's extinction dims and reddens it.

use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use se_core::catalog::ObjectRef;
use se_core::lens_flare::{FlareSet, PseudoFlare, parse_flare_set, parse_pseudo, preset_of};
use se_core::vfs::Vfs;

use crate::bloom::{FlareDraw, LensTextures, MAX_FLARE_SOURCES, SeBloom};
use crate::camera::{FlyCamera, MainCamera};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::system::SystemBody;
use crate::universe::UniverseSet;

const FLARE_DIR: &str = "textures/flares";
/// Extra gain of the ghost layer: its atlas tiles are dim (peak 0.13) yet SE shows the ghosts clearly.
const GHOST_GAIN: f32 = 3.0;
/// The bloom level read for a large sun: 7 - log2(screen height / sun radius), 0..5 (FUN_1402698e0).
const BLOOM_LEVEL_BASE: f64 = 7.0;
const BLOOM_LEVEL_MAX: f64 = 5.0;
/// The gain from the port's frame to SE's brightness scale for the flare test. The port draws a star's
/// disc at `system::STAR_SURFACE_BRIGHT` (2) where SE has `hdrStarSurfaceBright` (2000) times its Planck
/// table, so an open sun saturates SE's frame sample (at BloomDynamicRange) but reads about 2..5 in
/// the port; the port's scenery is on SE's scale. x10 puts an open disc past the top of SE's 3..10
/// ramp while the dark side of a mountain, a moon or a ring in front of the sun stays below it.
const STAR_FRAME_GAIN: f32 = 10.0;

/// The `PointRendering` / `Lighting` / `PostEffects` keys of `config/main-def.cfg` that drive the flares.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct LensFlareSettings {
    pub flares: bool,
    pub ghosts: bool,
    pub pseudo: bool,
    pub scale: f32,
    pub bright: f32,
    pub pseudo_bright: f32,
    pub flare_preset: String,
    pub ghost_preset: String,
    pub pseudo_preset: String,
    pub fade_begin_mag: f64,
    pub fade_end_mag: f64,
    /// `LensFlareSmallMethodRadius` / `LensFlareLargeMethodRadius`: sun radii (px) up to which the flare
    /// reads the frame, and from which it reads the bloom.
    pub small_method_px: f64,
    pub large_method_px: f64,
    /// `BloomDynamicRange`: the limit of the frame sample (`Params2.w`).
    pub frame_limit: f32,
}

impl Default for LensFlareSettings {
    fn default() -> Self {
        Self {
            flares: true,
            ghosts: true,
            pseudo: true,
            scale: 1.0,
            bright: 1.0,
            pseudo_bright: 0.5,
            flare_preset: "Astroniki2".into(),
            ghost_preset: "Fireinthehole".into(),
            pseudo_preset: "Default".into(),
            fade_begin_mag: -30.0,
            fade_end_mag: -14.0,
            small_method_px: 5.0,
            large_method_px: 8.0,
            frame_limit: 50.0,
        }
    }
}

impl LensFlareSettings {
    /// SE's defaults from the text of `config/main-def.cfg`.
    pub fn from_main_def(text: &str) -> Self {
        let doc = se_core::sc::parse(text);
        let mut s = Self::default();
        let num = |k: &str| doc.find_deep(k).and_then(|i| i.f64());
        let flag = |k: &str| doc.find_deep(k).and_then(|i| i.bool());
        let name = |k: &str| doc.find_deep(k).and_then(|i| i.str()).map(str::to_string);
        if let Some(v) = flag("EnableLensFlares") {
            s.flares = v;
        }
        if let Some(v) = flag("EnableLensGhosts") {
            s.ghosts = v;
        }
        if let Some(v) = flag("EnablePseudoFlares") {
            s.pseudo = v;
        }
        if let Some(v) = num("LensFlareScale") {
            s.scale = v as f32;
        }
        if let Some(v) = num("LensFlareBright") {
            s.bright = v as f32;
        }
        if let Some(v) = num("PseudoFlareBright") {
            s.pseudo_bright = v as f32;
        }
        if let Some(v) = name("LensFlarePreset") {
            s.flare_preset = v;
        }
        if let Some(v) = name("LensGhostPreset") {
            s.ghost_preset = v;
        }
        if let Some(v) = name("PseudoFlarePreset") {
            s.pseudo_preset = v;
        }
        if let Some(v) = num("LensFlareFadeBeginMagn") {
            s.fade_begin_mag = v;
        }
        if let Some(v) = num("LensFlareFadeEndMagn") {
            s.fade_end_mag = v;
        }
        if let Some(v) = num("LensFlareSmallMethodRadius") {
            s.small_method_px = v;
        }
        if let Some(v) = num("LensFlareLargeMethodRadius") {
            s.large_method_px = v;
        }
        if let Some(v) = num("BloomDynamicRange") {
            s.frame_limit = v as f32;
        }
        s
    }
}

/// Every preset of `Flares.pak`, by name.
#[derive(Resource, Default)]
pub struct FlarePresets {
    pub flares: BTreeMap<String, FlareSet>,
    pub ghosts: BTreeMap<String, FlareSet>,
    pub pseudo: BTreeMap<String, PseudoFlare>,
    /// Atlas textures by lower-case file stem, with their size in pixels.
    textures: BTreeMap<String, (Handle<Image>, Vec2)>,
}

impl FlarePresets {
    fn atlas(&self, name: &str) -> Option<&(Handle<Image>, Vec2)> {
        let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
        self.textures.get(&stem)
    }
}

pub struct LensFlarePlugin;

impl Plugin for LensFlarePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LensFlareSettings>()
            .add_systems(Startup, load_presets)
            .add_systems(PostUpdate, update_flares.after(UniverseSet::Camera));
    }
}

fn load_presets(mut commands: Commands, cfg: Res<EngineConfig>, data: Res<SeData>, mut images: ResMut<Assets<Image>>) {
    if let Ok(text) = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")) {
        commands.insert_resource(LensFlareSettings::from_main_def(&text));
    }
    let presets = read_presets(&data.vfs, &mut images);
    info!(
        "lens flares: {} flare, {} ghost, {} pseudo presets",
        presets.flares.len(),
        presets.ghosts.len(),
        presets.pseudo.len()
    );
    commands.insert_resource(presets);
}

fn read_presets(vfs: &Vfs, images: &mut Assets<Image>) -> FlarePresets {
    let mut p = FlarePresets::default();
    for path in vfs.list_ext(FLARE_DIR, "cfg") {
        let Some((name, kind)) = preset_of(&path) else { continue };
        let Ok(text) = vfs.read_string(&path) else { continue };
        match kind {
            "flares" => {
                p.flares.insert(name.to_string(), parse_flare_set(&text));
            }
            "ghosts" => {
                p.ghosts.insert(name.to_string(), parse_flare_set(&text));
            }
            _ => {
                p.pseudo.insert(name.to_string(), parse_pseudo(&text));
            }
        }
    }
    for path in vfs.list_ext(FLARE_DIR, "png") {
        let Ok(bytes) = vfs.read(&path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else { continue };
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let stem = path.rsplit('/').next().unwrap_or(&path).split('.').next().unwrap_or("").to_ascii_lowercase();
        let mut image = Image::new(
            Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            rgba.into_raw(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        p.textures.insert(stem, (images.add(image), Vec2::new(w as f32, h as f32)));
    }
    p
}

/// A sun on screen: its position in screen units (x scaled by the aspect, y up, half-height = 1),
/// flare strength (the magnitude fade) and its probe texel.
#[derive(Clone, Copy, Debug)]
struct Source {
    pos: Vec2,
    strength: f32,
    index: u32,
}

/// Fade of a sun's flare with its apparent magnitude: 1 at `begin` or brighter, 0 at `end`.
fn magnitude_fade(mag: f64, begin: f64, end: f64) -> f32 {
    ((end - mag) / (end - begin)).clamp(0.0, 1.0) as f32
}

/// SE's `smoothstep(e0, e1, x)` (FUN_140029820).
fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    (3.0 - 2.0 * t) * t * t
}

/// What the probe pass reads for a sun of radius `r_px` on a screen `height_px` tall (FUN_1402698e0
/// and FUN_1404e6240): the bloom level `clamp(7 - log2(height / r), 0, 5)` and the frame-to-bloom mix
/// `smoothstep(small, large, r)`.
fn probe_method(r_px: f64, height_px: f64, small: f64, large: f64) -> (f32, f32) {
    let level = (BLOOM_LEVEL_BASE - (height_px / r_px.max(1e-6)).log2()).clamp(0.0, BLOOM_LEVEL_MAX);
    (level as f32, smoothstep(small, large, r_px) as f32)
}

/// Appends the sprites of `set` for one sun (two triangles each). `aspect` is width / height.
fn push_sprites(set: &FlareSet, atlas_size: Vec2, src: &Source, s: &LensFlareSettings, aspect: f32, ghost_falloff: bool, out: &mut Vec<f32>) {
    // 0 with the sun at the screen centre, 1 at an edge.
    let t = (src.pos.x.abs() / aspect).max(src.pos.y.abs()).min(1.0);
    for e in &set.elements {
        let bright = (e.bright[0] + (e.bright[1] - e.bright[0]) * t) * set.bright_all * s.bright * src.strength;
        if bright <= 0.0 {
            continue;
        }
        let half = (e.scale[0] + (e.scale[1] - e.scale[0]) * t) * set.scale_all * s.scale;
        // Ghosts spread their light: past a tenth of the half-height their brightness falls with their
        // area, so the big faint ones of a preset stay washes as in SE while the small ones read clearly.
        let bright = if ghost_falloff && half > 0.1 { bright * (0.1 / half).powi(2) } else { bright };
        let angle = (e.angle[0] + (e.angle[1] - e.angle[0]) * t).to_radians();
        let centre = src.pos * (1.0 - e.position);
        let rot = Mat2::from_angle(angle);
        let [x, y, w, h] = e.rect;
        let col = Vec3::from_array(e.color) * bright;
        let corner = |cx: f32, cy: f32| {
            let p = centre + rot * Vec2::new(cx * half, cy * half * e.aspect);
            // Atlas rows run down the image; the sprite's +y is up.
            let u = (x + (cx + 1.0) * 0.5 * w) / atlas_size.x;
            let v = (y + (1.0 - cy) * 0.5 * h) / atlas_size.y;
            [p.x / aspect, p.y, u, v, col.x, col.y, col.z, src.index as f32]
        };
        let c = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(-1.0, 1.0), corner(1.0, 1.0)];
        for k in [0, 1, 2, 2, 1, 3] {
            out.extend_from_slice(&c[k]);
        }
    }
}

/// `LensRaysMatr`: the starburst turns with the camera. SE's matrix was not decoded; this is the
/// angle of John Chapman's pseudo lens flare, which hdr_combine.glsl follows (camera right . z plus
/// camera forward . y).
fn rays_matrix(rot: bevy::math::DQuat) -> Vec4 {
    let right = rot * bevy::math::DVec3::X;
    let fwd = rot * bevy::math::DVec3::NEG_Z;
    let a = (right.z + fwd.y) as f32;
    let (s, c) = a.sin_cos();
    Vec4::new(c, s, -s, c)
}

/// The pseudo lens flare's part of [`SeBloom`] (`hdr_combine.glsl` BrightParams, BlurParams,
/// FlareParams); off without a preset. Its ghosts and halo read the bloom with [`STAR_FRAME_GAIN`].
fn pseudo_uniform(b: &mut SeBloom, p: Option<&PseudoFlare>, bright: f32, aspect: f32, lens: &LensTextures, rays: Vec4) {
    let Some(p) = p else {
        b.flare_bright = Vec4::ZERO;
        return;
    };
    b.flare_bright = Vec4::new(p.ghosts_bright * bright, p.halo_bright * bright, p.dirt_bright, 1.0);
    b.flare_blur = Vec4::new(p.ghosts_blur, p.halo_blur, p.dirt_dispersal, 1.0);
    b.flare_shape = Vec4::new(p.ghosts_dispersal, p.ghosts_count as f32, p.halo_radius, p.chromaticism);
    b.lens = Vec4::new(aspect, lens.dirt.is_some() as u32 as f32, lens.rays.is_some() as u32 as f32, STAR_FRAME_GAIN);
    b.rays = rays;
}

#[allow(clippy::too_many_arguments)]
fn update_flares(
    mut commands: Commands,
    settings: Res<LensFlareSettings>,
    presets: Option<Res<FlarePresets>>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    windows: Query<&Window, With<PrimaryWindow>>,
    bodies: Query<&SystemBody>,
    mut camera: Query<(Entity, Option<&mut FlareDraw>, Option<&mut SeBloom>), With<MainCamera>>,
    mut lens: ResMut<LensTextures>,
) {
    let Some(presets) = presets else { return };
    let Ok((cam_entity, draw, bloom)) = camera.single_mut() else { return };
    let (w, h) = windows.iter().next().map(|w| (w.width(), w.height())).unwrap_or((1600.0, 900.0));
    let aspect = w / h.max(1.0);
    let tan_half = (cam.fov_deg * 0.5).to_radians().tan() as f64;
    let inv = cam.rot.inverse();
    let cat = &data.catalog;

    let mut sources: Vec<(f64, Vec2, f32, Vec4)> = Vec::new();
    for b in bodies.iter().filter(|b| b.star) {
        let ObjectRef::Star(i) = b.obj else { continue };
        let st = &cat.stars[i as usize];
        let rel = b.pos.delta_m(cam.pos);
        let dist = rel.length();
        if dist <= b.radius_m {
            continue;
        }
        let mag = se_core::star::app_mag_from_abs(st.abs_mag, dist / se_core::units::METERS_PER_PC);
        let fade = magnitude_fade(mag, settings.fade_begin_mag, settings.fade_end_mag);
        if fade <= 0.0 {
            continue;
        }
        // Screen position; the sun's disc must touch the screen.
        let v = inv * (rel / dist);
        if v.z >= 0.0 {
            continue;
        }
        let sx = v.x / -v.z / tan_half;
        let sy = v.y / -v.z / tan_half;
        let r_scr = (b.radius_m / dist).asin().tan() / tan_half;
        if sx.abs() - r_scr > aspect as f64 || sy.abs() - r_scr > 1.0 {
            continue;
        }
        let r_px = r_scr * h as f64 * 0.5;
        let (level, mix) = probe_method(r_px, h as f64, settings.small_method_px, settings.large_method_px);
        let uv = Vec2::new(((sx / aspect as f64 + 1.0) * 0.5) as f32, ((1.0 - sy) * 0.5) as f32);
        sources.push((mag, Vec2::new(sx as f32, sy as f32), fade, Vec4::new(uv.x, uv.y, level, mix)));
    }
    sources.sort_by(|a, b| a.0.total_cmp(&b.0));
    sources.truncate(MAX_FLARE_SOURCES);

    let mut fd = FlareDraw { count: sources.len() as u32, frame_limit: settings.frame_limit, frame_gain: STAR_FRAME_GAIN, ..default() };
    for (k, s) in sources.iter().enumerate() {
        fd.sources[k] = s.3;
    }
    for ghosts in [false, true] {
        let (on, set) = if ghosts {
            (settings.ghosts, presets.ghosts.get(&settings.ghost_preset))
        } else {
            (settings.flares, presets.flares.get(&settings.flare_preset))
        };
        let (Some(set), true) = (set, on) else { continue };
        let Some((tex, size)) = presets.atlas(&set.texture) else { continue };
        let mut verts = Vec::new();
        for (k, s) in sources.iter().enumerate() {
            let gain = if ghosts { GHOST_GAIN } else { 1.0 };
            let src = Source { pos: s.1, strength: s.2 * gain, index: k as u32 };
            push_sprites(set, *size, &src, &settings, aspect, ghosts, &mut verts);
        }
        if !verts.is_empty() {
            fd.batches.push((tex.id(), verts));
        }
    }
    match draw {
        Some(mut d) => *d = fd,
        None => {
            commands.entity(cam_entity).insert(fd);
        }
    }

    // The pseudo lens flare's preset and textures.
    let pseudo = presets.pseudo.get(&settings.pseudo_preset).filter(|_| settings.pseudo);
    let tex = |name: Option<&String>| name.filter(|n| !n.is_empty()).and_then(|n| presets.atlas(n)).map(|t| t.0.id());
    let want = LensTextures { dirt: tex(pseudo.map(|p| &p.dirt_texture)), rays: tex(pseudo.map(|p| &p.rays_texture)) };
    if *lens != want {
        *lens = want;
    }
    if let Some(mut b) = bloom {
        let mut u = *b;
        pseudo_uniform(&mut u, pseudo, settings.pseudo_bright, aspect, &lens, rays_matrix(cam.rot));
        if *b != u {
            *b = u;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloom::FLARE_VERTEX_FLOATS;
    use se_core::lens_flare::FlareElement;

    #[test]
    fn probe_reads_the_frame_for_small_suns_and_the_bloom_for_large_ones() {
        let (l, m) = probe_method(4.0, 900.0, 5.0, 8.0);
        assert_eq!((l, m), (0.0, 0.0));
        let (_, m) = probe_method(6.5, 900.0, 5.0, 8.0);
        assert!((m - 0.5).abs() < 1e-6);
        // A sun 56 px across on a 900 px screen: level 7 - log2(16) = 3.
        let (l, m) = probe_method(56.25, 900.0, 5.0, 8.0);
        assert_eq!((l, m), (3.0, 1.0));
        assert_eq!(probe_method(2000.0, 900.0, 5.0, 8.0).0, 5.0);
    }

    #[test]
    fn magnitude_fade_spans_the_configured_range() {
        assert_eq!(magnitude_fade(-31.0, -30.0, -14.0), 1.0);
        assert_eq!(magnitude_fade(-13.0, -30.0, -14.0), 0.0);
        // The Sun from Earth.
        assert!((magnitude_fade(-26.74, -30.0, -14.0) - 0.796).abs() < 0.01);
    }

    #[test]
    fn settings_read_the_method_radii() {
        let s = LensFlareSettings::from_main_def("PostEffects { LensFlareSmallMethodRadius 4 LensFlareLargeMethodRadius 9 BloomDynamicRange 40 }");
        assert_eq!((s.small_method_px, s.large_method_px, s.frame_limit), (4.0, 9.0, 40.0));
    }

    #[test]
    fn sprites_sit_on_the_sun_centre_axis() {
        let e = |position: f32| FlareElement {
            color: [1.0; 3],
            bright: [1.0, 1.0],
            scale: [1.0, 1.0],
            aspect: 1.0,
            position,
            angle: [0.0, 0.0],
            chroma: 0.0,
            rect: [0.0, 0.0, 64.0, 64.0],
        };
        let set = FlareSet { scale_all: 0.1, bright_all: 1.0, texture: String::new(), elements: vec![e(0.0), e(1.0), e(2.0)] };
        let src = Source { pos: Vec2::new(0.8, 0.4), strength: 1.0, index: 2 };
        let mut v = Vec::new();
        push_sprites(&set, Vec2::splat(64.0), &src, &LensFlareSettings::default(), 2.0, false, &mut v);
        let n = 6 * FLARE_VERTEX_FLOATS;
        assert_eq!(v.len(), 3 * n);
        let centre = |k: usize| {
            let q = &v[k * n..(k + 1) * n];
            let (mut x, mut y) = (0.0, 0.0);
            for c in q.chunks(FLARE_VERTEX_FLOATS) {
                x += c[0];
                y += c[1];
            }
            Vec2::new(x, y) / 6.0
        };
        // At the sun (x in NDC: 0.8 / aspect), at the centre, and mirrored.
        assert!((centre(0) - Vec2::new(0.4, 0.4)).length() < 1e-5);
        assert!(centre(1).length() < 1e-5);
        assert!((centre(2) + Vec2::new(0.4, 0.4)).length() < 1e-5);
        // Every vertex names its sun's probe texel.
        assert!(v.chunks(FLARE_VERTEX_FLOATS).all(|c| c[7] == 2.0));
    }

    #[test]
    fn rays_matrix_is_a_rotation() {
        let m = rays_matrix(bevy::math::DQuat::from_rotation_y(0.7));
        assert!((m.x * m.w - m.y * m.z - 1.0).abs() < 1e-5);
        assert_eq!(rays_matrix(bevy::math::DQuat::IDENTITY), Vec4::new(1.0, 0.0, 0.0, 1.0));
    }
}
