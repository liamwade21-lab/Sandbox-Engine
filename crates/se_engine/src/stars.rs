//! Catalog starfield: every luminous catalog star as a sprite at infinity.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy::math::{DQuat, DVec3};
use se_core::star::{StarKind, blackbody_rgb};

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::universe::UniverseSet;

/// Tolerance (pc) of the position hide of the active system's star in the star fields
/// ([`StarParams::hide`]); the catalog field, holding absolute f32 positions, widens it with distance.
const HIDE_EPS_PC: f32 = 2.0e-4;

pub const ATTRIBUTE_STAR_DATA: MeshVertexAttribute =
    MeshVertexAttribute::new("StarData", 0x5E57_0001, VertexFormat::Float32x4);
pub const ATTRIBUTE_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("SpriteCorner", 0x5E57_0002, VertexFormat::Float32x2);

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct StarParams {
    pub cam_pos: Vec4,
    pub params: Vec4,
    pub params2: Vec4,
    /// xyz render-space anchor, w = 1 to use it (otherwise drawn at infinity).
    pub anchor: Vec4,
    /// Motion blur ([`SpriteBlur`]), filled in for the star fields at infinity.
    pub blur: Vec4,
    pub blur_delta: Vec4,
    pub prev_view: Mat4,
    /// rgb: colour multiplier (a system's suns: the part of the disc no body hides, times the
    /// transmittance of the atmospheres in the way); 1 elsewhere.
    pub tint: Vec4,
    /// The active system draws its own stars: a star field hides the point at xyz (relative to the
    /// camera, pc) within w pc. Every other star stays drawn however close it is (SE hides only the
    /// barycentre pseudo-star, `star_point.glsl`); w = 0 hides nothing.
    pub hide: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct StarMaterial {
    #[uniform(0)]
    pub uniforms: StarParams,
}

impl Material for StarMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/stars.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/stars.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // Before everything else transparent, the galaxies included (see `SkyMaterial`).
        -5.0e15
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_STAR_DATA.at_shader_location(1),
            ATTRIBUTE_CORNER.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The glare sprite of a sun of the current system: the star shader, drawn after the skies (SE's
/// star sprite comes after the atmosphere, modulated by it), before clouds and rings.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SunSpriteMaterial {
    #[uniform(0)]
    pub uniforms: StarParams,
}

impl Material for SunSpriteMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/stars.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/stars.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // After the sky shells (render distances stay below ~3e8), before clouds and rings (1e10).
        5.0e9
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_STAR_DATA.at_shader_location(1),
            ATTRIBUTE_CORNER.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// How star and galaxy points are drawn: SE's "Star points style" (`StarPointMethod` 1-3; its
/// method 0, hardware points, is not offered). The blurs stretch each point along its path on
/// screen during the frame: from the camera's motion, or from its motion and rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PointStyle {
    Sprites,
    #[default]
    MotionBlur,
    MotionAndRotationBlur,
}

impl PointStyle {
    pub const ALL: [PointStyle; 3] = [PointStyle::Sprites, PointStyle::MotionBlur, PointStyle::MotionAndRotationBlur];

    pub fn label(self) -> &'static str {
        match self {
            PointStyle::Sprites => "Sprites",
            PointStyle::MotionBlur => "Motion blur",
            PointStyle::MotionAndRotationBlur => "Motion and rotation blur",
        }
    }

    /// SE's `StarPointMethod` number.
    pub fn method(self) -> u32 {
        match self {
            PointStyle::Sprites => 1,
            PointStyle::MotionBlur => 2,
            PointStyle::MotionAndRotationBlur => 3,
        }
    }

    pub fn from_method(m: u32) -> Self {
        match m {
            0 | 1 => PointStyle::Sprites,
            2 => PointStyle::MotionBlur,
            _ => PointStyle::MotionAndRotationBlur,
        }
    }
}

