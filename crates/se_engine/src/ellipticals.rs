//! Elliptical galaxies up close, as SE draws them (`seGalaxySpriteModel::DrawBulge`, FUN_14029ce50, with
//! `galaxy_bulge.glsl`): types E0..E7 have no sprite model; an ellipsoid of `GalaxyEllipticalScale` (2.4)
//! times the catalog radius, its x and z axes shortened to `1 - (index - 1) / 10` (FUN_140168210), shows on
//! its back faces the pre-integrated density table `textures/common/galaxy_bulge.tif`
//! (`PrecomputedDensityFunc`, `seGalaxyCatalog::LoadBulgeLookupTexture`) looked up by the ray's offset from
//! the centre and its half length inside, in `GalaxyEllipticalColor` and SE's colour correction
//! (`docs/formats/galaxy_model.md`, section 8). Like a sprite model it fades in as the far sprite fades out
//! (the `resident` list).

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use se_core::galaxy_model::hubble_type_index;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::galaxies::{
    GalaxySettings, GalaxyState, MAG_REF_OFFSET, angular_px, main_def_doc, profile_type, se_bright_term,
    se_glow_bright, update_galaxies,
};

/// Ellipticals drawn at once (the largest on screen).
const MAX_VOLUMES: usize = 16;
/// The bounding sphere of the faceted mesh, in ellipsoid radii.
const BOUND: f32 = 1.03;
/// SE's distance reddening (FUN_140299fe0): `(1 - k, 1 - 2k, 1 - 4k)` with `k = d / 4.227e9 pc`.
const REDDEN_PC: f64 = 4.227e9;

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct BulgeParams {
    /// xyz: galaxy centre relative to the camera (pc), w: ellipsoid radius (pc)
    pub center_rel: Vec4,
    /// Columns of the universe -> galaxy rotation.
    pub rot0: Vec4,
    pub rot1: Vec4,
    pub rot2: Vec4,
    /// xyz: the ellipsoid's axes in radii, w: unused
    pub axes: Vec4,
    /// SE's `Color` uniform
    pub color: Vec4,
    /// SE's `ColorEffects`: (brightness - 1, contrast, saturation - 1, 1 / gamma)
    pub effects: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct EllipticalMaterial {
    #[uniform(0)]
    pub u: BulgeParams,
    /// SE's `PrecomputedDensityFunc` (`galaxy_bulge.tif`).
    #[texture(1)]
    #[sampler(2)]
    pub table: Handle<Image>,
}

impl Material for EllipticalMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/galaxy_elliptical.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/galaxy_elliptical.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn depth_bias(&self) -> f32 {
        // Sorts with the sprite-model emission (see `GalaxyModelMaterial::depth_bias`).
        -3.0e15
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // Back faces, as SE (glCullFace(GL_FRONT)): one fragment per pixel from outside, all around inside.
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// main-def.cfg `GalaxyGenerator` and `PostProcess` keys of the bulge draw.
#[derive(Resource, Clone, Debug)]
pub struct BulgeSettings {
    /// `GalaxyEllipticalScale` (2.4): ellipsoid radius in catalog radii.
    pub scale: f32,
    /// `GalaxyEllipticalColor` (1 0.9 0.7 0).
    pub color: Vec4,
    /// `GalaxyEllipticalBright` (1): its square root in HDR mode.
    pub bright: f32,
    /// `GalaxyEllipticalGamma` (0.7, "replaces DSOGamma").
    pub gamma: f32,
    /// `DSOBrightness`, `DSOContrast`, `DSOSaturation` (1).
    pub dso: Vec3,
}

impl Default for BulgeSettings {
    fn default() -> Self {
        Self { scale: 2.4, color: Vec4::new(1.0, 0.9, 0.7, 0.0), bright: 1.0, gamma: 0.7, dso: Vec3::ONE }
    }
}

#[derive(Resource, Default)]
pub(crate) struct Volumes {
    mesh: Handle<Mesh>,
    table: Handle<Image>,
    slots: Vec<(Entity, Handle<EllipticalMaterial>)>,
    /// The catalog galaxies (in order) of profile 0 without a sprite model, among the first `scanned`,
    /// of the catalog at `cat_ptr`.
    candidates: Vec<usize>,
    scanned: usize,
    cat_ptr: usize,
}

pub struct EllipticalsPlugin;

impl Plugin for EllipticalsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<EllipticalMaterial>::default())
            .init_resource::<Volumes>()
            .init_resource::<BulgeSettings>()
            .add_systems(Startup, setup_bulge)
            .add_systems(PostUpdate, update_volumes.after(update_galaxies));
    }
}

