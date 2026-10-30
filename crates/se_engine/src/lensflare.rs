//! Lens flares of the suns of the current system (SE's `seLensFlareEffect`, docs/formats/lens_flares.md).
//!
//! Two sprite layers come from the presets in `textures/flares/Flares.pak`: the flare (central glow and
//! diffraction spikes, `LensFlarePreset`) and the ghosts (`LensGhostPreset`). Each sprite sits on the
//! line from the sun through the screen centre and interpolates its brightness, size and angle
//! between the "centre" and "edge" values by how far the sun is from the centre. A third layer, the
//! "pseudo lens flare" (`PseudoFlarePreset`), is a screen-space pass that mirrors the bright parts of
//! the frame into ghosts and a halo (`assets/shaders/pseudo_flare.wesl`).
//!
//! A flare fades with the sun's apparent magnitude (`LensFlareFadeBeginMagn` -30 .. `EndMagn` -14) and
//! with the part of its disc hidden by the bodies of the system.

use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::math::DVec3;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use se_core::catalog::ObjectRef;
use se_core::lens_flare::{FlareSet, PseudoFlare, parse_flare_set, parse_pseudo, preset_of};
use se_core::vfs::Vfs;

use crate::camera::{FlyCamera, MainCamera};
use crate::config::EngineConfig;
use crate::data::SeData;
use crate::stars::star_render_color;
use crate::system::SystemBody;
use crate::terrain::disc_overlap;
use crate::universe::UniverseSet;

const FLARE_DIR: &str = "textures/flares";
/// Disc radius (px) around which a sun's flare goes out (the old star sprite's largest glare radius;
/// SE switches between its frame and bloom samples by the sun's size, `LensFlareLargeMethodRadius`).
const SUN_DISC_FADE_PX: f64 = 40.0;
/// Suns flared at once.
const MAX_SOURCES: usize = 4;
/// Scene brightness of a flare sprite of brightness 1 (HDR, before tone mapping).
const FLARE_GAIN: f32 = 8.0;
/// Extra gain of the ghost layer: its atlas tiles are dim (peak 0.13) yet SE shows the ghosts clearly.
const GHOST_GAIN: f32 = 3.0;

/// The `PointRendering` / `Lighting` keys of `config/main-def.cfg` that drive the flares.
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
    /// `lenscolor_harb.png` sampled at 16 points (the pseudo flare's colour along the radius).
    lens_color: [Vec4; 16],
}

impl FlarePresets {
    fn atlas(&self, name: &str) -> Option<&(Handle<Image>, Vec2)> {
        let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
        self.textures.get(&stem)
    }
}

/// A sprite layer: its mesh, rebuilt every frame, and its material.
#[derive(Component)]
struct FlareLayer {
    ghosts: bool,
    mesh: Handle<Mesh>,
    material: Handle<FlareMaterial>,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct FlareMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub atlas: Handle<Image>,
}

impl Material for FlareMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/lens_flare.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/lens_flare.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // After every other transparent layer (galaxies use up to 3e15): flares are in the lens.
        1.0e16
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Camera component of the pseudo lens flare pass. Layout must match `pseudo_flare.wesl`.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug)]
#[extract_app(RenderApp)]
pub struct PseudoFlarePass {
    /// x: ghost count, y: ghost dispersal, z: ghost brightness, w: ghost blur
    pub ghosts: Vec4,
    /// x: halo radius, y: halo brightness, z: halo blur, w: chromatic distortion
    pub halo: Vec4,
    /// x: overall brightness, y: width / height, z: brightness cap of the source, w: unused
    pub params: Vec4,
    pub lens_color: [Vec4; 16],
}

impl FullscreenMaterial for PseudoFlarePass {
    fn fragment_shader() -> ShaderRef {
        "shaders/pseudo_flare.wesl".into()
    }
}

pub struct LensFlarePlugin;