/// Star point settings, SE's `PointRendering` keys (exposed in the HUD).
#[derive(Resource, Debug, Clone)]
pub struct StarSettings {
    /// `StarMagnLimit`: a star this bright has the cutoff pixel brightness `StarBrightMin` (before
    /// the overbright), so stars show down to about `mag_limit + overbright`.
    pub mag_limit: f32,
    /// `PointOverbright` (magnitudes): extra brightness of every point, x 10^(0.4 overbright).
    pub overbright: f32,
    /// `PointScale`: size of the points (their Gaussian's 2 sigma^2 is PointScale^2 px^2 at 1080 rows).
    pub point_scale: f32,
    /// `StarColorSaturation`.
    pub saturation: f32,
    /// `PointDesaturation`: points fainter than this pixel brightness lose their colour.
    pub desaturation: f32,
    /// `StarColorShift`: star colours at the temperature x 10^-shift (+ red, - blue).
    pub color_shift: f32,
    /// `SunIllumSaturation`: colour saturation of starlight on planets (0-2).
    pub illum_saturation: f32,
    /// `IllumSaturationAuto`: the saturation follows the star's temperature instead.
    pub illum_saturation_auto: bool,
    pub point_style: PointStyle,
    /// Trail length factor (SE's `MotionBlurLength`; 1 = automatic).
    pub blur_length: f32,
    /// Window height / 1080 (SE's `LinkScaleToScreenRes`), kept up to date by `track_screen_scale`.
    pub screen_scale: f32,
    /// SE's zoom brightness `(FOV / DefaultFOV)^-1.5` (FUN_140572300): zooming in works like a
    /// telescope and shows fainter stars. Kept up to date from the camera by `track_zoom`.
    pub zoom: f32,
    /// `StarMagnLimit` as loaded at startup, which SE's ResetAllMagnCoef (P) goes back to.
    pub mag_limit_reset: f32,
    /// `GalaxyMagnLimit` / `NebulaMagnLimit` / `GlowMagnLimit` (main-def.cfg, 7): the deep-sky limits,
    /// read by `galaxies.rs` / `nebulae.rs` (far sprites' reference flux; the glow limit scales the sprite
    /// models, `galaxies::se_glow_bright`), and their startup values for ResetAllMagnCoef.
    pub galaxy_mag_limit: f32,
    pub nebula_mag_limit: f32,
    pub glow_mag_limit: f32,
    pub dso_mag_limit_reset: [f32; 3],
    /// SE's `StarFrustumClipFOV` (degrees, main-def.cfg): under this field of view SE loads the
    /// stars inside the view frustum only (FUN_14047b670), so zooming in loads fainter stars there.
    pub clip_fov: f32,
}

impl Default for StarSettings {
    fn default() -> Self {
        Self {
            mag_limit: 7.0,
            overbright: 1.0,
            point_scale: 1.0,
            saturation: 1.0,
            desaturation: 0.1,
            color_shift: 0.0,
            illum_saturation: 0.5,
            illum_saturation_auto: true,
            point_style: PointStyle::default(),
            blur_length: 1.0,
            screen_scale: 1.0,
            zoom: 1.0,
            mag_limit_reset: 7.0,
            galaxy_mag_limit: 7.0,
            nebula_mag_limit: 7.0,
            glow_mag_limit: 7.0,
            dso_mag_limit_reset: [7.0; 3],
            clip_fov: 30.0,
        }
    }
}

/// SE's range of `StarMagnLimit` (its config registration, FUN_140013a10), and the step of the
/// magnitude limit keys (its coefficient times or over 1.0990567, FUN_1404c27c0).
pub const MAG_LIMIT_MIN: f32 = -10.0;
pub const MAG_LIMIT_MAX: f32 = 20.0;
const MAG_LIMIT_STEP: f32 = 0.1;
/// SE's magnitude scale: a limit `m` is the brightness coefficient `0.05 exp((m + 0.2) / 1.0587)`
/// (FUN_1404c4610; Pogson's would be 2.5 / ln 10 = 1.0857). The port keeps Pogson's for drawing and
/// uses SE's only to show the limit as SE does.
const SE_MAG_SCALE: f32 = 1.058_736_2;

