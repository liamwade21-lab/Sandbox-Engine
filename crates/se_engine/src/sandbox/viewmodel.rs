//! The weapon in his hands as Half-Life 2 draws it: its first-person model (`models/weapons/v_*.mdl`, hands and all) with
//! its own animations, read from the Garry's Mod install (`se_core::gmod` `GmodInstall::skeleton`).
//!
//! - **Skinning** is the Viltrumite hero's way: the meshes sit at the identity with his joint and weight attributes, and a
//!   palette of bone matrices (one storage buffer a weapon, written in place each frame) takes a bind-pose vertex straight
//!   to the camera-relative render frame (`sandbox_viewmodel.wesl`). No material changes after it is made.
//! - **Where it is held** is Source's `CBaseViewModel::CalcViewModelView`: at the eyes, along the view's angles WITHOUT the
//!   punch (so a shot's kick lifts the view off the gun), with Half-Life 2's bob (`CBaseHLCombatWeapon::AddViewmodelBob`:
//!   `HL2_BOB_CYCLE_MAX` 0.45 s, by his speed up to 320 units/s) and the lag behind the view's turn (`CalcViewModelLag`:
//!   5 units of the facing's change, caught up at 5 a second, and the pitch's 0.035 / 0.03 / 0.02 units a degree). It is
//!   drawn with `viewmodel_fov` 54 against the world's 75 (the shader's `glow.w`) and in the nearest slice of the depth
//!   range, so it never goes into a wall.
//! - **Animation**: a sequence plays once and hands over to the idle loop, as `WeaponIdle` does; a new one blends in over
//!   the old one's fade-out time unless its file says snap (`STUDIO_SNAP`). What the weapon does and when comes from the
//!   sequences' own lengths (`SequenceDuration`), as in the SDK's weapon code.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::math::{Mat3, Mat4, Quat, Vec3};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::*;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use se_core::gmod::{GmodInstall, GmodSequence, GmodSkeleton};

use crate::viltrumite::draw::{ATTR_HERO_JOINTS, ATTR_HERO_WEIGHTS};

use super::assets::{self, MatData};
use super::light::PropLight;
use super::movement::UNIT_M;

/// The most bones a view model may have (Half-Life 2's have 30 to 60).
pub const MAX_BONES: usize = 128;
/// `tan(75 / 2) / tan(54 / 2)`: the world's field of view over `viewmodel_fov`.
pub const FOV_RATIO: f32 = 1.505_96;

/// A view model's material: the props' (`PropMaterial`) with the bone palette.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(ViewKey)]
pub struct ViewMaterial {
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
    /// x light of its own, y 1 when seen from both sides, z the least that daylight counts as, w `FOV_RATIO`.
    #[uniform(5)]
    pub glow: Vec4,
    #[storage(6, read_only)]
    pub palette: Handle<ShaderBuffer>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewKey {
    two_sided: bool,
}

impl From<&ViewMaterial> for ViewKey {
    fn from(m: &ViewMaterial) -> Self {
        ViewKey { two_sided: m.glow.y > 0.5 }
    }
}

impl Material for ViewMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sandbox_viewmodel.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sandbox_viewmodel.wesl".into()
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

pub fn build(app: &mut App) {
    app.add_plugins(MaterialPlugin::<ViewMaterial>::default());
}

/// One mesh of a view model as read: Source units, the bind pose.
pub struct ViewMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u8; 4]>,
    pub weights: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
    pub material: MatData,
}

/// A view model as read from the game's files. Any thread.
pub struct ViewData {
    pub meshes: Vec<ViewMesh>,
    pub skeleton: GmodSkeleton,
}

/// Three weights as bytes that sum to 255 (the vertex format's unorm).
fn weight_bytes(w: [f32; 3]) -> [u8; 4] {
    let sum = (w[0] + w[1] + w[2]).max(1e-6);
    let mut b = [0u8; 4];
    let mut left = 255i32;
    for k in 0..3 {
        let v = ((w[k] / sum * 255.0).round() as i32).clamp(0, left);
        b[k] = v as u8;
        left -= v;
    }
    // (What rounding left over goes to the heaviest bone.)
    let most = (0..3).max_by(|a, c| w[*a].total_cmp(&w[*c])).unwrap_or(0);
    b[most] = (b[most] as i32 + left) as u8;
    b
}

