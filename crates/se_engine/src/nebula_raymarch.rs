//! Ray-marched nebulae: the `CustomModel`s of `models/custom/*.cfg` (`se_core::custom_model`,
//! docs/formats/nebula_raymarch.md). SE draws every nebula without a named sprite model this way, the
//! procedural ones included.
//!
//! `galaxy_nebula_raymarch.wesl` is SE's `nebula_raymarch.glsl` translated: the script's keys fill SE's
//! uniform block `CustomModelUniforms2` (`uFlags`, `uIntData`, `uFloatData[39]`, laid out by
//! `nebula_raymarch_uniforms.glh`, here `FLAGS` and `FLOATS`), and the draw's own uniforms (`uShapeDims`,
//! `uRandomize`, `uParamsImg`) come from the nebula as in SE's custom-model draw (FUN_1404877f0,
//! FUN_140488a20).
//!
//! Like the elliptical galaxies (`ellipticals.rs`), a sphere around the model is drawn at infinity and the
//! fragment shader marches along the view ray, so the volume works from outside and from inside.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use se_core::custom_model::{BaseShape, CustomModel, CustomModelSet};
use se_core::star_octree::sphere_meets_cone;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::galaxies::{angular_px, nebula_model_bias, se_bright_term, se_glow_bright};
use crate::nebulae::{NebulaSettings, NebulaState, ResidentNebulae, fade_in, update_nebulae};

/// Volumes drawn at once (the largest in view), since procedural nebulae fill the view inside a galaxy.
/// With the sprite models (`NebulaSettings::max_models`) they fit the far sprites' `RESIDENT_SLOTS`.
const MAX_VOLUMES: usize = 12;
/// `uFloatData` vec4s of SE's `CustomModelUniforms2`; must match `galaxy_nebula_raymarch.wesl`.
pub const FLOAT_SLOTS: usize = 39;

/// `uFlags.x` bits, bit `i` = `1 << i` (nebula_raymarch_uniforms.glh).
const FLAGS: [&str; 23] = [
    "Single_star",
    "Render_star",
    "Render_star_bloom",
    "Gas_emission_enabled",
    "Emission_gas_1",
    "Emission_gas_2",
    "Emission_gas_3",
    "NebScaleAbsBoth",
    "NebScaleAbsX",
    "NebScaleAbsY",
    "NebulaFormPlane",
    "NebulaFormDsk",
    "NebulaFormCyl",
    "NebulaFormSph",
    "NebulaFormCapsule",
    "NebulaFormTorus",
    "NebulaFormSpir",
    "NebulaFormNoise",
    "NebulaFormFBMNoise",
    "NebulaFormSpirNoise",
    "NebulaFormSpirIQNoise",
    "NebulaFormSpirNoise3D",
    "NebulaMapTwist",
];