impl StarSettings {
    /// Faintest apparent magnitude drawn at the default field of view: where the pixel brightness
    /// falls to `StarBrightMin`.
    pub fn faint_limit(&self) -> f32 {
        self.mag_limit + self.overbright
    }

    /// The zoom brightness in magnitudes: zoomed in, stars this much fainter reach the cutoff.
    pub fn zoom_mag(&self) -> f32 {
        2.5 * self.zoom.log10()
    }

    /// The Camera toolbar's "Mag limit": `StarMagnLimit + 1.0587 ln(zoom)` (FUN_1404c27c0).
    pub fn shown_limit(&self) -> f32 {
        self.mag_limit + SE_MAG_SCALE * self.zoom.ln()
    }

    /// Typing the "Mag limit" sets the limits so it shows `v` at this zoom (FUN_14024d6e0).
    pub fn set_shown_limit(&mut self, v: f32) {
        self.mag_limit = (v - SE_MAG_SCALE * self.zoom.ln()).clamp(MAG_LIMIT_MIN, MAG_LIMIT_MAX);
    }

    /// Faintest apparent magnitude stars are loaded down to, zoomed: SE stops raising it at
    /// magnitude 20 (its loading coefficient's cap, FUN_1404c4610).
    pub fn zoomed_load_limit(&self) -> f32 {
        (self.mag_limit + self.zoom_mag()).min(MAG_LIMIT_MAX) + self.overbright
    }

    /// The view as a cone (unit axis in the universe frame, half-angle out to the frustum's
    /// corners) when the field of view is under `clip_fov`; `aspect` = width / height.
    pub fn view_cone(&self, cam: &FlyCamera, aspect: f64) -> Option<(DVec3, f64)> {
        (cam.fov_deg < self.clip_fov).then(|| {
            let half = (cam.fov_deg as f64 * 0.5).to_radians().tan();
            ((cam.rot * DVec3::NEG_Z).normalize(), (half * (1.0 + aspect * aspect).sqrt()).atan())
        })
    }
}

/// Width / height of the primary window.
pub fn window_aspect(windows: &Query<&Window, With<bevy::window::PrimaryWindow>>) -> f64 {
    windows.iter().next().map_or(16.0 / 9.0, |w| (w.width() / w.height().max(1.0)) as f64)
}

/// Loads `StarFrustumClipFOV` from SE's main-def.cfg.
fn load_star_config(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<StarSettings>) {
    if let Some(v) = main_def_f64(&cfg, "StarFrustumClipFOV") {
        s.clip_fov = v as f32;
    }
}

/// SE's `StarBrightMin`: cutoff pixel brightness of a point (main-def.cfg).
pub const STAR_BRIGHT_MIN: f32 = 0.03;

/// SE's `LinkScaleToScreenRes`: point sizes follow the window height (1x at 1080 rows).
fn track_screen_scale(windows: Query<&Window, With<bevy::window::PrimaryWindow>>, mut s: ResMut<StarSettings>) {
    let Some(w) = windows.iter().next() else { return };
    let k = (w.resolution.physical_height() as f32 / 1080.0).max(0.1);
    if (s.screen_scale - k).abs() > 1e-4 {
        s.screen_scale = k;
    }
}

/// Longest blur trail (px).
const MAX_TRAIL_PX: f32 = 300.0;

/// Per-frame motion blur state for the point sprites: the camera's step since the last frame
/// and its rotation then.
#[derive(Resource, Default)]
pub struct SpriteBlur {
    prev: Option<(DVec3, DQuat)>,
    prev_step_pc: f64,
    /// x: trail from motion (0/1), y: from rotation too (0/1), z: trail scale, w: longest trail (px).
    pub blur: Vec4,
    /// Camera displacement since the previous frame (pc, world axes).
    pub delta: Vec4,
    /// The previous frame's view rotation (view from world).
    pub prev_view: Mat4,
}

