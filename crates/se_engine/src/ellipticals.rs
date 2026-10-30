//! Elliptical galaxies up close. SE has no sprite model for types E0..E7: `galaxy_bulge.glsl` draws them
//! from a pre-integrated density table (`docs/formats/galaxy_model.md`, section 8). Here a sphere around
//! the galaxy, projected to infinity like the other galaxy layers, ray-marches a deprojected de Vaucouleurs
//! ellipsoid (the far sprite's profile, flattened to the Hubble type's axis ratio), so the glow stays when
//! the camera approaches or enters the galaxy. Like a sprite model it fades in as the far sprite fades out
//! (the `resident` list) and carries the same total flux.

use std::f64::consts::PI;
use std::sync::OnceLock;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use se_core::star_octree::GalaxyForm;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::galaxies::{
    FOCAL_REF, GalaxyFarMaterial, GalaxySettings, GalaxyState, VISION_CAP_MODEL, angular_px, model_fade,
    profile_type, type_color, update_galaxies, vision,
};

/// Volumes drawn at once (the nearest ellipticals).
const MAX_VOLUMES: usize = 4;
/// The bounding sphere, in galaxy radii (the faceted mesh must still enclose the ellipsoid).
const BOUND: f32 = 1.03;

/// Density profile, matching `galaxy_far.wesl`'s elliptical sprite: de Vaucouleurs with the effective
/// radius at a quarter of the radius, deprojected after Prugniel & Simien (1997), softened core, the
/// sprite's `(1 - x^2)^2` window. Must match `density` in `galaxy_elliptical.wesl`.
fn density(m: f64) -> f64 {
    if m >= 1.0 {
        return 0.0;
    }
    let r = (m * m + 1.0e-4).sqrt() / 0.25;
    let w = 1.0 - m * m;
    r.powf(-0.851) * (-7.67 * (r.powf(0.25) - 1.0)).exp() * w * w
}

/// `4 pi * int m^2 rho(m) dm` over the unit sphere (log-spaced: the profile is cuspy).
fn unit_mass() -> f64 {
    static M: OnceLock<f64> = OnceLock::new();
    *M.get_or_init(|| {
        let (a, b, n) = (1.0e-6f64.ln(), 0.0f64, 20_000);
        let h = (b - a) / n as f64;
        let s: f64 = (0..n)
            .map(|i| {
                let m = (a + (i as f64 + 0.5) * h).exp();
                density(m) * m * m * m
            })
            .sum();
        4.0 * PI * s * h
    })
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct VolumeParams {
    /// xyz: galaxy centre relative to the camera (pc), w: radius (pc)
    pub center_rel: Vec4,
    /// Columns of the universe -> galaxy rotation.
    pub rot0: Vec4,
    pub rot1: Vec4,
    pub rot2: Vec4,
    /// rgb: colour times brightness per unit column density, w: axis ratio
    pub color: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct EllipticalMaterial {
    #[uniform(0)]
    pub u: VolumeParams,
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
        // Back faces only: one fragment per pixel from outside, and all around the camera inside.
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

#[derive(Resource, Default)]
struct Volumes {
    mesh: Handle<Mesh>,
    slots: Vec<(Entity, Handle<EllipticalMaterial>)>,
}

pub struct EllipticalsPlugin;

impl Plugin for EllipticalsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<EllipticalMaterial>::default())
            .init_resource::<Volumes>()
            .add_systems(PostUpdate, update_volumes.after(update_galaxies));
    }
}