impl Plugin for LensFlarePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((MaterialPlugin::<FlareMaterial>::default(), FullscreenMaterialPlugin::<PseudoFlarePass>::default()))
            .init_resource::<LensFlareSettings>()
            .add_systems(Startup, load_presets)
            .add_systems(PostUpdate, update_flares.after(UniverseSet::Camera));
    }
}

fn load_presets(
    mut commands: Commands,
    cfg: Res<EngineConfig>,
    data: Res<SeData>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<FlareMaterial>>,
) {
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
    for ghosts in [false, true] {
        let mesh = meshes.add(Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default()));
        let material = mats.add(FlareMaterial { atlas: Handle::default() });
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
            // Seen by the main camera only (not by the black-hole environment cameras).
            crate::blackholes::MAIN_VIEW_ONLY,
            FlareLayer { ghosts, mesh, material },
        ));
    }
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
    p.lens_color = [Vec4::ONE; 16];
    for path in vfs.list_ext(FLARE_DIR, "png") {
        let Ok(bytes) = vfs.read(&path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else { continue };
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let stem = path.rsplit('/').next().unwrap_or(&path).split('.').next().unwrap_or("").to_ascii_lowercase();
        if stem == "lenscolor_harb" {
            p.lens_color = std::array::from_fn(|i| {
                let x = ((i as f32 + 0.5) / 16.0 * w as f32) as u32;
                let c = rgba.get_pixel(x.min(w - 1), 0).0;
                Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, 255.0) / 255.0
            });
        }
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
/// colour, and flare strength (magnitude fade times the visible part of its disc).
#[derive(Clone, Copy, Debug)]
struct Source {
    pos: Vec2,
    color: Vec3,
    strength: f32,
}

/// Fade of a sun's flare with its apparent magnitude: 1 at `begin` or brighter, 0 at `end`.
fn magnitude_fade(mag: f64, begin: f64, end: f64) -> f32 {
    ((end - mag) / (end - begin)).clamp(0.0, 1.0) as f32
}

/// Photosphere brightness on screen (`star_surface.wesl` at the disc centre).
const PHOTOSPHERE_LUMA: f64 = crate::system::STAR_SURFACE_BRIGHT as f64;

/// SE scales a flare by the frame's brightness at the sun, `smoothstep(3, 10, luma)` of the bloom
/// around it (`lens_flare.glsl`): a point-like sun there is its saturated glare, a close one its
/// photosphere, so the flare goes out as the disc (`disc_px` radius) outgrows the glare sprite
/// (`glare_px`), whose light spreads over the disc's area.
fn disc_fade(disc_px: f64, glare_px: f64) -> f32 {
    let luma = PHOTOSPHERE_LUMA + (10.0 - PHOTOSPHERE_LUMA) * (glare_px / disc_px.max(1e-6)).powi(2);
    let t = ((luma - 3.0) / 7.0).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// Appends the sprites of `set` for one sun (two triangles each). `aspect` is width / height.
fn push_sprites(
    set: &FlareSet,
    atlas_size: Vec2,
    src: &Source,
    s: &LensFlareSettings,
    aspect: f32,
    ghost_falloff: bool,
    mesh: &mut (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>),
) {
    // 0 with the sun at the screen centre, 1 at an edge.
    let t = (src.pos.x.abs() / aspect).max(src.pos.y.abs()).min(1.0);
    for e in &set.elements {
        let bright = (e.bright[0] + (e.bright[1] - e.bright[0]) * t) * set.bright_all * s.bright * src.strength * FLARE_GAIN;
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
        let base = mesh.0.len() as u32;
        let [x, y, w, h] = e.rect;
        let col = (Vec3::from_array(e.color) * src.color * bright).extend(1.0).to_array();
        for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let p = centre + rot * Vec2::new(cx * half, cy * half * e.aspect);
            mesh.0.push([p.x / aspect, p.y, 0.0]);
            // Atlas rows run down the image; the sprite's +y is up.
            let u = (x + (cx + 1.0) * 0.5 * w) / atlas_size.x;
            let v = (y + (1.0 - cy) * 0.5 * h) / atlas_size.y;
            mesh.1.push([u, v]);
            mesh.2.push(col);
        }
        mesh.3.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_flares(
    mut commands: Commands,
    settings: Res<LensFlareSettings>,
    presets: Option<Res<FlarePresets>>,
    data: Res<SeData>,
    cam: Res<FlyCamera>,
    windows: Query<&Window, With<PrimaryWindow>>,
    bodies: Query<(&SystemBody, Option<&crate::terrain::Terrain>)>,
    mut layers: Query<(&FlareLayer, &mut Visibility)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<FlareMaterial>>,
    camera: Query<(Entity, Option<&PseudoFlarePass>), With<MainCamera>>,
) {
    let Some(presets) = presets else { return };
    let (w, h) = windows.iter().next().map(|w| (w.width(), w.height())).unwrap_or((1600.0, 900.0));
    let aspect = w / h.max(1.0);
    let tan_half = (cam.fov_deg * 0.5).to_radians().tan() as f64;
    let inv = cam.rot.inverse();
    let cat = &data.catalog;

    // Occluders: every body of the system with a size (and whether it has a sea).
    let occluders: Vec<(DVec3, f64, bool)> = bodies
        .iter()
        .filter(|(b, _)| !b.star && b.radius_m > 0.0)
        .map(|(b, t)| (b.pos.delta_m(cam.pos), b.radius_m, t.is_some_and(|t| t.has_ocean)))
        .collect();
    let mut sources: Vec<(f64, Source)> = Vec::new();
    for (b, _) in bodies.iter().filter(|(b, _)| b.star) {
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
        // Part of the disc hidden by the bodies in front of it.
        let ang_r = (b.radius_m / dist).asin();
        let mut dir = rel / dist;
        let mut visible = 1.0;
        for &(o, r_o, sea) in &occluders {
            let d_o = o.length();
            if d_o <= r_o {
                // On low ground inside the body's sphere: its horizon plane hides the lower sky.
                let up = -o / d_o.max(1e-9);
                let cos_z = dir.dot(up);
                if cos_z < 0.0 {
                    visible = 0.0;
                } else if sea {
                    // Under the sea the star shows in Snell's window, raised by refraction and
                    // dimmed along its path through the water above.
                    let sin_t = (1.0 - cos_z * cos_z).max(0.0).sqrt() / 1.333;
                    let cos_t = (1.0 - sin_t * sin_t).sqrt();
                    dir = (up * cos_t + (dir - up * cos_z).normalize_or_zero() * sin_t).normalize();
                    visible *= (-0.03 * (r_o - d_o) / cos_t).exp();
                }
                continue;
            }
            if d_o >= dist {
                continue;
            }
            let sep = (o / d_o).angle_between(dir);
            visible *= 1.0 - disc_overlap(ang_r.max(1e-9), (r_o / d_o).asin(), sep);
        }
        if visible <= 0.0 {
            continue;
        }
        // Screen position; the sun's disc must touch the screen.
        let v = inv * dir;
        if v.z >= 0.0 {
            continue;
        }
        let sx = v.x / -v.z / tan_half;
        let sy = v.y / -v.z / tan_half;
        let r_scr = ang_r.tan() / tan_half;
        if sx.abs() - r_scr > aspect as f64 || sy.abs() - r_scr > 1.0 {
            continue;
        }
        let disc = disc_fade(r_scr * h as f64 * 0.5, SUN_DISC_FADE_PX);
        if disc <= 0.0 {
            continue;
        }
        let color = Vec3::from_array(star_render_color(st.class.kind, st.teff));
        let color = color / color.max_element().max(1e-3);
        sources.push((mag, Source { pos: Vec2::new(sx as f32, sy as f32), color, strength: fade * visible as f32 * disc }));
    }
    sources.sort_by(|a, b| a.0.total_cmp(&b.0));
    sources.truncate(MAX_SOURCES);

    for (layer, mut vis) in &mut layers {
        let (on, set) = if layer.ghosts {
            (settings.ghosts, presets.ghosts.get(&settings.ghost_preset))
        } else {
            (settings.flares, presets.flares.get(&settings.flare_preset))
        };
        let atlas = set.and_then(|s| presets.atlas(&s.texture));
        let (Some(set), Some((tex, size)), true, false) = (set, atlas, on, sources.is_empty()) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let mut m = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (_, src) in &sources {
            let gain = if layer.ghosts { GHOST_GAIN } else { 1.0 };
            push_sprites(set, *size, &Source { strength: src.strength * gain, ..*src }, &settings, aspect, layer.ghosts, &mut m);
        }
        if let Some(mat) = mats.get(&layer.material)
            && mat.atlas != *tex
            && let Some(mut mat) = mats.get_mut(&layer.material)
        {
            mat.atlas = tex.clone();
        }
        if let Some(mut mesh) = meshes.get_mut(&layer.mesh) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.0);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.1);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, m.2);
            mesh.insert_indices(Indices::U32(m.3));
        }
        *vis = Visibility::Visible;
    }

    // Pseudo lens flare: only while a sun is flared, so the pass costs nothing otherwise.
    let Ok((cam_entity, pass)) = camera.single() else { return };
    let pseudo = presets.pseudo.get(&settings.pseudo_preset);
    match (settings.pseudo && !sources.is_empty(), pseudo) {
        (true, Some(p)) => {
            let strength = sources.iter().map(|(_, s)| s.strength).fold(0.0, f32::max);
            let u = PseudoFlarePass {
                ghosts: Vec4::new(p.ghosts_count as f32, p.ghosts_dispersal, p.ghosts_bright, p.ghosts_blur),
                halo: Vec4::new(p.halo_radius, p.halo_bright, p.halo_blur, p.chromaticism),
                params: Vec4::new(settings.pseudo_bright * strength, aspect, 50.0, 0.0),
                lens_color: presets.lens_color,
            };
            commands.entity(cam_entity).insert(u);
        }
        _ => {
            if pass.is_some() {
                commands.entity(cam_entity).remove::<PseudoFlarePass>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::lens_flare::FlareElement;

    #[test]
    fn disc_fade_goes_out_as_the_disc_outgrows_the_glare() {
        assert_eq!(disc_fade(4.0, 40.0), 1.0);
        assert_eq!(disc_fade(40.0, 40.0), 1.0);
        assert!(disc_fade(80.0, 40.0) > 0.0 && disc_fade(80.0, 40.0) < 1.0);
        assert_eq!(disc_fade(120.0, 40.0), 0.0);
    }

    #[test]
    fn magnitude_fade_spans_the_configured_range() {
        assert_eq!(magnitude_fade(-31.0, -30.0, -14.0), 1.0);
        assert_eq!(magnitude_fade(-13.0, -30.0, -14.0), 0.0);
        // The Sun from Earth.
        assert!((magnitude_fade(-26.74, -30.0, -14.0) - 0.796).abs() < 0.01);
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
        let src = Source { pos: Vec2::new(0.8, 0.4), color: Vec3::ONE, strength: 1.0 };
        let mut m = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        push_sprites(&set, Vec2::splat(64.0), &src, &LensFlareSettings::default(), 2.0, false, &mut m);
        let centre = |k: usize| {
            let q = &m.0[k * 4..k * 4 + 4];
            Vec2::new(q.iter().map(|p| p[0]).sum::<f32>() / 4.0, q.iter().map(|p| p[1]).sum::<f32>() / 4.0)
        };
        // At the sun (x in NDC: 0.8 / aspect), at the centre, and mirrored.
        assert!((centre(0) - Vec2::new(0.4, 0.4)).length() < 1e-5);
        assert!(centre(1).length() < 1e-5);
        assert!((centre(2) + Vec2::new(0.4, 0.4)).length() < 1e-5);
        assert_eq!(m.3.len(), 18);
    }
}