/// Converts to IEEE half precision (the table is R16Float, which filters on every device).
fn f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = b & 0x7f_ffff;
    if exp <= 0 {
        // Subnormal half (below 6.1e-5): the mantissa in units of 2^-24, rounded.
        if exp < -10 {
            return sign;
        }
        let shift = (14 - exp) as u32;
        return sign | (((man | 0x80_0000) + (1 << (shift - 1))) >> shift) as u16;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    sign | (((exp as u32) << 10) + ((man + 0x1000) >> 13)) as u16
}

fn setup_bulge(
    cfg: Res<crate::config::EngineConfig>,
    data: Res<SeData>,
    mut settings: ResMut<BulgeSettings>,
    mut vols: ResMut<Volumes>,
    mut images: ResMut<Assets<Image>>,
) {
    if let Some(doc) = main_def_doc(&cfg) {
        let num = |k: &str| doc.find_deep(k).and_then(|i| i.f64()).map(|v| v as f32);
        let s = &mut *settings;
        s.scale = num("GalaxyEllipticalScale").unwrap_or(s.scale);
        s.bright = num("GalaxyEllipticalBright").unwrap_or(s.bright);
        s.gamma = num("GalaxyEllipticalGamma").unwrap_or(s.gamma);
        if let Some(c) = doc.find_deep("GalaxyEllipticalColor").map(|i| i.nums()).filter(|c| c.len() >= 3) {
            s.color = Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c.get(3).copied().unwrap_or(0.0) as f32);
        }
        s.dso = Vec3::new(
            num("DSOBrightness").unwrap_or(s.dso.x),
            num("DSOContrast").unwrap_or(s.dso.y),
            num("DSOSaturation").unwrap_or(s.dso.z),
        );
    }
    let table = data.vfs.read("textures/common/galaxy_bulge.tif").ok().and_then(|b| se_core::tiff::read_gray_f32(&b));
    let Some((w, h, px)) = table else {
        warn!("galaxy_bulge.tif not found or unreadable: elliptical galaxies are not drawn up close");
        return;
    };
    // The file's last row is texture row 0, as SE's loader hands it to OpenGL: depth 1 (the full chord
    // through the centre) then reads the first, saturated rows.
    let bytes: Vec<u8> =
        px.chunks_exact(w as usize).rev().flat_map(|row| row.iter().flat_map(|&v| f16_bits(v).to_le_bytes())).collect();
    let mut img = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::R16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    vols.table = images.add(img);
}