/// Marks the catalog starfield entity; its mesh origin is the Solar System.
#[derive(Component)]
pub struct CatalogStarfield {
    pub material: Handle<StarMaterial>,
}

pub struct StarsPlugin;

impl Plugin for StarsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((MaterialPlugin::<StarMaterial>::default(), MaterialPlugin::<SunSpriteMaterial>::default()))
            .init_resource::<StarSettings>()
            .init_resource::<SpriteBlur>()
            .add_systems(Startup, (load_star_config, build_catalog_starfield))
            .add_systems(PreUpdate, track_screen_scale)
            .add_systems(Update, mag_limit_keys)
            .add_systems(PostUpdate, (track_zoom, update_star_uniforms).chain().after(UniverseSet::Camera))
            .add_systems(
                PostUpdate,
                update_sprite_blur
                    .after(update_star_uniforms)
                    .after(crate::galaxies::update_galaxies)
                    .after(UniverseSet::FloatingOrigin)
                    .before(bevy::asset::AssetEventSystems),
            );
    }
}

/// Append one star sprite (4 vertices, 6 indices) to mesh buffers.
pub fn push_star(
    positions: &mut Vec<[f32; 3]>,
    data: &mut Vec<[f32; 4]>,
    corners: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    pos: [f32; 3],
    rgb: [f32; 3],
    abs_mag: f32,
) {
    let base = positions.len() as u32;
    for c in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
        positions.push(pos);
        data.push([rgb[0], rgb[1], rgb[2], abs_mag]);
        corners.push(c);
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

pub fn build_sprite_mesh(
    positions: Vec<[f32; 3]>,
    data: Vec<[f32; 4]>,
    corners: Vec<[f32; 2]>,
    indices: Vec<u32>,
) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(ATTRIBUTE_STAR_DATA, data)
        .with_inserted_attribute(ATTRIBUTE_CORNER, corners)
        .with_inserted_indices(Indices::U32(indices))
}

/// Visual colour of a star (black holes and neutron stars get special tints).
pub fn star_render_color(kind: StarKind, teff: f64) -> [f32; 3] {
    match kind {
        StarKind::BlackHole => [0.0, 0.0, 0.0],
        StarKind::NeutronStar => [0.75, 0.85, 1.0],
        _ => blackbody_rgb(teff),
    }
}

fn build_catalog_starfield(
    mut commands: Commands,
    data: Res<SeData>,
    settings: Res<StarSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let cat = &data.catalog;
    let n = cat.stars.len();
    let (mut positions, mut sdata, mut corners, mut indices) =
        (Vec::with_capacity(n * 4), Vec::with_capacity(n * 4), Vec::with_capacity(n * 4), Vec::with_capacity(n * 6));
    let mut count = 0;
    // A ForceAppMagn system's components share its catalog AppMagn (catalog.rs).
    let forced = cat.forced_system_offsets();
    for (s, off) in cat.stars.iter().zip(forced) {
        if s.barycenter || !s.has_position || s.hidden() {
            continue;
        }
        // A black hole shines only by its accretion disk's `Luminosity` (blackholes.rs).
        let (rgb, abs_mag) = if s.class.kind == StarKind::BlackHole {
            match crate::blackholes::disk_light(s) {
                Some(l) => l,
                None => continue,
            }
        } else {
            (star_render_color(s.class.kind, s.teff), s.abs_mag as f32 + off)
        };
        let p = s.pos_pc.as_vec3();
        push_star(&mut positions, &mut sdata, &mut corners, &mut indices, [p.x, p.y, p.z], rgb, abs_mag);
        count += 1;
    }
    info!("catalog starfield: {count} stars");
    let mesh = meshes.add(build_sprite_mesh(positions, sdata, corners, indices));
    let material = materials.add(StarMaterial {
        uniforms: uniforms_for(Vec3::ZERO, &settings),
    });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        NoFrustumCulling,
        CatalogStarfield { material },
    ));
}