#[allow(clippy::too_many_arguments)]
fn update_volumes(
    mut commands: Commands,
    cam: Res<FlyCamera>,
    data: Res<SeData>,
    settings: Res<GalaxySettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    gs: Option<ResMut<GalaxyState>>,
    mut vols: ResMut<Volumes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<EllipticalMaterial>>,
    mut far_mats: ResMut<Assets<GalaxyFarMaterial>>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(mut gs) = gs else { return };
    let cat = &data.catalog;
    let cam_pc = cam.pos.to_parsecs();
    let height = windows.iter().next().map(|w| w.resolution.physical_height()).unwrap_or(900) as f64;
    let focal = height * 0.5 / (cam.fov_deg as f64 * 0.5).to_radians().tan();

    // The ellipticals big enough on screen to be resolved, largest first.
    let mut wanted: Vec<(f64, usize, f32)> = Vec::new();
    if settings.enabled {
        for (g, rec) in cat.galaxies.iter().enumerate() {
            if profile_type(rec) != 0 || gs.has_model(g) {
                continue;
            }
            let px = angular_px(rec.radius_pc, (rec.pos_pc - cam_pc).length(), focal);
            let fade = model_fade(px, settings.build_px as f64);
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
            u: VolumeParams {
                center_rel: Vec4::ZERO,
                rot0: Vec4::X,
                rot1: Vec4::Y,
                rot2: Vec4::Z,
                color: Vec4::new(0.0, 0.0, 0.0, 1.0),
            },
        });
        let e = commands
            .spawn((Mesh3d(vols.mesh.clone()), MeshMaterial3d(mat.clone()), Transform::IDENTITY, Visibility::Hidden, NoFrustumCulling))
            .id();
        vols.slots.push((e, mat));
    }

    let mut far = far_mats.get_mut(&gs.far_handle());
    for (i, (e, mat)) in vols.slots.iter().enumerate() {
        let Some(&(px, g, fade)) = wanted.get(i) else {
            if let Ok(mut v) = vis.get_mut(*e) {
                *v = Visibility::Hidden;
            }
            continue;
        };
        let rec = &cat.galaxies[g];
        let r = rec.radius_pc;
        let q = match GalaxyForm::from_type(&rec.kind) {
            GalaxyForm::Elliptical { q } => q,
            _ => 1.0,
        };
        // Total flux as the far sprite's (`galaxy_far.wesl`): the pixel sum of a galaxy at distance d is
        // `far_brightness * 10^(0.4 (mag_ref - M)) * (10 pc / d)^2 * (focal / FOCAL_REF)^2`. A pixel shows
        // `k * int rho ds` (s in radii), and the sum of that over the screen is `k * mass * r * focal^2 / d^2`
        // with mass in radii^3: hence k below. Intergalactic vision and the daylight fade as for sprite models.
        let flux = 10f64.powf(0.4 * (settings.mag_ref as f64 - rec.vis_mag)) * 100.0;
        let mass = unit_mass() * q;
        let mean_sb = settings.far_brightness as f64 * flux / (PI * r * r * FOCAL_REF * FOCAL_REF);
        let boost = vision(mean_sb as f32, gs.gain, px as f32, VISION_CAP_MODEL) as f64 / mean_sb.max(1e-30);
        let k = settings.far_brightness as f64 * flux / (FOCAL_REF * FOCAL_REF * mass * r * r)
            * boost
            * settings.model_brightness as f64
            * fade as f64;
        let col = type_color(&rec.kind);
        let inv = Mat3::from_quat(rec.orientation.inverse().as_quat());
        if let Some(mut m) = mats.get_mut(mat) {
            m.u = VolumeParams {
                center_rel: (rec.pos_pc - cam_pc).as_vec3().extend(r as f32),
                rot0: inv.x_axis.extend(0.0),
                rot1: inv.y_axis.extend(0.0),
                rot2: inv.z_axis.extend(0.0),
                color: Vec4::new(col[0] * k as f32, col[1] * k as f32, col[2] * k as f32, q as f32),
            };
        }
        if let Ok(mut v) = vis.get_mut(*e) {
            *v = if fade > 0.001 { Visibility::Visible } else { Visibility::Hidden };
        }
        // The far sprite hides behind the volume as it fades in.
        gs.resident.push((g, fade));
        if let Some(far) = far.as_mut() {
            if let Some(slot) = far.uniforms.resident.iter_mut().find(|s| s.x < 0.0) {
                *slot = Vec4::new(g as f32, fade, 0.0, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_mass_is_finite_and_centrally_concentrated() {
        let m = unit_mass();
        assert!(m.is_finite() && m > 0.0, "{m}");
        // Half of the light within about the effective radius (0.25 of the radius, before the window).
        let n = 20_000;
        let (a, b) = (1.0e-6f64.ln(), 0.25f64.ln());
        let h = (b - a) / n as f64;
        let inner: f64 = (0..n).map(|i| (a + (i as f64 + 0.5) * h).exp()).map(|m| density(m) * m * m * m).sum::<f64>()
            * 4.0
            * PI
            * h;
        let f = inner / m;
        assert!(f > 0.4 && f < 0.8, "{f}");
    }
}