/// `uFloatData`: (key, vec4, first component, components) (nebula_raymarch_uniforms.glh). Every shipped
/// model sets every key.
#[rustfmt::skip]
const FLOATS: &[(&str, usize, usize, usize)] = &[
    ("NebulaFormSpirParam", 0, 0, 4),
    ("sum_transparency_break", 1, 0, 3), ("Internal_scale", 1, 3, 1),
    ("Light_Color_11", 2, 0, 3), ("td_break_value", 2, 3, 1),
    ("Ambient_Light_Color_11", 3, 0, 3), ("Light_intensivity", 3, 3, 1),
    ("Star_position", 4, 0, 3), ("Light_radius_single_star", 4, 3, 1),
    ("Nebula_emission_color_1", 5, 0, 3), ("Ambient_Light_intensivity", 5, 3, 1),
    ("Emission_gas_1_direction", 6, 0, 3), ("star_empty_sphere_radius", 6, 3, 1),
    ("Emission_gas_1_coef_0", 7, 0, 3), ("Star_field_scale", 7, 3, 1),
    ("Nebula_emission_color_2", 8, 0, 3), ("Star_light_coef_1", 8, 3, 1),
    ("Emission_gas_2_direction", 9, 0, 3), ("Star_light_coef_2", 9, 3, 1),
    ("Emission_gas_2_coef_0", 10, 0, 3), ("Bloom_light_coef_1", 10, 3, 1),
    ("Nebula_emission_color_3", 11, 0, 3), ("Bloom_light_coef_2", 11, 3, 1),
    ("Emission_gas_3_direction", 12, 0, 3), ("Dust_volume_height", 12, 3, 1),
    ("Emission_gas_3_coef_0", 13, 0, 3), ("dust_absorption_factor", 13, 3, 1),
    ("NebulaCentPos", 14, 0, 3), ("dust_scattering_factor", 14, 3, 1),
    ("NebulaFormPlaneXYZ", 15, 0, 3), ("Dust_volume_density_factor", 15, 3, 1),
    ("NebulaFormDskParam", 16, 0, 3), ("Albedo", 16, 3, 1),
    ("NebulaFormCapsulePos_1", 17, 0, 3), ("Forward_scattering_g", 17, 3, 1),
    ("NebulaFormCapsulePos_2", 18, 0, 3), ("Backward_scattering_g", 18, 3, 1),
    ("NebScaleCoef", 19, 0, 2), ("SpirNoiseNudge", 19, 2, 2),
    ("SpirNoiseAmount", 20, 0, 2), ("SpirNoiseIter", 20, 2, 2),
    ("SpirNoiseCoef", 21, 0, 2), ("Nudge_3D_2", 21, 2, 2),
    ("Noise_amount_3D_2", 22, 0, 2), ("Frequency_iteration_count_3D_2", 22, 2, 2),
    ("SpirNoise3DCoef_2", 23, 0, 2), ("NebulaFormPlaneSign", 23, 2, 2),
    ("NebulaFormDskSign", 24, 0, 2), ("NebulaFormCylSign", 24, 2, 2),
    ("NebulaFormCylParam", 25, 0, 2), ("NebulaFormSphSign", 25, 2, 2),
    ("NebulaFormCapsuleSign", 26, 0, 2), ("NebulaFormTorusSign", 26, 2, 2),
    ("NebulaFormSpirSign", 27, 0, 2), ("NebulaFormNoiseSign", 27, 2, 2),
    ("NebulaFormFBMNoiseSign", 28, 0, 2), ("NebulaFormSpirNoiseSign", 28, 2, 2),
    ("NebulaFormSpirIQNoiseSign", 29, 0, 2), ("NebulaFormSpirNoise3DSign", 29, 2, 2),
    ("NebulaMapThickCoef", 30, 0, 2), ("HG_mix_coef", 30, 2, 1), ("Gas_volume_height", 30, 3, 1),
    ("gas_scattering_factor", 31, 0, 1), ("gas_density_factor", 31, 1, 1),
    ("Emission_gas_1_coef_1", 31, 2, 1), ("Emission_gas_1_coef_2", 31, 3, 1),
    ("Emission_gas_1_coef_3", 32, 0, 1), ("Emission_gas_2_coef_1", 32, 1, 1),
    ("Emission_gas_2_coef_2", 32, 2, 1), ("Emission_gas_2_coef_3", 32, 3, 1),
    ("Emission_gas_3_coef_1", 33, 0, 1), ("Emission_gas_3_coef_2", 33, 1, 1),
    ("Emission_gas_3_coef_3", 33, 2, 1), ("Emission_intensivity_coef", 33, 3, 1),
    ("NebScale", 34, 0, 1), ("NebulaFormPlaneHeight", 34, 1, 1),
    ("NebulaFormSphRad", 34, 2, 1), ("NebulaFormCapsuleRadius", 34, 3, 1),
    ("NebulaFormTorusRadius_1", 35, 0, 1), ("NebulaFormTorusRadius_2", 35, 1, 1),
    ("NebulaFormNoiseCoef1", 35, 2, 1), ("NebulaFormNoiseCoef2", 35, 3, 1),
    ("NebulaFormFBMNoiseCoef", 36, 0, 1), ("NebulaFormSpirNoiseCoef1", 36, 1, 1),
    ("NebulaFormSpirNoiseCoef2", 36, 2, 1), ("NebulaFormSpirNoiseCoef3", 36, 3, 1),
    ("NebulaFormSpirIQNoiseCoef1", 37, 0, 1), ("NebulaFormSpirIQNoiseCoef2", 37, 1, 1),
    ("NebulaFormSpirIQNoiseCoef3", 37, 2, 1), ("NebulaFormSpirIQNoiseCoef4", 37, 3, 1),
    ("NebulaFormSpirNoise3DCoef", 38, 0, 1), ("NebulaMapTwistCoef1", 38, 1, 1),
    ("NebulaMapTwistCoef2", 38, 2, 1),
];