pub fn uniforms_for(cam_rel_pc: Vec3, s: &StarSettings) -> StarParams {
    StarParams {
        cam_pos: cam_rel_pc.extend(0.0),
        params: star_params(s),
        params2: Vec4::new(1.0 / s.desaturation.max(1e-3), s.color_shift, s.saturation, 0.0),
        anchor: Vec4::ZERO,
        blur: Vec4::ZERO,
        blur_delta: Vec4::ZERO,
        prev_view: Mat4::IDENTITY,
        tint: Vec4::ONE,
        hide: Vec4::ZERO,
    }
}

/// `StarParams::params` of SE's star sprite (`star_sprite.glsl` Param): magnitude limit (raised by
/// the zoom, as SE multiplies its brightness coefficient by it, FUN_1404c51a0), overbright factor,
/// the Gaussian's 2 sigma^2 in px^2 (`FUN_14045ea50`: PointScale^2 x height/1080) and the cutoff
/// brightness.
fn star_params(s: &StarSettings) -> Vec4 {
    Vec4::new(
        s.mag_limit + s.zoom_mag(),
        10f32.powf(0.4 * s.overbright),
        s.point_scale * s.point_scale * s.screen_scale,
        STAR_BRIGHT_MIN,
    )
}

/// SE's zoom brightness (FUN_140572300, kept by its camera at +0x1390): `(FOV / DefaultFOV)^-1.5`,
/// 1 at the default field of view. It multiplies the brightness of the stars, planets and
/// clusters drawn as points (FUN_1404c51a0), and how far SE loads stars (FUN_1404a0e80).
fn track_zoom(cam: Res<FlyCamera>, mut s: ResMut<StarSettings>) {
    let z = (cam.fov_deg / crate::camera::DEFAULT_FOV_DEG).max(1e-9).powf(-1.5);
    if (s.zoom - z).abs() > z * 1e-6 {
        s.zoom = z;
    }
}

/// SE's magnitude limit keys: `]` / `[` raise or lower every limit by 0.1 (IncAllMagnCoef /
/// DecAllMagnCoef), Ctrl+`]` / `[` the stars' alone (IncStarMagnCoef / DecStarMagnCoef), and P
/// resets them to the startup values (ResetAllMagnCoef). The port has one limit for stars and
/// planets, so both act on it; Shift+`]` / `[` change the galaxies' and nebulae' limits
/// (`GalaxyMagnLimit`, `NebulaMagnLimit`, `GlowMagnLimit`; "Galaxies magnitude limit"), which `]` / `[`
/// raise too;
/// Ctrl+Shift (planets) is left unbound. Each shows the limit as a splash message (its wording
/// here is assumed).
fn mag_limit_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut egui: bevy_egui::EguiContexts,
    mut s: ResMut<StarSettings>,
    mut msg: ResMut<crate::messages::Messages>,
    i18n: Res<crate::i18n::I18n>,
) {
    if egui.ctx_mut().is_ok_and(|c| c.egui_wants_keyboard_input()) {
        return;
    }
    let held = |a, b| keys.pressed(a) || keys.pressed(b);
    let shift = held(KeyCode::ShiftLeft, KeyCode::ShiftRight);
    let ctrl = held(KeyCode::ControlLeft, KeyCode::ControlRight);
    let alt = held(KeyCode::AltLeft, KeyCode::AltRight);
    if alt || (shift && ctrl) {
        return;
    }
    let step = match (keys.just_pressed(KeyCode::BracketRight), keys.just_pressed(KeyCode::BracketLeft)) {
        (true, false) => MAG_LIMIT_STEP,
        (false, true) => -MAG_LIMIT_STEP,
        _ => 0.0,
    };
    let dso = |v: f32| (v + step).clamp(MAG_LIMIT_MIN, MAG_LIMIT_MAX);
    if shift {
        if step == 0.0 {
            return;
        }
        s.galaxy_mag_limit = dso(s.galaxy_mag_limit);
        s.nebula_mag_limit = dso(s.nebula_mag_limit);
        s.glow_mag_limit = dso(s.glow_mag_limit);
        msg.show(format!("{}: {:.1}", i18n.tr("Galaxies magnitude limit"), s.galaxy_mag_limit));
        return;
    }
    let name = if ctrl { "Star mag limit" } else { "Mag limit" };
    if step != 0.0 {
        s.mag_limit = (s.mag_limit + step).clamp(MAG_LIMIT_MIN, MAG_LIMIT_MAX);
        if !ctrl {
            s.galaxy_mag_limit = dso(s.galaxy_mag_limit);
            s.nebula_mag_limit = dso(s.nebula_mag_limit);
            s.glow_mag_limit = dso(s.glow_mag_limit);
        }
    } else if keys.just_pressed(KeyCode::KeyP) && !ctrl {
        s.mag_limit = s.mag_limit_reset;
        [s.galaxy_mag_limit, s.nebula_mag_limit, s.glow_mag_limit] = s.dso_mag_limit_reset;
    } else {
        return;
    }
    msg.show(format!("{}: {:.1}", i18n.tr(name), s.shown_limit()));
}