pub fn load(install: &GmodInstall, path: &str) -> Result<ViewData, String> {
    let model = install.model(path).map_err(|e| e.to_string())?;
    let skeleton = install.skeleton(path).map_err(|e| e.to_string())?;
    let n = skeleton.bones.len();
    if n == 0 || n > MAX_BONES {
        return Err(format!("{path} has {n} bones"));
    }
    if skeleton.bones.iter().enumerate().any(|(i, b)| b.parent >= i as i32) {
        return Err(format!("{path}: a bone comes before its parent"));
    }
    if skeleton.sequences.iter().any(|s| s.frames.is_empty() || s.frames.iter().any(|f| f.len() != n)) {
        return Err(format!("{path}: a sequence's frames don't fit its bones"));
    }
    let mut meshes = Vec::new();
    for m in model.meshes {
        if m.indices.is_empty() || m.positions.is_empty() || m.bones.len() != m.positions.len() || m.weights.len() != m.positions.len() {
            continue;
        }
        if m.bones.iter().flatten().any(|b| *b as usize >= n) {
            return Err(format!("{path}: a vertex names a bone it hasn't got"));
        }
        let material = assets::material_data(install, &m.material);
        meshes.push(ViewMesh {
            joints: m.bones.iter().map(|b| [b[0], b[1], b[2], 0]).collect(),
            weights: m.weights.iter().map(|w| weight_bytes(*w)).collect(),
            positions: m.positions,
            normals: m.normals,
            uvs: m.uvs,
            indices: m.indices,
            material,
        });
    }
    if meshes.is_empty() {
        return Err(format!("{path} has no mesh"));
    }
    Ok(ViewData { meshes, skeleton })
}

/// One mesh of a view model in the scene.
#[derive(Component)]
pub struct ViewModelPart;

/// A sequence that is playing: which, and how far in (seconds).
#[derive(Clone, Copy, Debug)]
struct Playing {
    seq: usize,
    time: f32,
}

/// A weapon's view model in the scene, and what it is playing.
pub struct ViewModel {
    skeleton: GmodSkeleton,
    by_name: HashMap<String, usize>,
    pub parts: Vec<Entity>,
    palette: Handle<ShaderBuffer>,
    /// Bind-pose model space to each bone's space.
    pose_to_bone: Vec<Mat4>,
    /// The muzzle: its bone and its place in that bone's space.
    muzzle: Option<(usize, Mat4)>,
    now: Playing,
    /// The sequence it came from, still running, and how much of its fade is left and was given.
    before: Option<(Playing, f32, f32)>,
    /// What plays when a one-shot sequence ends.
    idle: usize,
    /// The sequence laid over whatever plays, on the bones it weighs, between its two ends by `open`: the gravity gun's
    /// prongs (`prong_blend`, by the pose parameter `active` in the game), shut at 0 and open at 1.
    overlay: Option<usize>,
    pub open: f32,
    /// Scratch: each bone's place in model space this frame.
    world: Vec<Mat4>,
    local: Vec<(Vec3, Quat)>,
    blend: Vec<(Vec3, Quat)>,
    bytes: Vec<u8>,
}

fn rows(m: &[[f32; 4]; 3]) -> Mat4 {
    Mat4::from_cols(Vec4::new(m[0][0], m[1][0], m[2][0], 0.0), Vec4::new(m[0][1], m[1][1], m[2][1], 0.0), Vec4::new(m[0][2], m[1][2], m[2][2], 0.0), Vec4::new(m[0][3], m[1][3], m[2][3], 1.0))
}