/// Which catalog nebula is ray-marched with which model (filled by `nebulae::setup_nebulae`).
#[derive(Resource, Default, Clone)]
pub struct RaymarchAssign {
    pub set: Arc<CustomModelSet>,
    pub model_of: Vec<Option<usize>>,
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct RaymarchParams {
    /// xyz: nebula centre relative to the camera (pc), w: radius (pc)
    pub center_rel: Vec4,
    /// Columns of the universe -> model rotation.
    pub rot0: Vec4,
    pub rot1: Vec4,
    pub rot2: Vec4,
    /// x: 0 box, 1 sphere, 2 cylinder; yzw: `uShapeDims` (`BaseShapeDims`)
    pub shape: Vec4,
    /// xyz: `Scale`, w: radius of the drawn sphere (nebula radii)
    pub scale: Vec4,
    /// xyz: `uRandomize` (the nebula's `Randomize` + the model's)
    pub rnd: Vec4,
    /// x: colour brightness, y: alpha brightness (`uParamsImg.zw`)
    pub img: Vec4,
    /// x: `uFlags.x`, y: `Raymarch_step_count` (`uIntData[0].x`)
    pub ints: UVec4,
    pub f: [Vec4; FLOAT_SLOTS],
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct RaymarchMaterial {
    #[uniform(0)]
    pub u: RaymarchParams,
    /// SE's `IqNoiseTex` (`blackholes::noise_texture`).
    #[texture(1)]
    #[sampler(2)]
    pub noise: Handle<Image>,
    /// `galaxies::nebula_model_bias` of the nebula's distance, so nearer volumes draw over farther ones.
    pub depth_bias: f32,
}

impl Material for RaymarchMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/galaxy_nebula_raymarch.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/galaxy_nebula_raymarch.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // Light is added and dust hides what lies behind it (alpha = 1 - transmittance).
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        // With the nebula sprite models, after the galaxy models, far to near.
        self.depth_bias
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// Packs a model's keys into SE's `uFlags` / `uIntData` (`ints`) and `uFloatData` (`f`).
pub fn pack(m: &CustomModel) -> (UVec4, [Vec4; FLOAT_SLOTS]) {
    let mut flags = 0u32;
    for (i, k) in FLAGS.iter().enumerate() {
        if m.flag(k) {
            flags |= 1 << i;
        }
    }
    let mut f = [Vec4::ZERO; FLOAT_SLOTS];
    for &(k, slot, c, n) in FLOATS {
        let v: Vec<f64> = match n {
            1 => vec![m.num(k)],
            2 => m.pair(k).to_vec(),
            3 => m.v3(k).to_vec(),
            _ => m.v4(k).to_vec(),
        };
        for (j, x) in v.into_iter().enumerate() {
            f[slot][c + j] = x as f32;
        }
    }
    // A model without Internal_scale falls back to its largest dimension.
    f[1].w = m.internal_scale() as f32;
    (UVec4::new(flags, m.num("Raymarch_step_count").max(1.0) as u32, 0, 0), f)
}

/// The nebula's draw factor (FUN_140488a20, custom models of nebulae): the size fade-in
/// `clamp((px - 4) / 3)` times SE's brightness term (`clamp(NebulaBrightMin coef(NebulaMagnLimit) L / d^2 -
/// NebulaBrightMin)`, as for the sprite models), `px` the nebula's own radius on screen.
fn draw_fade(px: f64, bright: f32) -> f32 {
    fade_in(px) * bright
}

#[derive(Resource, Default)]
pub(crate) struct Volumes {
    mesh: Handle<Mesh>,
    noise: Handle<Image>,
    slots: Vec<(Entity, Handle<RaymarchMaterial>)>,
    /// [`ModelConsts`] of every model of the set they were made from.
    consts: Option<(Arc<CustomModelSet>, Vec<ModelConsts>)>,
}

/// What `update_volumes` reads of a model every frame, computed once per model set: each is a pure
/// function of the model's script (a key lookup each).
struct ModelConsts {
    bound: f64,
    ints: UVec4,
    f: [Vec4; FLOAT_SLOTS],
    randomize: Vec3,
}

impl ModelConsts {
    fn of(m: &CustomModel) -> Self {
        let (ints, f) = pack(m);
        let r = m.v3("Randomize");
        Self { bound: m.bound_radius(), ints, f, randomize: Vec3::new(r[0] as f32, r[1] as f32, r[2] as f32) }
    }
}

pub struct NebulaRaymarchPlugin;

impl Plugin for NebulaRaymarchPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<RaymarchMaterial>::default())
            .init_resource::<RaymarchAssign>()
            .init_resource::<Volumes>()
            .add_systems(PostUpdate, update_volumes.after(update_nebulae));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_volumes(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    assign: Res<RaymarchAssign>,
    settings: Res<NebulaSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    st: Option<ResMut<NebulaState>>,
    mut resident: ResMut<ResidentNebulae>,
    mut vols: ResMut<Volumes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<RaymarchMaterial>>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(mut st) = st else { return };
    let vols = &mut *vols;
    if vols.consts.as_ref().is_none_or(|(set, _)| !Arc::ptr_eq(set, &assign.set)) {
        vols.consts = Some((assign.set.clone(), assign.set.models.iter().map(ModelConsts::of).collect()));
    }
    let consts = &vols.consts.as_ref().expect("set above").1;
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();
    // The cone around the screen's corners: volumes outside it would take the slots of visible ones.
    let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| (w.width() / w.height().max(1.0)) as f64);
    let half_fov = (cam.fov_deg as f64 * 0.5).to_radians().tan();
    let view = ((cam.rot * DVec3::NEG_Z).normalize(), (half_fov * (1.0 + aspect * aspect).sqrt()).atan());