/// Motion blur of the star and galaxy points: how the camera moved and turned since the last
/// frame, written into every star field at infinity and the far galaxy sprites (after their own
/// uniform updates, which reset it). The star fields also get the active system's hide position.
#[allow(clippy::too_many_arguments)]
fn update_sprite_blur(
    time: Res<Time>,
    cam: Res<FlyCamera>,
    settings: Res<StarSettings>,
    active: Res<crate::system::ActiveSystem>,
    data: Res<SeData>,
    lensed: Option<Res<crate::blackholes::LensedHoles>>,
    mut blur: ResMut<SpriteBlur>,
    mut stars: ResMut<Assets<StarMaterial>>,
    mut far: ResMut<Assets<crate::galaxies::GalaxyFarMaterial>>,
) {
    let pos = cam.pos.to_parsecs();
    let dt = time.delta_secs().max(1e-4);
    let (mut delta, mut prev_rot) = (DVec3::ZERO, cam.rot);
    if let Some((p, r)) = blur.prev {
        delta = pos - p;
        // A jump (Go to without travel, a script) is not motion: its step comes out of nowhere.
        let step = delta.length();
        if step > blur.prev_step_pc * 50.0 + 1e-9 {
            delta = DVec3::ZERO;
        }
        blur.prev_step_pc = step;
        if r.angle_between(cam.rot) < 0.5 {
            prev_rot = r;
        }
    }
    blur.prev = Some((pos, cam.rot));
    let (motion, rotation) = match settings.point_style {
        PointStyle::Sprites => (0.0, 0.0),
        PointStyle::MotionBlur => (1.0, 0.0),
        PointStyle::MotionAndRotationBlur => (1.0, 1.0),
    };
    // Automatic length: the motion over 1/60 s, however high the frame rate.
    let scale = settings.blur_length * (1.0 / 60.0 / dt).max(1.0);
    blur.blur = Vec4::new(motion, rotation, scale, MAX_TRAIL_PX);
    blur.delta = delta.as_vec3().extend(0.0);
    blur.prev_view = Mat4::from_quat(prev_rot.as_quat()).inverse();
    // The active system's star (all components of a catalog system share its position in the fields).
    // A system without bodies (a lone black hole, whose disk shines as a field point) draws nothing
    // itself, so its point stays until the lens draws the disk.
    let lensed = |r: &u32| {
        lensed.as_ref().is_some_and(|l| {
            l.0.iter().any(|&h| data.catalog.system_root(se_core::catalog::ObjectRef::Star(h)).unwrap_or(h) == *r)
        })
    };
    let hide = active
        .root
        .filter(|r| !active.bodies.is_empty() || lensed(r))
        .and_then(|r| data.catalog.stars.get(r as usize))
        .map(|s| (s.pos_pc - pos).as_vec3());
    let eps_abs = (pos.length() as f32 * 4.0e-7).max(HIDE_EPS_PC);
    for (_, m) in stars.iter_mut() {
        let u = &mut m.uniforms;
        if u.anchor.w < 0.5 || (u.anchor.w > 1.5 && u.anchor.w < 2.5) {
            (u.blur, u.blur_delta, u.prev_view) = (blur.blur, blur.delta, blur.prev_view);
            // The point settings, for the fields set up once (procedural stars, cluster members).
            u.params = star_params(&settings);
            (u.params2.x, u.params2.y, u.params2.z) = (1.0 / settings.desaturation.max(1e-3), settings.color_shift, settings.saturation);
            let eps = if u.anchor.w < 0.5 { eps_abs } else { HIDE_EPS_PC };
            u.hide = hide.map_or(Vec4::ZERO, |h| h.extend(eps));
        }
    }
    for (_, m) in far.iter_mut() {
        let u = &mut m.uniforms;
        (u.blur, u.blur_delta, u.prev_view) = (blur.blur, blur.delta, blur.prev_view);
    }
}