/// A sequence's pose `time` seconds in: between its two nearest frames. A loop's last frame is its first again.
fn sample(seq: &GmodSequence, time: f32, out: &mut [(Vec3, Quat)]) {
    let n = seq.frames.len();
    let last = (n - 1) as f32;
    let mut f = time.max(0.0) * seq.fps;
    if n < 2 {
        f = 0.0;
    } else if seq.looping {
        f = f.rem_euclid(last);
    } else {
        f = f.min(last);
    }
    let i0 = (f as usize).min(n - 1);
    let i1 = (i0 + 1).min(n - 1);
    let a = f - i0 as f32;
    for (b, o) in out.iter_mut().enumerate() {
        let (p0, q0) = seq.frames[i0][b];
        let (p1, q1) = seq.frames[i1][b];
        let (q0, mut q1) = (Quat::from_array(q0), Quat::from_array(q1));
        if q0.dot(q1) < 0.0 {
            q1 = -q1;
        }
        *o = (Vec3::from_array(p0).lerp(Vec3::from_array(p1), a), q0.lerp(q1, a).normalize());
    }
}

impl ViewModel {
    /// The view model's meshes, materials and palette, hidden until it is in hand.
    pub fn spawn(
        commands: &mut Commands,
        data: ViewData,
        light: &PropLight,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        materials: &mut Assets<ViewMaterial>,
        buffers: &mut Assets<ShaderBuffer>,
        white: Handle<Image>,
    ) -> ViewModel {
        let n = data.skeleton.bones.len();
        // (A palette of zeros puts every vertex at the camera: nothing is drawn until the first real one is written.)
        let palette = buffers.add(ShaderBuffer::new(vec![0u8; MAX_BONES * 64], RenderAssetUsages::default()));
        let mut textures: HashMap<String, Handle<Image>> = HashMap::new();
        let parts = data
            .meshes
            .iter()
            .map(|m| {
                let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
                let count = m.positions.len();
                let mut normals = m.normals.clone();
                normals.resize(count, [0.0, 0.0, 1.0]);
                let mut uvs = m.uvs.clone();
                uvs.resize(count, [0.0, 0.0]);
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
                mesh.insert_attribute(ATTR_HERO_JOINTS, VertexAttributeValues::Uint8x4(m.joints.clone()));
                mesh.insert_attribute(ATTR_HERO_WEIGHTS, VertexAttributeValues::Unorm8x4(m.weights.clone()));
                mesh.insert_indices(Indices::U32(m.indices.iter().copied().filter(|i| (*i as usize) < count).collect::<Vec<u32>>()));
                let diffuse = textures
                    .entry(m.material.key.clone())
                    .or_insert_with(|| match m.material.texture.as_ref().and_then(assets::bevy_image) {
                        Some(image) => images.add(image),
                        None => white.clone(),
                    })
                    .clone();
                let material = materials.add(ViewMaterial {
                    frame: light.frame.clone(),
                    light: light.light.clone(),
                    diffuse,
                    tint: Vec3::from_array(m.material.tint).extend(m.material.alpha_cut),
                    glow: Vec4::new(0.0, if m.material.two_sided { 1.0 } else { 0.0 }, 0.0, FOV_RATIO),
                    palette: palette.clone(),
                });
                commands
                    .spawn((
                        Name::new("Sandbox view model"),
                        ViewModelPart,
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material),
                        Transform::IDENTITY,
                        Visibility::Hidden,
                        NoFrustumCulling,
                        RenderLayers::layer(crate::ship::warp::SHIP_LAYER),
                    ))
                    .id()
            })
            .collect();
        let by_name: HashMap<String, usize> = data.skeleton.sequences.iter().enumerate().map(|(i, s)| (s.name.clone(), i)).collect();
        let idle = ["idle01", "idle"].iter().find_map(|n| by_name.get(*n).copied()).unwrap_or(0);
        let overlay = data.skeleton.sequences.iter().position(|s| s.autoplay && s.blend_to.len() == n && s.weights.len() == n);
        let muzzle = data.skeleton.attachments.iter().find(|a| a.name.eq_ignore_ascii_case("muzzle")).or(data.skeleton.attachments.first()).map(|a| (a.bone, rows(&a.local)));
        ViewModel {
            pose_to_bone: data.skeleton.bones.iter().map(|b| rows(&b.pose_to_bone)).collect(),
            skeleton: data.skeleton,
            by_name,
            parts,
            palette,
            muzzle,
            now: Playing { seq: idle, time: 0.0 },
            before: None,
            idle,
            overlay,
            open: 0.0,
            world: vec![Mat4::IDENTITY; n],
            local: vec![(Vec3::ZERO, Quat::IDENTITY); n],
            blend: vec![(Vec3::ZERO, Quat::IDENTITY); n],
            bytes: vec![0u8; MAX_BONES * 64],
        }
    }

    fn find(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.by_name.get(*n).copied())
    }

    /// How long the first of these sequences it has lasts (seconds).
    pub fn duration(&self, names: &[&str]) -> Option<f32> {
        let s = &self.skeleton.sequences[self.find(names)?];
        Some((s.frames.len().max(2) - 1) as f32 / s.fps.max(1.0))
    }

    /// Start the first of these sequences it has, from its beginning; its length in seconds.
    pub fn play(&mut self, names: &[&str]) -> Option<f32> {
        let seq = self.find(names)?;
        let fade = if self.skeleton.sequences[seq].snap { 0.0 } else { self.skeleton.sequences[self.now.seq].fade_out.clamp(0.0, 0.3) };
        self.before = (fade > 0.0).then_some((self.now, fade, fade));
        self.now = Playing { seq, time: 0.0 };
        self.duration(names)
    }

    /// The loop it rests in (the gravity gun's is another while it holds something); taken up when nothing else plays.
    pub fn set_idle(&mut self, names: &[&str]) {
        let Some(seq) = self.find(names) else { return };
        if seq == self.idle {
            return;
        }
        let resting = self.now.seq == self.idle;
        self.idle = seq;
        if resting {
            let fade = self.skeleton.sequences[self.now.seq].fade_out.clamp(0.0, 0.3);
            self.before = (fade > 0.0).then_some((self.now, fade, fade));
            self.now = Playing { seq, time: 0.0 };
        }
    }

    /// The sequence that is playing, by name (for the log).
    pub fn playing(&self) -> &str {
        &self.skeleton.sequences[self.now.seq].name
    }

    /// The clocks move on; a one-shot that has ended hands over to the idle loop.
    pub fn advance(&mut self, dt: f32) {
        self.now.time += dt;
        let s = &self.skeleton.sequences[self.now.seq];
        let length = (s.frames.len().max(2) - 1) as f32 / s.fps.max(1.0);
        if !s.looping && self.now.time >= length && self.now.seq != self.idle {
            let fade = s.fade_out.clamp(0.0, 0.3);
            self.before = (fade > 0.0).then_some((Playing { seq: self.now.seq, time: length }, fade, fade));
            self.now = Playing { seq: self.idle, time: 0.0 };
        }
        if let Some((p, left, _)) = self.before.as_mut() {
            p.time += dt;
            *left -= dt;
            if *left <= 0.0 {
                self.before = None;
            }
        }
    }

    /// This frame's pose into the palette: `place` takes model space (Source units: x forward, y left, z up) to the render
    /// frame. Returns where the muzzle is in the render frame.
    pub fn pose(&mut self, place: Mat4, buffers: &mut Assets<ShaderBuffer>) -> Option<Vec3> {
        sample(&self.skeleton.sequences[self.now.seq], self.now.time, &mut self.local);
        if let Some((p, left, total)) = self.before {
            sample(&self.skeleton.sequences[p.seq], p.time, &mut self.blend);
            // (Source's `SimpleSpline` of the fade.)
            let k = (left / total).clamp(0.0, 1.0);
            let k = k * k * (3.0 - 2.0 * k);
            for (l, b) in self.local.iter_mut().zip(&self.blend) {
                let q = if l.1.dot(b.1) < 0.0 { -b.1 } else { b.1 };
                *l = (l.0.lerp(b.0, k), l.1.lerp(q, k).normalize());
            }
        }
        if let Some(s) = self.overlay.map(|s| &self.skeleton.sequences[s]) {
            let open = self.open.clamp(0.0, 1.0);
            for (i, l) in self.local.iter_mut().enumerate() {
                let w = s.weights[i].clamp(0.0, 1.0);
                if w <= 0.0 {
                    continue;
                }
                let ((p0, q0), (p1, q1)) = (s.frames[0][i], s.blend_to[i]);
                let (q0, q1) = (Quat::from_array(q0), Quat::from_array(q1));
                let q = q0.lerp(if q0.dot(q1) < 0.0 { -q1 } else { q1 }, open).normalize();
                let p = Vec3::from_array(p0).lerp(Vec3::from_array(p1), open);
                *l = (l.0.lerp(p, w), l.1.lerp(if l.1.dot(q) < 0.0 { -q } else { q }, w).normalize());
            }
        }
        for (i, bone) in self.skeleton.bones.iter().enumerate() {
            let m = Mat4::from_rotation_translation(self.local[i].1, self.local[i].0);
            self.world[i] = if bone.parent >= 0 { self.world[bone.parent as usize] * m } else { m };
        }
        for i in 0..self.world.len() {
            let m = place * self.world[i] * self.pose_to_bone[i];
            self.bytes[i * 64..(i + 1) * 64].copy_from_slice(bytemuck::cast_slice(&m.to_cols_array()));
        }
        if let Some(mut b) = buffers.get_mut(&self.palette) {
            if let Some(d) = b.cast_slice_mut::<u8>() {
                let n = d.len().min(self.bytes.len());
                d[..n].copy_from_slice(&self.bytes[..n]);
            }
        }
        self.muzzle.map(|(bone, local)| (place * self.world[bone] * local).transform_point3(Vec3::ZERO))
    }
}