/// SE's axis ratio of an elliptical (FUN_140168210): x and z are `1 - (index - 1) / 10` of y, index the
/// Hubble type index (E0 1 .. E7 8; 0 for an unrecognised type).
fn axis_ratio(kind: &str) -> f32 {
    1.0 - (hubble_type_index(kind) as f32 - 1.0) / 10.0
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_volumes(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<GalaxySettings>,
    bulge: Res<BulgeSettings>,
    photo: Option<Res<crate::exposure::PhotoMode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    gs: Option<ResMut<GalaxyState>>,
    mut vols: ResMut<Volumes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<EllipticalMaterial>>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(mut gs) = gs else { return };
    let gs = &mut *gs;
    if vols.table == Handle::default() {
        return;
    }
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();
    // The ellipsoid's radius: the catalog radius times GalaxyEllipticalScale (promoted procedural galaxies
    // already carry it, `ProcGalaxy::display_radius_pc`).
    let radius = |rec: &se_core::catalog::GalaxyRecord| {
        if rec.source == "procedural" { rec.radius_pc } else { rec.radius_pc * bulge.scale as f64 }
    };

    // The ellipticals SE draws (fade-in over 2..5 px of the ellipsoid's radius), largest first.
    let mut wanted: Vec<(f64, usize, f32)> = Vec::new();
    if settings.enabled {
        // Which galaxies can be ellipticals is fixed once a galaxy has its model choice (catalog
        // galaxies are only appended): those are listed once, the rest are tested each frame.
        let ptr = Arc::as_ptr(&data.catalog) as usize;
        if vols.cat_ptr != ptr || vols.scanned > cat.galaxies.len() {
            (vols.cat_ptr, vols.scanned) = (ptr, 0);
            vols.candidates.clear();
        }
        let known = cat.galaxies.len().min(gs.model_count());
        for g in vols.scanned..known {
            if profile_type(&cat.galaxies[g]) == 0 && !gs.has_model(g) {
                vols.candidates.push(g);
            }
        }
        vols.scanned = vols.scanned.max(known);
        for g in vols.candidates.iter().copied().chain(vols.scanned..cat.galaxies.len()) {
            let rec = &cat.galaxies[g];
            if profile_type(rec) != 0 || gs.has_model(g) {
                continue;
            }
            let px = angular_px(radius(rec), (rec.pos_pc - cam_pc).length(), focal);
            let fade = ((px as f32 - 2.0) / 3.0).clamp(0.0, 1.0);
            if fade > 0.0 {
                wanted.push((px, g, fade));
            }
        }
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        wanted.truncate(MAX_VOLUMES);
    }

    if vols.mesh == Handle::default() {
        vols.mesh = meshes.add(Sphere::new(BOUND).mesh().ico(3).expect("icosphere"));
    }
    while vols.slots.len() < wanted.len() {
        let mat = mats.add(EllipticalMaterial {
            u: BulgeParams {
                center_rel: Vec4::ZERO,
                rot0: Vec4::X,
                rot1: Vec4::Y,
                rot2: Vec4::Z,
                axes: Vec4::ONE,
                color: Vec4::ZERO,
                effects: Vec4::ZERO,
            },
            table: vols.table.clone(),
        });
        let e = commands
            .spawn((Mesh3d(vols.mesh.clone()), MeshMaterial3d(mat.clone()), Transform::IDENTITY, Visibility::Hidden, NoFrustumCulling))
            .id();
        vols.slots.push((e, mat));
    }

    // SE's ColorEffects (FUN_14029ce50): the DSO brightness, contrast and saturation, the ellipticals' gamma.
    let effects = Vec4::new(bulge.dso.x - 1.0, bulge.dso.y, bulge.dso.z - 1.0, 1.0 / bulge.gamma.max(0.01));
    // GalaxyEllipticalBright, square-rooted in HDR mode (PhotoMode 0).
    let bright = if photo.is_none_or(|p| p.0 == 0) { bulge.bright.max(0.0).sqrt() } else { bulge.bright };
    let glow = se_glow_bright(settings.glow_mag_limit, settings.glow_bright_min, settings.sprite_bright);
    let limit = settings.mag_ref - MAG_REF_OFFSET;
    // This frame's far-sprite uniforms (`update_galaxies`), which `galaxies::write_far` then puts in place.
    let mut far = gs.far_u.as_mut();
    for (i, (e, mat)) in vols.slots.iter().enumerate() {
        let Some(&(px, g, fade)) = wanted.get(i) else {
            if let Ok(mut v) = vis.get_mut(*e) {
                v.set_if_neq(Visibility::Hidden);
            }
            continue;
        };
        let rec = &cat.galaxies[g];
        let d = (rec.pos_pc - cam_pc).length();
        // FUN_140299fe0: p1 = fade-in x brightness term (with the intergalactic vision); the colour then
        // takes hdrGalaxySpriteBright x GlowBrightMin coef(GlowMagnLimit), the galaxy's ModelBright and
        // the large-size fade 1 - 1e-7 px.
        let m_app = rec.vis_mag + 5.0 * (d.max(1e-9) / 10.0).log10();
        let p1 = fade * se_bright_term(limit, m_app as f32, settings.bright_min * gs.gain, settings.bright_min);
        let p3 = (1.0 - px * 1.0e-7).clamp(0.0, 1.0) as f32;
        let k = (d / REDDEN_PC) as f32;
        let redden = Vec3::new(1.0 - k, 1.0 - 2.0 * k, 1.0 - 4.0 * k).max(Vec3::ZERO);
        let b = bright * p1 * glow * p3 * rec.model_bright as f32;
        let q = axis_ratio(&rec.kind);
        let inv = Mat3::from_quat(rec.orientation.inverse().as_quat());
        // A hidden volume draws nothing: its material is written again on the frame it shows.
        if p1 > 0.0
            && let Some(mut m) = mats.get_mut(mat)
        {
            m.u = BulgeParams {
                center_rel: (rec.pos_pc - cam_pc).as_vec3().extend(radius(rec) as f32),
                rot0: inv.x_axis.extend(0.0),
                rot1: inv.y_axis.extend(0.0),
                rot2: inv.z_axis.extend(0.0),
                axes: Vec4::new(q, 1.0, q, 0.0),
                color: (redden * b * bulge.color.truncate()).extend(bulge.color.w),
                effects,
            };
        }
        if let Ok(mut v) = vis.get_mut(*e) {
            v.set_if_neq(if p1 > 0.0 { Visibility::Visible } else { Visibility::Hidden });
        }
        // The far sprite hides behind the ellipsoid as it fades in.
        gs.resident.push((g, fade));
        if let Some(far) = far.as_mut()
            && let Some(slot) = far.resident.iter_mut().find(|s| s.x < 0.0)
        {
            *slot = Vec4::new(g as f32, fade, 0.0, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_and_axis_ratios() {
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(0.0), 0);
        // 1e-4 = 1.6384 x 2^-14: exponent 1, mantissa 654. 3e-5 is subnormal: 3e-5 / 2^-24 = 503.3.
        assert_eq!(f16_bits(1.0e-4), 1678);
        assert_eq!(f16_bits(3.0e-5), 503);
        assert!((axis_ratio("E0") - 1.0).abs() < 1e-6);
        assert!((axis_ratio("E7") - 0.3).abs() < 1e-6);
    }
}
