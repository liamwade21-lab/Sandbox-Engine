//! Drawing the skater and his board: Skate 3's default skater (`skater.s3m`), skinned the Viltrumite hero's way (the meshes at
//! the identity with his joint and weight attributes, a palette of joint matrices written in place each frame taking a
//! bind-pose vertex straight to the camera-relative render frame: `skate_skater.wesl`), lit as the sandbox's props are (its
//! light records at the player's place: `sandbox/light.rs`). Made on the mod's first enable, hidden while it's off; no material
//! changes after it is made.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{DQuat, Mat4, Quat, Vec3, Vec4};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::*;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use se_core::skate3::{MAX_BONES, Model, Sample};

use crate::camera::FlyCamera;
use crate::sandbox::light::PropLight;
use crate::sandbox::Sandbox;
use crate::system::SystemBody;
use crate::viltrumite::draw::{ATTR_HERO_JOINTS, ATTR_HERO_WEIGHTS};

use super::SkateAssets;

/// The skater's material: the props' (`PropMaterial`) with the joint palette.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(SkaterKey)]
pub struct SkaterMaterial {
    #[storage(0, read_only)]
    pub frame: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub diffuse: Handle<Image>,
    /// rgb over the texture's colour; a the alpha under which pixels are cut out (0: none).
    #[uniform(4)]
    pub tint: Vec4,
    /// x light of its own, y 1 when seen from both sides, z the least that daylight counts as.
    #[uniform(5)]
    pub glow: Vec4,
    #[storage(6, read_only)]
    pub palette: Handle<ShaderBuffer>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkaterKey {
    two_sided: bool,
}

impl From<&SkaterMaterial> for SkaterKey {
    fn from(m: &SkaterMaterial) -> Self {
        SkaterKey { two_sided: m.glow.y > 0.5 }
    }
}

impl Material for SkaterMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/skate_skater.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/skate_skater.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.tint.w > 0.0 { AlphaMode::Mask(self.tint.w) } else { AlphaMode::Opaque }
    }

    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            ATTR_HERO_JOINTS.at_shader_location(3),
            ATTR_HERO_WEIGHTS.at_shader_location(4),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        if key.bind_group_data.two_sided {
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}

/// His meshes and palette, once made.
#[derive(Resource)]
pub struct SkaterDraw {
    parts: Vec<Entity>,
    palette: Handle<ShaderBuffer>,
    bytes: Vec<u8>,
    model_m: Vec<Mat4>,
    pub shown: bool,
}

#[derive(Component)]
pub struct SkaterPart;

pub fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<SkaterMaterial>::default()).add_systems(Update, spawn).add_systems(
        PostUpdate,
        place.after(crate::sandbox::sandbox_step)
            .after(crate::universe::UniverseSet::Camera)
            .after(crate::terrain::update_node_transforms)
            .before(crate::universe::UniverseSet::FloatingOrigin)
            .before(bevy::camera::CameraUpdateSystems),
    );
}

fn image(w: u32, h: u32, rgba: &[u8]) -> Image {
    let mut img = Image::new(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, rgba.to_vec(), TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    img
}

/// His meshes, made once the mod's files are read and the sandbox's light exists. A palette of zeros puts every vertex at the
/// camera: nothing shows until the first real one is written.
#[allow(clippy::too_many_arguments)]
fn spawn(
    mut commands: Commands,
    assets: Option<Res<SkateAssets>>,
    light: Option<Res<PropLight>>,
    done: Option<Res<SkaterDraw>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SkaterMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let (Some(assets), Some(light), None) = (assets, light, done) else { return };
    let m: &Model = &assets.model;
    let palette = buffers.add(ShaderBuffer::new(vec![0u8; MAX_BONES * 64], RenderAssetUsages::default()));
    let white = images.add(image(1, 1, &[255, 255, 255, 255]));
    let textures: Vec<(String, Handle<Image>)> = assets.textures.iter().map(|(n, w, h, px)| (n.clone(), images.add(image(*w, *h, px)))).collect();
    let mut parts = Vec::new();
    for d in &m.draws {
        // Each draw as its own mesh of only its vertices.
        let tris = &m.indices[d.first_index as usize..(d.first_index + d.index_count) as usize];
        let mut remap = std::collections::HashMap::new();
        let mut verts = Vec::new();
        let idx: Vec<u32> = tris
            .iter()
            .map(|&i| {
                *remap.entry(i).or_insert_with(|| {
                    verts.push(m.vertices[i as usize]);
                    verts.len() as u32 - 1
                })
            })
            .collect();
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts.iter().map(|v| v.position).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, verts.iter().map(|v| v.normal).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, verts.iter().map(|v| v.uv).collect::<Vec<_>>());
        mesh.insert_attribute(ATTR_HERO_JOINTS, VertexAttributeValues::Uint8x4(verts.iter().map(|v| v.joints).collect()));
        mesh.insert_attribute(ATTR_HERO_WEIGHTS, VertexAttributeValues::Unorm8x4(verts.iter().map(|v| v.weights).collect()));
        mesh.insert_indices(Indices::U32(idx));
        let diffuse = textures.iter().find(|(n, _)| *n == d.texture).map(|(_, h)| h.clone()).unwrap_or_else(|| white.clone());
        let material = materials.add(SkaterMaterial {
            frame: light.frame.clone(),
            light: light.light.clone(),
            diffuse,
            tint: Vec4::new(d.tint[0], d.tint[1], d.tint[2], d.alpha_cut),
            glow: Vec4::new(0.0, if d.two_sided { 1.0 } else { 0.0 }, 0.0, 0.0),
            palette: palette.clone(),
        });
        parts.push(
            commands
                .spawn((
                    Name::new("Skate 3 skater"),
                    SkaterPart,
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    NoFrustumCulling,
                    RenderLayers::layer(crate::ship::warp::SHIP_LAYER),
                ))
                .id(),
        );
    }
    info!("skate: the skater's {} meshes are made ({} vertices, {} joints)", parts.len(), m.vertices.len(), m.joints.len());
    commands.insert_resource(SkaterDraw { parts, palette, bytes: vec![0u8; MAX_BONES * 64], model_m: vec![Mat4::IDENTITY; m.bones.len()], shown: false });
}