/// SE's `StarFadeDuration` (main-def.cfg `PointRendering`, FUN_14048dd70): a newly generated block
/// of stars or galaxies brightens linearly from nothing over this many seconds (at least 1 ms).
pub fn block_fade_seconds(cfg: &crate::config::EngineConfig) -> f32 {
    main_def_f64(cfg, "StarFadeDuration").unwrap_or(0.5).max(0.001) as f32
}

/// A number from SE's `config/main-def.cfg` (read from the install).
pub fn main_def_f64(cfg: &crate::config::EngineConfig, key: &str) -> Option<f64> {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).ok()?;
    se_core::sc::parse(&text).find_deep(key).and_then(|i| i.f64())
}

fn update_star_uniforms(
    cam: Res<FlyCamera>,
    settings: Res<StarSettings>,
    q: Query<&CatalogStarfield>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let cam_pc = cam.pos.to_parsecs().as_vec3();
    for field in &q {
        if let Some(mut m) = materials.get_mut(&field.material) {
            // A bright star or planet in view does not dim the background: SE keeps the Milky
            // Way and the stars as they are from far out to a star's disc filling the view.
            m.uniforms = uniforms_for(cam_pc, &settings);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_fov(fov: f32) -> StarSettings {
        StarSettings { zoom: (fov / crate::camera::DEFAULT_FOV_DEG).powf(-1.5), ..Default::default() }
    }

    #[test]
    fn zoom_raises_the_shown_limit() {
        // SE's camera tutorial: a "Mag limit" of 10 is a StarMagnLimit of about 7.7 to 9.4 at a
        // field of view of 10 to 30 degrees.
        let mut s = at_fov(10.0);
        s.set_shown_limit(10.0);
        assert!((s.mag_limit - 7.6).abs() < 0.1, "{}", s.mag_limit);
        let mut s = at_fov(30.0);
        s.set_shown_limit(10.0);
        assert!((s.mag_limit - 9.36).abs() < 0.05, "{}", s.mag_limit);
        // At the default field of view nothing changes; zoomed in, stars are 1.5 x 2.5 log10 of
        // the zoom brighter.
        let s = at_fov(45.0);
        assert!((s.shown_limit() - 7.0).abs() < 1e-5 && s.zoom_mag().abs() < 1e-5);
        assert!((at_fov(4.5).zoom_mag() - 3.75).abs() < 1e-4);
        // Loading stops at magnitude 20.
        assert_eq!(at_fov(1e-4).zoomed_load_limit(), MAG_LIMIT_MAX + 1.0);
    }
}