    // The ray-marched nebulae in view and big enough on screen, largest first.
    let mut wanted: Vec<(f64, usize, usize, f32)> = Vec::new();
    if settings.enabled {
        for (n, m) in assign.model_of.iter().enumerate() {
            let (Some(m), Some(rec)) = (m, cat.nebulae.get(n)) else { continue };
            let bound = rec.radius_pc * consts[*m].bound.max(1.0) * 1.1;
            if !sphere_meets_cone(cam_pc, view, rec.pos_pc, bound) {
                continue;
            }
            let d = (rec.pos_pc - cam_pc).length();
            let px = angular_px(rec.radius_pc, d, focal);
            let m_app = rec.abs_mag + 5.0 * (d.max(1e-9) / 10.0).log10();
            let fade = draw_fade(px, se_bright_term(settings.mag_limit, m_app as f32, settings.bright_min, settings.bright_min));
            if fade > 0.0 {
                wanted.push((px, n, *m, fade));
            }
        }
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        wanted.truncate(MAX_VOLUMES);
    }
    debug!(
        "ray-marched volumes (name, model, px, fade, abs mag): {:?}",
        wanted
            .iter()
            .map(|w| (cat.nebulae[w.1].names[0].as_str(), assign.set.models[w.2].name.as_str(), w.0 as i64, w.3, cat.nebulae[w.1].abs_mag as f32))
            .collect::<Vec<_>>()
    );