/// How the weapon rides on the view: Half-Life 2's bob and the view model's lag.
#[derive(Clone, Copy, Debug)]
pub struct Sway {
    bob_time: f32,
    /// The facing the model trails behind (the frame's axes).
    last_facing: Vec3,
}

impl Default for Sway {
    fn default() -> Self {
        Sway { bob_time: 0.0, last_facing: Vec3::X }
    }
}

/// Source's `AngleVectors`: forward, left and up of angles in degrees (pitch down, yaw to the left, roll to the right),
/// in axes x east, y north, z up.
pub fn angle_axes(pitch: f32, yaw: f32, roll: f32) -> Mat3 {
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sr, cr) = roll.to_radians().sin_cos();
    let forward = Vec3::new(cp * cy, cp * sy, -sp);
    let left = Vec3::new(sp * sr * cy - cr * sy, sp * sr * sy + cr * cy, sr * cp);
    let up = Vec3::new(sp * cr * cy + sr * sy, sp * cr * sy - sr * cy, cr * cp);
    Mat3::from_cols(forward, left, up)
}

impl Sway {
    /// The view model's place for this frame: its origin from the eyes (units, the frame's axes) and its axes (forward,
    /// left, up), from the view's angles (radians: `yaw` to the left of east, `pitch` up) and his speed over the ground
    /// (units/s).
    pub fn place(&mut self, yaw: f32, pitch: f32, speed: f32, dt: f32) -> (Vec3, Mat3) {
        // `CalcViewmodelBob`.
        let speed = speed.clamp(0.0, 320.0);
        self.bob_time += dt * speed / 320.0;
        let wave = |period: f32| {
            let c = (self.bob_time - (self.bob_time / period).floor() * period) / period;
            let c = if c < 0.5 { std::f32::consts::PI * c / 0.5 } else { std::f32::consts::PI + std::f32::consts::PI * (c - 0.5) / 0.5 };
            let amp = speed * 0.005;
            (amp * 0.3 + amp * 0.7 * c.sin()).clamp(-7.0, 4.0)
        };
        let (vertical, lateral) = (wave(0.45), wave(0.9));
        // `AddViewmodelBob`, on the eyes' own angles.
        let (pitch_deg, yaw_deg) = (-pitch.to_degrees(), yaw.to_degrees());
        let eye = angle_axes(pitch_deg, yaw_deg, 0.0);
        let (forward, right, up) = (eye.x_axis, -eye.y_axis, eye.z_axis);
        let mut origin = forward * (vertical * 0.1) + Vec3::Z * (vertical * 0.1) + right * (lateral * 0.8);
        let axes = angle_axes(pitch_deg - vertical * 0.4, yaw_deg - lateral * 0.3, vertical * 0.5);
        // `CalcViewModelLag`: the model trails the facing, and sits lower and nearer as he looks down.
        if dt > 0.0 {
            let diff = axes.x_axis - self.last_facing;
            let mut rate = 5.0;
            let far = diff.length();
            if far > 1.5 {
                rate *= far / 1.5;
            }
            self.last_facing = (self.last_facing + diff * (rate * dt).min(1.0)).try_normalize().unwrap_or(axes.x_axis);
            origin -= diff * 5.0;
        }
        origin += forward * (-pitch_deg * 0.035) + right * (-pitch_deg * 0.03) + up * (-pitch_deg * 0.02);
        (origin, axes)
    }

