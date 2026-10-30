//! Ray-marched nebulae: the `CustomModel`s of `models/custom/*.cfg` (`se_core::custom_model`,
//! docs/formats/nebula_raymarch.md). SE draws every catalog nebula without a named sprite model this way.
//!
//! SE's shader (`nebula_raymarch.glsl`) is in the encrypted `Shaders.pak`; `galaxy_nebula_raymarch.wesl`
//! is the port's own ray marcher driven by the same parameters: two density fields, dust and emitting
//! gas (the `(dust gas)` pairs of the script), each the near-zero level of a signed sum of shapes
//! (`NebulaForm*`) and spiral noises; the dust scatters the light of a star and the ambient light with a
//! two-lobe Henyey-Greenstein phase function and absorbs, the gas glows in up to three colours whose weight
//! falls off from a centre (`Emission_gas_N_*`).
//!
//! Like the elliptical galaxies (`ellipticals.rs`), a sphere around the model is drawn at infinity and the
//! fragment shader marches along the view ray, so the volume works from outside and from inside.

use std::sync::Arc;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use se_core::custom_model::{BaseShape, CustomModel, CustomModelSet};

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::galaxies::{GalaxyFarMaterial, angular_px, model_fade};
use crate::graphics::GraphicsSettings;
use crate::nebulae::{NebulaSettings, NebulaState, ResidentNebulae, update_nebulae};