/// A pose's model matrices: the Skate 3 engine's hierarchy (bones under `TRAJECTORY`, bone 0, are in model space).
pub fn model_matrices(pose: &[Sample], model: &Model, out: &mut [Mat4]) {
    for (i, b) in model.bones.iter().enumerate() {
        let s = pose[i];
        let local = Mat4::from_scale_rotation_translation(Vec3::new(s[0], s[1], s[2]), Quat::from_xyzw(s[3], s[4], s[5], s[6]).normalize(), Vec3::new(s[7], s[8], s[9]));
        out[i] = if i == 0 {
            Mat4::IDENTITY
        } else if b.parent <= 0 {
            local
        } else {
            out[b.parent as usize] * local
        };
    }
}

/// This frame's pose into the palette, where the board is drawn; shown while he skates.
#[allow(clippy::too_many_arguments)]
fn place(
    mut sb: ResMut<Sandbox>,
    assets: Option<Res<SkateAssets>>,
    draw: Option<ResMut<SkaterDraw>>,
    cam: Res<FlyCamera>,
    bodies: Query<&SystemBody>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut parts: Query<&mut Visibility, With<SkaterPart>>,
) {
    let (Some(assets), Some(mut draw)) = (assets, draw) else { return };
    let k = sb.tick_fraction();
    let sb = &mut *sb;
    let frame = sb.world.as_ref().map(|w| w.frame);
    let show = match (sb.skate.as_mut(), frame) {
        (Some(sk), Some(frame)) => match bodies.iter().find(|b| b.obj == frame.obj) {
            Some(b) => {
                let (pos, rot) = sk.drawn(k);
                let origin = crate::sandbox::props::render_point(b, &frame, &cam, pos);
                let to_world: DQuat = b.orient * frame.rotation();
                let place = Mat4::from_rotation_translation(to_world.as_quat() * rot, origin);
                let carve = Some((sk.board.tilt.signum() * (sk.board.tilt.abs() / 0.35).min(1.0), sk.ctl.carve_share()));
                let pose = sk.ctl.anim.pose(carve);
                let m = &assets.model;
                let mut mm = std::mem::take(&mut draw.model_m);
                model_matrices(pose, m, &mut mm);
                for (j, joint) in m.joints.iter().enumerate().take(MAX_BONES) {
                    let skin = place * mm[joint.bone as usize] * Mat4::from_cols_array(&joint.bind);
                    draw.bytes[j * 64..(j + 1) * 64].copy_from_slice(bytemuck::cast_slice(&skin.to_cols_array()));
                }
                draw.model_m = mm;
                if let Some(mut b) = buffers.get_mut(&draw.palette) {
                    if let Some(d) = b.cast_slice_mut::<u8>() {
                        let n = d.len().min(draw.bytes.len());
                        d[..n].copy_from_slice(&draw.bytes[..n]);
                    }
                }
                true
            }
            None => false,
        },
        _ => false,
    };
    if draw.shown != show {
        draw.shown = show;
        for e in draw.parts.clone() {
            if let Ok(mut v) = parts.get_mut(e) {
                *v = if show { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }
}