    /// Start the lag from this facing (a weapon just taken, a view just set).
    pub fn settle(&mut self, yaw: f32, pitch: f32) {
        self.last_facing = angle_axes(-pitch.to_degrees(), yaw.to_degrees(), 0.0).x_axis;
    }
}

/// Model space to the render frame: `origin` (units) and `axes` in the sandbox's frame, `to_render` that frame's turn to
/// the render frame's axes (the camera is the render frame's origin, at his eyes).
pub fn placement(origin: Vec3, axes: Mat3, to_render: Quat) -> Mat4 {
    let turn = Mat3::from_quat(to_render) * axes;
    Mat4::from_cols(turn.x_axis.extend(0.0) * UNIT_M, turn.y_axis.extend(0.0) * UNIT_M, turn.z_axis.extend(0.0) * UNIT_M, (to_render * (origin * UNIT_M)).extend(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_axes_are_sources() {
        // Level, facing east: forward +x, left +y, up +z.
        let m = angle_axes(0.0, 0.0, 0.0);
        assert!((m.x_axis - Vec3::X).length() < 1e-6 && (m.y_axis - Vec3::Y).length() < 1e-6 && (m.z_axis - Vec3::Z).length() < 1e-6);
        // Yaw 90 turns left to north; pitch 30 looks down.
        let m = angle_axes(30.0, 90.0, 0.0);
        assert!((m.x_axis - Vec3::new(0.0, 0.866_025, -0.5)).length() < 1e-5);
        assert!((m.y_axis - Vec3::NEG_X).length() < 1e-5);
        // Always a right-handed, orthonormal frame, roll and all.
        let m = angle_axes(-41.0, 213.0, 17.0);
        assert!((m.x_axis.cross(m.y_axis) - m.z_axis).length() < 1e-5);
        assert!((m.determinant() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_bob_and_the_lag_are_half_life_2s() {
        // Standing still, looking level east: at the eyes, along the view.
        let mut s = Sway::default();
        s.settle(0.0, 0.0);
        let (o, a) = s.place(0.0, 0.0, 0.0, 0.015);
        assert!(o.length() < 1e-5 && (a.x_axis - Vec3::X).length() < 1e-6);
        // At a run the bob's two waves stay inside the SDK's clamps and move the model by under a unit and a half.
        let mut most = 0.0f32;
        for _ in 0..400 {
            let (o, _) = s.place(0.0, 0.0, 320.0, 0.015);
            most = most.max(o.length());
        }
        assert!(most > 0.3 && most < 1.6, "{most}");
        // A sudden quarter turn: the model is left behind by 5 units of the facing's change, and catches up.
        let mut s = Sway::default();
        s.settle(0.0, 0.0);
        let (o, _) = s.place(std::f32::consts::FRAC_PI_2, 0.0, 0.0, 0.015);
        let diff = Vec3::Y - Vec3::X;
        assert!((o + diff * 5.0).length() < 0.05, "{o}");
        let mut last = o;
        for _ in 0..200 {
            last = s.place(std::f32::consts::FRAC_PI_2, 0.0, 0.0, 0.015).0;
        }
        assert!(last.length() < 0.01, "{last}");
        // Looking 40 degrees down: 1.4 units back, 1.2 to the left, 0.8 down (the pitch's terms).
        let mut s = Sway::default();
        let pitch = -40f32.to_radians();
        s.settle(0.0, pitch);
        let (o, a) = s.place(0.0, pitch, 0.0, 0.015);
        let eye = angle_axes(40.0, 0.0, 0.0);
        assert!((o.dot(eye.x_axis) + 1.4).abs() < 1e-3 && (o.dot(eye.y_axis) - 1.2).abs() < 1e-3 && (o.dot(eye.z_axis) + 0.8).abs() < 1e-3, "{o}");
        assert!((a.x_axis - eye.x_axis).length() < 1e-5);
    }

    #[test]
    fn weights_sum_to_a_whole() {
        for w in [[1.0, 0.0, 0.0], [0.5, 0.5, 0.0], [0.333, 0.333, 0.334], [0.9, 0.06, 0.04], [0.0, 0.0, 0.0]] {
            let b = weight_bytes(w);
            assert_eq!(b.iter().map(|x| *x as u32).sum::<u32>(), 255, "{w:?} {b:?}");
        }
    }

    /// With the game installed: every view model reads, plays its sequences and keeps its hands near its origin.
    #[test]
    fn the_games_view_models_pose() {
        let Some(root) = assets::find_install() else {
            eprintln!("Garry's Mod is not installed: skipped");
            return;
        };
        let install = GmodInstall::open(&root).unwrap();
        for w in &super::super::weapons::WEAPONS {
            let data = load(&install, w.view).unwrap_or_else(|e| panic!("{}: {e}", w.view));
            let n = data.skeleton.bones.len();
            let by_name: HashMap<&str, usize> = data.skeleton.sequences.iter().enumerate().map(|(i, s)| (s.name.as_str(), i)).collect();
            for name in w.sequences() {
                assert!(by_name.contains_key(name), "{} has no sequence {name}: it has {:?}", w.view, by_name.keys().collect::<Vec<_>>());
            }
            // Skin every vertex by the idle's first frame: it stays within a few units of where a hand can be.
            let idle = ["idle01", "idle"].iter().find_map(|n| by_name.get(*n).copied()).unwrap();
            let mut local = vec![(Vec3::ZERO, Quat::IDENTITY); n];
            sample(&data.skeleton.sequences[idle], 0.0, &mut local);
            let mut world = vec![Mat4::IDENTITY; n];
            for (i, b) in data.skeleton.bones.iter().enumerate() {
                let m = Mat4::from_rotation_translation(local[i].1, local[i].0);
                world[i] = if b.parent >= 0 { world[b.parent as usize] * m } else { m };
            }
            let skin: Vec<Mat4> = (0..n).map(|i| world[i] * rows(&data.skeleton.bones[i].pose_to_bone)).collect();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for m in &data.meshes {
                for (v, p) in m.positions.iter().enumerate() {
                    let (j, wt) = (m.joints[v], m.weights[v]);
                    let mut at = Vec3::ZERO;
                    for k in 0..3 {
                        at += skin[j[k] as usize].transform_point3(Vec3::from_array(*p)) * (wt[k] as f32 / 255.0);
                    }
                    lo = lo.min(at);
                    hi = hi.max(at);
                }
            }
            eprintln!("{}: {n} bones, {} sequences, idle pose from {lo:.1} to {hi:.1} units", w.view, data.skeleton.sequences.len());
            assert!(lo.min_element() > -120.0 && hi.max_element() < 120.0);
            // Forward of the eyes (the model's +x): what is drawn is mostly ahead.
            assert!(hi.x > 5.0, "{} is behind the eyes", w.view);
        }
    }
}