/// Volumes drawn at once (the largest on screen).
const MAX_VOLUMES: usize = 4;
/// Uniform slots (vec4) per model; must match `galaxy_nebula_raymarch.wesl`.
pub const SLOTS: usize = 52;
/// Display brightness per unit of `Bright` (SE's `hdrNebulaRaymarchBright` is 0.03 in its own units; this is
/// matched by eye to SE's Spirograph Nebula at 3 radii).
const GAIN: f32 = 0.02;

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
    pub p: [Vec4; SLOTS],
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct RaymarchMaterial {
    #[uniform(0)]
    pub u: RaymarchParams,
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
        // With the galaxy and nebula model emission (`GalaxyModelMaterial`).
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
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// Factor applied to internal positions before the forms are evaluated. `NebScale` reads as a divisor
/// for the Spirograph Nebula's model (its shell then has SE's size) but must multiply for models whose
/// tori and capsules would otherwise lie outside their bounding shape (the Omega Nebula's `Diff 1
/// JackDole`); SE's shader is encrypted, so the port divides unless that puts the largest enabled form
/// outside the bounding shape.
fn form_scale(m: &CustomModel) -> f64 {
    let ns = m.num("NebScale");
    if ns <= 0.0 {
        return 1.0;
    }
    let mut size: f64 = 0.0;
    if m.flag("NebulaFormSph") {
        size = size.max(m.num("NebulaFormSphRad"));
    }
    if m.flag("NebulaFormTorus") {
        size = size.max(0.5 * (m.num("NebulaFormTorusRadius_1") + m.num("NebulaFormTorusRadius_2")));
    }
    if m.flag("NebulaFormCapsule") {
        size = size.max(m.num("NebulaFormCapsuleRadius"));
    }
    if m.flag("NebulaFormCyl") {
        size = size.max(m.pair("NebulaFormCylParam")[0]);
    }
    if m.flag("NebulaFormDsk") {
        size = size.max(m.v3("NebulaFormDskParam")[0]);
    }
    let bound = m.dims.iter().fold(0.0f64, |a, &d| a.max(d.abs()));
    if size * ns <= bound { 1.0 / ns } else { ns }
}

/// Packs a model into the uniform slots (layout in `galaxy_nebula_raymarch.wesl`).
/// `gain`: brightness; `fade`: the model's fade-in (scales the dust's opacity).
pub fn pack(m: &CustomModel, steps: f32, gain: f32, fade: f32) -> [Vec4; SLOTS] {
    let f = |v: f64| v as f32;
    let v3 = |k: &str| {
        let a = m.v3(k);
        Vec3::new(f(a[0]), f(a[1]), f(a[2]))
    };
    let pair = |k: &str| {
        let a = m.pair(k);
        Vec2::new(f(a[0]), f(a[1]))
    };
    let on = |k: &str| if m.flag(k) { 1.0 } else { 0.0 };
    let form = |k: &str, extra: f32| {
        let s = pair(&format!("{k}Sign"));
        Vec4::new(s.x, s.y, on(k), extra)
    };
    let mut p = [Vec4::ZERO; SLOTS];
    let shape = match m.shape {
        BaseShape::Box => 0.0,
        BaseShape::Sphere => 1.0,
        BaseShape::Cylinder => 2.0,
    };
    p[0] = Vec4::new(shape, f(m.dims[0]), f(m.dims[1]), f(m.dims[2]));
    p[1] = Vec3::new(f(m.scale[0]), f(m.scale[1]), f(m.scale[2])).extend(f(m.internal_scale()));
    p[2] = Vec4::new(steps, f(m.num("td_break_value")), f(m.bound_radius()), gain);
    p[3] = v3("sum_transparency_break").extend(fade);
    p[4] = Vec4::new(on("Dust_enabled"), f(m.num("Dust_volume_height")), f(m.num("dust_absorption_factor")), f(m.num("dust_scattering_factor")));
    p[5] = Vec4::new(f(m.num("Dust_volume_density_factor")), f(m.num("Albedo")), f(m.num("Forward_scattering_g")), f(m.num("Backward_scattering_g")));
    p[6] = Vec4::new(f(m.num("HG_mix_coef")), on("Gas_emission_enabled"), f(m.num("Gas_volume_height")), f(m.num("gas_scattering_factor")));
    p[7] = Vec4::new(f(m.num("gas_density_factor")), f(m.num("Emission_intensivity_coef")), f(form_scale(m)), on("NebulaMapTwist"));
    p[8] = (v3("Light_Color_11") * f(m.num("Light_intensivity"))).extend(f(m.num("Light_radius_single_star")));
    p[9] = (v3("Ambient_Light_Color_11") * f(m.num("Ambient_Light_intensivity"))).extend(on("Single_star"));
    p[10] = v3("Star_position").extend(f(m.num("star_empty_sphere_radius")));
    p[11] = Vec4::new(on("Render_star"), f(m.num("Star_light_coef_1")), f(m.num("Star_light_coef_2")), on("Render_star_bloom"));
    p[12] = Vec4::new(f(m.num("Bloom_light_coef_1")), f(m.num("Bloom_light_coef_2")), f(m.num("NebulaMapTwistCoef1")), f(m.num("NebulaMapTwistCoef2")));
    let mut c3 = Vec4::ZERO;
    for i in 0..3 {
        let g = format!("Emission_gas_{}", i + 1);
        p[13 + i * 3] = v3(&format!("Nebula_emission_color_{}", i + 1)).extend(on(&g));
        p[14 + i * 3] = v3(&format!("{g}_direction")).extend(f(m.num(&format!("{g}_coef_1"))));
        p[15 + i * 3] = v3(&format!("{g}_coef_0")).extend(f(m.num(&format!("{g}_coef_2"))));
        c3[i] = f(m.num(&format!("{g}_coef_3")));
    }
    p[22] = c3;
    p[23] = pair("NebScaleCoef").extend(0.0).extend(0.0) + Vec4::new(0.0, 0.0, pair("NebulaMapThickCoef").x, pair("NebulaMapThickCoef").y);
    p[24] = pair("SpirNoiseNudge").extend(0.0).extend(0.0) + Vec4::new(0.0, 0.0, pair("SpirNoiseAmount").x, pair("SpirNoiseAmount").y);
    p[25] = pair("SpirNoiseIter").extend(0.0).extend(0.0) + Vec4::new(0.0, 0.0, pair("SpirNoiseCoef").x, pair("SpirNoiseCoef").y);
    p[26] = pair("Nudge_3D_2").extend(0.0).extend(0.0) + Vec4::new(0.0, 0.0, pair("Noise_amount_3D_2").x, pair("Noise_amount_3D_2").y);
    p[27] = pair("Frequency_iteration_count_3D_2").extend(0.0).extend(0.0)
        + Vec4::new(0.0, 0.0, pair("SpirNoise3DCoef_2").x, pair("SpirNoise3DCoef_2").y);
    p[28] = v3("NebulaCentPos").extend(0.0);
    p[29] = form("NebulaFormPlane", f(m.num("NebulaFormPlaneHeight")));
    p[30] = v3("NebulaFormPlaneXYZ").extend(0.0);
    p[31] = form("NebulaFormDsk", 0.0);
    p[32] = v3("NebulaFormDskParam").extend(0.0);
    p[33] = form("NebulaFormCyl", 0.0);
    p[34] = pair("NebulaFormCylParam").extend(0.0).extend(0.0);
    p[35] = form("NebulaFormSph", f(m.num("NebulaFormSphRad")));
    p[36] = form("NebulaFormCapsule", f(m.num("NebulaFormCapsuleRadius")));
    p[37] = v3("NebulaFormCapsulePos_1").extend(0.0);
    p[38] = v3("NebulaFormCapsulePos_2").extend(0.0);
    p[39] = form("NebulaFormTorus", 0.0);
    p[40] = Vec4::new(f(m.num("NebulaFormTorusRadius_1")), f(m.num("NebulaFormTorusRadius_2")), 0.0, 0.0);
    p[41] = form("NebulaFormSpir", 0.0);
    let s = m.v4("NebulaFormSpirParam");
    p[42] = Vec4::new(f(s[0]), f(s[1]), f(s[2]), f(s[3]));
    p[43] = form("NebulaFormNoise", 0.0);
    p[44] = Vec4::new(f(m.num("NebulaFormNoiseCoef1")), f(m.num("NebulaFormNoiseCoef2")), 0.0, 0.0);
    p[45] = form("NebulaFormFBMNoise", f(m.num("NebulaFormFBMNoiseCoef")));
    p[46] = form("NebulaFormSpirNoise", 0.0);
    p[47] = Vec4::new(f(m.num("NebulaFormSpirNoiseCoef1")), f(m.num("NebulaFormSpirNoiseCoef2")), f(m.num("NebulaFormSpirNoiseCoef3")), 0.0);
    p[48] = form("NebulaFormSpirIQNoise", 0.0);
    p[49] = Vec4::new(
        f(m.num("NebulaFormSpirIQNoiseCoef1")),
        f(m.num("NebulaFormSpirIQNoiseCoef2")),
        f(m.num("NebulaFormSpirIQNoiseCoef3")),
        f(m.num("NebulaFormSpirIQNoiseCoef4")),
    );
    p[50] = form("NebulaFormSpirNoise3D", f(m.num("NebulaFormSpirNoise3DCoef")));
    p
}

#[derive(Resource, Default)]
struct Volumes {
    mesh: Handle<Mesh>,
    slots: Vec<(Entity, Handle<RaymarchMaterial>)>,
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
fn update_volumes(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    assign: Res<RaymarchAssign>,
    settings: Res<NebulaSettings>,
    gfx: Res<GraphicsSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    st: Option<Res<NebulaState>>,
    mut resident: ResMut<ResidentNebulae>,
    mut vols: ResMut<Volumes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<RaymarchMaterial>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(st) = st else { return };
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();

    // The ray-marched nebulae big enough on screen, largest first.
    let mut wanted: Vec<(f64, usize, usize, f32)> = Vec::new();
    if settings.enabled {
        for (n, m) in assign.model_of.iter().enumerate() {
            let (Some(m), Some(rec)) = (m, cat.nebulae.get(n)) else { continue };
            let r = rec.radius_pc * assign.set.models[*m].bound_radius();
            let px = angular_px(r, (rec.pos_pc - cam_pc).length(), focal);
            let fade = model_fade(px, settings.build_px as f64);
            if fade > 0.0 {
                wanted.push((px, n, *m, fade));
            }
        }
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        wanted.truncate(MAX_VOLUMES);
    }
    debug!("ray-marched volumes: {:?}", wanted.iter().map(|w| (cat.nebulae[w.1].names[0].as_str(), assign.set.models[w.2].name.as_str(), w.0 as i64)).collect::<Vec<_>>());

    if vols.mesh == Handle::default() {
        vols.mesh = meshes.add(Sphere::new(1.03).mesh().ico(3).expect("icosphere"));
    }
    while vols.slots.len() < wanted.len() {
        let mat = mats.add(RaymarchMaterial {
            u: RaymarchParams { center_rel: Vec4::ZERO, rot0: Vec4::X, rot1: Vec4::Y, rot2: Vec4::Z, p: [Vec4::ZERO; SLOTS] },
        });
        let e = commands
            .spawn((Mesh3d(vols.mesh.clone()), MeshMaterial3d(mat.clone()), Transform::IDENTITY, Visibility::Hidden, NoFrustumCulling))
            .id();
        vols.slots.push((e, mat));
    }

    let mut far = far_mats.get_mut(&st.far_handle());
    let quality = gfx.quality.volume_resolution.clamp(0.1, 1.0);
    for (i, (e, mat)) in vols.slots.iter().enumerate() {
        let Some(&(_, n, m, fade)) = wanted.get(i) else {
            if let Ok(mut v) = vis.get_mut(*e) {
                *v = Visibility::Hidden;
            }
            continue;
        };
        let rec = &cat.nebulae[n];
        let model = &assign.set.models[m];
        // SE marches `Raymarch_step_count` steps at its volumetric resolution; the port renders at full
        // resolution and spends the quality on steps instead.
        let steps = (model.num("Raymarch_step_count") as f32).max(32.0) * (0.5 + quality);
        let gain = model.bright as f32 * GAIN * fade;
        let inv = Mat3::from_quat(rec.orientation.inverse().as_quat());
        if let Some(mut mm) = mats.get_mut(mat) {
            mm.u = RaymarchParams {
                center_rel: (rec.pos_pc - cam_pc).as_vec3().extend(rec.radius_pc as f32),
                rot0: inv.x_axis.extend(0.0),
                rot1: inv.y_axis.extend(0.0),
                rot2: inv.z_axis.extend(0.0),
                p: pack(model, steps, gain, fade),
            };
        }
        if let Ok(mut v) = vis.get_mut(*e) {
            *v = if fade > 0.001 { Visibility::Visible } else { Visibility::Hidden };
        }
        // The far sprite hides behind the volume as it fades in.
        resident.0.push((n, fade));
        if let Some(far) = far.as_mut()
            && let Some(slot) = far.uniforms.resident.iter_mut().find(|s| s.x < 0.0)
        {
            *slot = Vec4::new(n as f32, fade, 0.0, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_pairs_and_forms() {
        let set = CustomModelSet::new(vec![(
            "a.cfg".into(),
            "CustomModel \"x\" { UseForType \"Nebula/SNR\" BaseShape \"sphere\" BaseShapeDims (10 10 10) Internal_scale 10 \
             NebulaFormSph true NebulaFormSphSign (0.5 -1) NebulaFormSphRad 7 SpirNoiseNudge (6.7 10) \
             Emission_gas_2 true Nebula_emission_color_2 (1 0.5 0) Emission_gas_2_coef_3 2 }"
                .into(),
        )]);
        let p = pack(&set.models[0], 100.0, 1.0, 1.0);
        assert_eq!(p[0], Vec4::new(1.0, 10.0, 10.0, 10.0));
        assert_eq!(p[35], Vec4::new(0.5, -1.0, 1.0, 7.0));
        assert_eq!(p[24], Vec4::new(6.7, 10.0, 0.0, 0.0));
        assert_eq!(p[16], Vec4::new(1.0, 0.5, 0.0, 1.0));
        assert_eq!(p[22].y, 2.0);
        assert_eq!(p[2].z, 1.0);
    }
}