    if vols.mesh == Handle::default() {
        vols.mesh = meshes.add(Sphere::new(1.03).mesh().ico(3).expect("icosphere"));
        let texels: Vec<u8> = crate::blackholes::noise_texture(0x1A_9E37).into_iter().flatten().collect();
        let size = Extent3d { width: 16, height: 16, depth_or_array_layers: 1 };
        let mut noise = Image::new(size, TextureDimension::D2, texels, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
        noise.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        vols.noise = images.add(noise);
    }
    while vols.slots.len() < wanted.len() {
        let mat = mats.add(RaymarchMaterial {
            u: RaymarchParams {
                center_rel: Vec4::ZERO,
                rot0: Vec4::X,
                rot1: Vec4::Y,
                rot2: Vec4::Z,
                shape: Vec4::ZERO,
                scale: Vec4::ONE,
                rnd: Vec4::ZERO,
                img: Vec4::ZERO,
                ints: UVec4::ZERO,
                f: [Vec4::ZERO; FLOAT_SLOTS],
            },
            noise: vols.noise.clone(),
            depth_bias: crate::galaxies::NEBULA_MODEL_BIAS,
        });
        let e = commands
            .spawn((Mesh3d(vols.mesh.clone()), MeshMaterial3d(mat.clone()), Transform::IDENTITY, Visibility::Hidden, NoFrustumCulling))
            .id();
        vols.slots.push((e, mat));
    }

    // This frame's far-sprite uniforms (`update_nebulae`), which `nebulae::write_far` then puts in place.
    let mut far = st.far_u.as_mut();
    // SE's colour brightness (FUN_1404877f0, drawn straight to the frame): model Bright x 10 x
    // DAT_14080b5a4 (GlowBrightMin x coef(GlowMagnLimit)) x the draw factor x hdrNebulaRaymarchBright; the
    // alpha brightness is the draw factor.
    let glow = 10.0 * se_glow_bright(settings.glow_mag_limit, settings.glow_bright_min, settings.raymarch_bright);
    for (i, (e, mat)) in vols.slots.iter().enumerate() {
        let Some(&(_, n, m, fade)) = wanted.get(i) else {
            if let Ok(mut v) = vis.get_mut(*e) {
                *v = Visibility::Hidden;
            }
            continue;
        };
        let rec = &cat.nebulae[n];
        let model = &assign.set.models[m];
        let c = &consts[m];
        let inv = Mat3::from_quat(rec.orientation.inverse().as_quat());
        let shape = match model.shape {
            BaseShape::Box => 0.0,
            BaseShape::Sphere => 1.0,
            BaseShape::Cylinder => 2.0,
        };
        let v3 = |a: [f64; 3]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
        if let Some(mut mm) = mats.get_mut(mat) {
            let u = RaymarchParams {
                center_rel: (rec.pos_pc - cam_pc).as_vec3().extend(rec.radius_pc as f32),
                rot0: inv.x_axis.extend(0.0),
                rot1: inv.y_axis.extend(0.0),
                rot2: inv.z_axis.extend(0.0),
                shape: Vec4::new(shape, model.dims[0] as f32, model.dims[1] as f32, model.dims[2] as f32),
                scale: v3(model.scale).extend(c.bound as f32),
                rnd: (Vec3::from(rec.randomize) + c.randomize).extend(0.0),
                img: Vec4::new(model.bright as f32 * glow * fade, fade, 0.0, 0.0),
                ints: c.ints,
                f: c.f,
            };
            let bias = nebula_model_bias((rec.pos_pc - cam_pc).length());
            // Written only when it changes: a changed material is prepared again.
            if !crate::stars::same_bytes(&mm.u, &u) || mm.depth_bias.to_bits() != bias.to_bits() {
                mm.u = u;
                mm.depth_bias = bias;
            }
        }
        if let Ok(mut v) = vis.get_mut(*e) {
            *v = if fade > 0.001 { Visibility::Visible } else { Visibility::Hidden };
        }
        // The far sprite hides behind the volume as it fades in (a procedural nebula's is streamed by
        // `procsystems.rs`, which reads `resident`).
        resident.0.push((n, fade));
        if rec.source != "procedural"
            && let Some(far) = far.as_mut()
            && let Some(slot) = far.resident.iter_mut().find(|s| s.x < 0.0)
        {
            *slot = Vec4::new(n as f32, fade, 0.0, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_ses_uniform_block() {
        let set = CustomModelSet::new(vec![(
            "a.cfg".into(),
            "CustomModel \"x\" { UseForType \"Nebula/SNR\" BaseShape \"sphere\" BaseShapeDims (10 10 10) Internal_scale 10 \
             Raymarch_step_count 86 NebulaFormSph true NebulaFormSphSign (0.5 -1) NebulaFormSphRad 7 SpirNoiseNudge (6.7 10) \
             Emission_gas_2 true Nebula_emission_color_2 (1 0.5 0) Emission_gas_2_coef_3 2 NebulaMapTwist true }"
                .into(),
        )]);
        let (ints, f) = pack(&set.models[0]);
        // NebulaFormSph 0x2000, Emission_gas_2 0x20, NebulaMapTwist 0x400000.
        assert_eq!(ints, UVec4::new(0x2000 | 0x20 | 0x40_0000, 86, 0, 0));
        assert_eq!(f[25].zw(), Vec2::new(0.5, -1.0));
        assert_eq!(f[34].z, 7.0);
        assert_eq!(f[19].zw(), Vec2::new(6.7, 10.0));
        assert_eq!(f[8].xyz(), Vec3::new(1.0, 0.5, 0.0));
        assert_eq!(f[32].w, 2.0);
        assert_eq!(f[1].w, 10.0);
    }

    #[test]
    fn model_consts_match_the_model() {
        let set = CustomModelSet::new(vec![(
            "a.cfg".into(),
            "CustomModel \"x\" { UseForType \"Nebula/SNR\" BaseShape \"sphere\" BaseShapeDims (10 12 14) Internal_scale 10 \
             Raymarch_step_count 86 NebulaFormSph true Randomize (0.25 -3 7.5) Scale (1 2 3) }"
                .into(),
        )]);
        let m = &set.models[0];
        let c = ModelConsts::of(m);
        assert_eq!(c.bound.to_bits(), m.bound_radius().to_bits());
        let (ints, f) = pack(m);
        assert_eq!(c.ints, ints);
        assert_eq!(c.f.map(|v| v.to_array().map(f32::to_bits)), f.map(|v| v.to_array().map(f32::to_bits)));
        let r = m.v3("Randomize");
        assert_eq!(c.randomize, Vec3::new(r[0] as f32, r[1] as f32, r[2] as f32));
        assert_eq!(c.randomize, Vec3::new(0.25, -3.0, 7.5));
    }

    #[test]
    fn nebulae_fade_in_from_four_pixels() {
        assert_eq!(draw_fade(4.0, 1.0), 0.0);
        assert!((draw_fade(5.5, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(draw_fade(30.0, 0.25), 0.25);
    }
}
