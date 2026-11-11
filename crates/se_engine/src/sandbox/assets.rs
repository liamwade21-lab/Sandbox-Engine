//! What the sandbox reads from the Garry's Mod install (`se_core::gmod`): prop models with their textures and collision
//! pieces, the stock spawn lists, and sounds. Read at run time, never copied into the project or changed there. Found through
//! Steam (app 4000), or `GMOD_PATH`. Without the game the sandbox still works: a handful of plain props made here, and plain
//! synthesised sounds (`placeholder_*`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::audio::AudioSource;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use se_core::dds::DdsFormat;
use se_core::gmod::{GmodInstall, GmodTexture, SpawnList};

use super::light::{PropLight, PropMaterial};
use super::movement::UNIT_M;
use super::world::{PropShape, Surface};

/// Garry's Mod's Steam app.
const GMOD_APP: &str = "4000";

/// One mesh of a prop, as read: metres, the model's own axes (x forward, y left, z up).
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub material: MatData,
}

pub struct MatData {
    /// What the material is cached by.
    pub key: String,
    pub texture: Option<GmodTexture>,
    pub tint: [f32; 3],
    pub alpha_cut: f32,
    pub two_sided: bool,
}

/// A prop as read from the install (or made up): no GPU objects yet, so a worker thread can read it.
pub struct PropData {
    pub meshes: Vec<MeshData>,
    pub shape: PropShape,
    pub mass: f32,
    pub surface: Surface,
    pub min: Vec3,
    pub max: Vec3,
}

/// A prop ready to spawn.
pub struct PropAsset {
    pub parts: Vec<(Handle<Mesh>, Handle<PropMaterial>)>,
    pub shape: PropShape,
    pub mass: f32,
    pub surface: Surface,
    pub min: Vec3,
    pub max: Vec3,
}

/// A category of the Q menu's props.
#[derive(Clone, Debug)]
pub struct PropList {
    pub name: String,
    /// How deep under its parents it is.
    pub depth: usize,
    pub models: Vec<String>,
}

#[derive(Resource, Default)]
pub struct GmodAssets {
    pub install: Option<Arc<GmodInstall>>,
    /// The search for the install is over (with or without one).
    pub looked: bool,
    pub lists: Vec<PropList>,
    props: HashMap<String, Option<PropAsset>>,
    materials: HashMap<String, Handle<PropMaterial>>,
    sounds: HashMap<String, Option<Handle<AudioSource>>>,
    white: Option<Handle<Image>>,
}

/// The Garry's Mod folder: `GMOD_PATH`, else where Steam has it installed.
pub fn find_install() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GMOD_PATH").map(PathBuf::from) {
        return p.is_dir().then_some(p);
    }
    crate::ownership::steam_app_dir(GMOD_APP)
}

/// Open the install and read its spawn lists (a worker thread's job: half a second).
pub fn open_install() -> (Option<Arc<GmodInstall>>, Vec<PropList>, String) {
    let Some(root) = find_install() else {
        return (None, placeholder_lists(), "Garry's Mod is not installed: plain props and plain sounds".to_string());
    };
    match GmodInstall::open(&root) {
        Ok(install) => {
            let lists = prop_lists(&install, install.spawnlists());
            let n: usize = lists.iter().map(|l| l.models.len()).sum();
            let line = format!("Garry's Mod's files are read from {} ({} spawn lists, {n} props)", root.display(), lists.len());
            (Some(Arc::new(install)), if lists.is_empty() { placeholder_lists() } else { lists }, line)
        }
        Err(e) => (None, placeholder_lists(), format!("Garry's Mod's files at {} can't be read ({e}): plain props and plain sounds", root.display())),
    }
}

/// The stock spawn lists as the menu shows them: in their tree's order, only the models the install really has (the lists
/// also name other games' props), empty lists dropped unless they have children.
fn prop_lists(install: &GmodInstall, lists: Vec<SpawnList>) -> Vec<PropList> {
    let mut out = Vec::new();
    fn walk(install: &GmodInstall, lists: &[SpawnList], parent: u32, depth: usize, out: &mut Vec<PropList>) {
        for l in lists.iter().filter(|l| l.parent == parent && l.id != parent) {
            let mut seen = std::collections::HashSet::new();
            let models: Vec<String> = l.models.iter().filter(|m| seen.insert((*m).clone()) && install.exists(m)).cloned().collect();
            let at = out.len();
            out.push(PropList { name: l.name.clone(), depth, models });
            if depth < 4 {
                walk(install, lists, l.id, depth + 1, out);
            }
            if out.len() == at + 1 && out[at].models.is_empty() {
                out.pop();
            }
        }
    }
    walk(install, &lists, 0, 0, &mut out);
    out
}

const PLACEHOLDERS: [(&str, &str); 6] = [
    ("placeholder/crate", "Crate"),
    ("placeholder/barrel", "Barrel"),
    ("placeholder/ball", "Ball"),
    ("placeholder/plank", "Plank"),
    ("placeholder/block", "Concrete block"),
    ("placeholder/can", "Can"),
];

fn placeholder_lists() -> Vec<PropList> {
    vec![PropList { name: "Basic props".to_string(), depth: 0, models: PLACEHOLDERS.iter().map(|p| p.0.to_string()).collect() }]
}

/// A prop's name for the menu: its file's.
pub fn prop_name(path: &str) -> String {
    if let Some(p) = PLACEHOLDERS.iter().find(|p| p.0 == path) {
        return p.1.to_string();
    }
    path.rsplit('/').next().unwrap_or(path).trim_end_matches(".mdl").to_string()
}

/// Read a prop: its meshes, its materials' textures, its collision pieces. Any thread.
pub fn load_prop(install: Option<&GmodInstall>, path: &str) -> Result<PropData, String> {
    if path.starts_with("placeholder/") {
        return placeholder_prop(path).ok_or_else(|| format!("no placeholder {path}"));
    }
    let install = install.ok_or("Garry's Mod is not installed")?;
    let model = install.model(path).map_err(|e| e.to_string())?;
    let mut meshes = Vec::new();
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for m in model.meshes {
        if m.indices.is_empty() || m.positions.is_empty() {
            continue;
        }
        let positions: Vec<[f32; 3]> = m.positions.iter().map(|p| [p[0] * UNIT_M, p[1] * UNIT_M, p[2] * UNIT_M]).collect();
        for p in &positions {
            min = min.min(Vec3::from_array(*p));
            max = max.max(Vec3::from_array(*p));
        }
        let mat = install.material(&m.material);
        let texture = mat.as_ref().and_then(|x| x.base_texture.as_ref()).and_then(|t| install.texture(t).ok());
        let (alpha_test, translucent, no_cull, color) = mat.as_ref().map_or((false, false, false, [1.0; 3]), |x| (x.alpha_test, x.translucent, x.no_cull, x.color));
        let has_alpha = texture.as_ref().is_some_and(|t| t.has_alpha);
        let alpha_cut = if alpha_test && has_alpha {
            0.5
        } else if translucent && has_alpha {
            0.3
        } else {
            0.0
        };
        // (No texture: Source's missing-material purple would only look like a bug here; a plain grey.)
        let tint = if texture.is_some() { color } else { [0.5, 0.5, 0.5] };
        meshes.push(MeshData { positions, normals: m.normals, uvs: m.uvs, indices: m.indices, material: MatData { key: m.material, texture, tint, alpha_cut, two_sided: no_cull || alpha_cut > 0.0 } });
    }
    if meshes.is_empty() {
        return Err(format!("{path} has no mesh"));
    }
    let shape = collision_shape(install.collision(path), &meshes, min, max);
    // (Source's masses are kilograms; a model without one gets water's weight for a tenth of its box.)
    let size = max - min;
    let mass = if model.mass > 0.0 { model.mass } else { (size.x * size.y * size.z * 100.0).clamp(1.0, 500.0) };
    Ok(PropData { meshes, shape, mass, surface: Surface::named(&model.surface_prop), min, max })
}

/// The model's own collision pieces where they fit its mesh, else the mesh's convex hull. (A tenth of the models keep their
/// pieces in another pose's frame than the mesh's: those would stand beside what is drawn.)
fn collision_shape(pieces: Option<Vec<Vec<[f32; 3]>>>, meshes: &[MeshData], min: Vec3, max: Vec3) -> PropShape {
    let size = max - min;
    let slack = size * 0.15 + Vec3::splat(0.03);
    if let Some(pieces) = pieces {
        let pieces: Vec<Vec<Vec3>> = pieces.into_iter().map(|p| p.into_iter().map(|v| Vec3::from_array(v) * UNIT_M).collect::<Vec<Vec3>>()).filter(|p| p.len() >= 4).collect();
        let all = pieces.iter().flatten();
        let (lo, hi) = all.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |a, p| (a.0.min(*p), a.1.max(*p)));
        let inside = lo.cmpge(min - slack).all() && hi.cmple(max + slack).all();
        let covers = ((hi - lo) + Vec3::splat(0.02)).cmpge(size * 0.6).all();
        if !pieces.is_empty() && inside && covers {
            return PropShape::Hulls(pieces);
        }
    }
    let points: Vec<Vec3> = meshes.iter().flat_map(|m| m.positions.iter().map(|p| Vec3::from_array(*p))).collect();
    if points.len() >= 4 && size.min_element() > 0.005 {
        PropShape::Hulls(vec![points])
    } else {
        PropShape::Box { centre: (min + max) * 0.5, half: (size * 0.5).max(Vec3::splat(0.01)) }
    }
}

/// A plain prop for a computer without Garry's Mod: a Bevy primitive in the model's axes (z up) with a flat colour.
fn placeholder_prop(path: &str) -> Option<PropData> {
    let (mesh, shape, mass, surface, tint): (Mesh, PropShape, f32, Surface, [f32; 3]) = match path {
        "placeholder/crate" => (Cuboid::new(0.8, 0.8, 0.8).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::splat(0.4) }, 30.0, Surface::WoodCrate, [0.55, 0.38, 0.2]),
        "placeholder/barrel" => (Cylinder::new(0.28, 0.86).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::new(0.22, 0.22, 0.43) }, 30.0, Surface::MetalBarrel, [0.2, 0.3, 0.45]),
        "placeholder/ball" => (Sphere::new(0.3).mesh().ico(4).ok()?, PropShape::Ball { centre: Vec3::ZERO, radius: 0.3 }, 5.0, Surface::Rubber, [0.75, 0.15, 0.12]),
        "placeholder/plank" => (Cuboid::new(2.0, 0.25, 0.05).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::new(1.0, 0.125, 0.025) }, 12.0, Surface::Wood, [0.6, 0.45, 0.28]),
        "placeholder/block" => (Cuboid::new(0.4, 0.2, 0.2).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::new(0.2, 0.1, 0.1) }, 20.0, Surface::Concrete, [0.55, 0.55, 0.52]),
        "placeholder/can" => (Cylinder::new(0.033, 0.12).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::new(0.03, 0.03, 0.06) }, 0.4, Surface::Metal, [0.7, 0.1, 0.1]),
        // (The weapon in his hands without the game: a gun's and a bar's rough shapes, pointing along the model's +y.)
        "placeholder/gun" => (Cuboid::new(0.045, 0.1, 0.34).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::new(0.0225, 0.17, 0.05) }, 2.0, Surface::Metal, [0.16, 0.17, 0.19]),
        "placeholder/bar" => (Cuboid::new(0.03, 0.03, 0.7).into(), PropShape::Box { centre: Vec3::ZERO, half: Vec3::new(0.015, 0.35, 0.015) }, 2.0, Surface::Metal, [0.55, 0.12, 0.1]),
        _ => return None,
    };
    // Bevy's primitives stand along y: turned to the model's z (the weapons' stand-ins lie along the model's y).
    let lying = matches!(path, "placeholder/gun" | "placeholder/bar");
    let turn = |p: [f32; 3]| if lying { [p[0], p[2], -p[1]] } else { [p[0], -p[2], p[1]] };
    let read = |attr| match mesh.attribute(attr) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.iter().map(|p| turn(*p)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let positions = read(Mesh::ATTRIBUTE_POSITION);
    let normals = read(Mesh::ATTRIBUTE_NORMAL);
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(bevy::mesh::VertexAttributeValues::Float32x2(v)) => v.clone(),
        _ => vec![[0.0; 2]; positions.len()],
    };
    let indices: Vec<u32> = mesh.indices().map(|i| i.iter().map(|i| i as u32).collect()).unwrap_or_default();
    let (min, max) = positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |a, p| (a.0.min(Vec3::from_array(*p)), a.1.max(Vec3::from_array(*p))));
    let material = MatData { key: path.to_string(), texture: None, tint, alpha_cut: 0.0, two_sided: false };
    Some(PropData { meshes: vec![MeshData { positions, normals, uvs, indices, material }], shape, mass, surface, min, max })
}

/// A Bevy mesh of one of a prop's meshes.
pub fn bevy_mesh(m: &MeshData) -> Mesh {
    let n = m.positions.len();
    let mut normals = m.normals.clone();
    normals.resize(n, [0.0, 0.0, 1.0]);
    let mut uvs = m.uvs.clone();
    uvs.resize(n, [0.0, 0.0]);
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(m.indices.iter().copied().filter(|i| (*i as usize) < n).collect::<Vec<u32>>()));
    mesh
}

/// A Bevy image of one of the game's textures: block-compressed data as it is where the card takes it (sizes in fours),
/// decoded here otherwise. Colour: sRGB. Repeating, trilinear.
pub fn bevy_image(t: &GmodTexture) -> Option<Image> {
    if t.mips.is_empty() || t.width == 0 || t.height == 0 {
        return None;
    }
    let aligned = t.width % 4 == 0 && t.height % 4 == 0;
    let block = match t.format {
        DdsFormat::Bc1 => Some(TextureFormat::Bc1RgbaUnormSrgb),
        DdsFormat::Bc2 => Some(TextureFormat::Bc2RgbaUnormSrgb),
        DdsFormat::Bc3 => Some(TextureFormat::Bc3RgbaUnormSrgb),
        _ => None,
    };
    let (format, data, levels) = match block.filter(|_| aligned) {
        Some(f) => (f, t.mips.concat(), t.mips.len() as u32),
        None => {
            let mut data = Vec::new();
            for mip in 0..t.mips.len() {
                data.extend(t.rgba(mip));
            }
            (TextureFormat::Rgba8UnormSrgb, data, t.mips.len() as u32)
        }
    };
    let mut image = Image::new_uninit(Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 }, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    Some(image)
}

/// What the GPU side needs to make a prop's parts.
pub struct Gpu<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub images: &'a mut Assets<Image>,
    pub materials: &'a mut Assets<PropMaterial>,
    pub light: &'a PropLight,
}

impl GmodAssets {
    fn white(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        self.white
            .get_or_insert_with(|| images.add(Image::new_fill(Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2, &[255, 255, 255, 255], TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)))
            .clone()
    }

    /// A material of the sandbox's own: a texture (or none) and a tint, lit by the sandbox's light.
    pub fn material(&mut self, gpu: &mut Gpu, m: &MatData, glow: f32) -> Handle<PropMaterial> {
        let key = format!("{}|{glow}", m.key);
        if let Some(h) = self.materials.get(&key) {
            return h.clone();
        }
        let diffuse = match m.texture.as_ref().and_then(bevy_image) {
            Some(image) => gpu.images.add(image),
            None => self.white(gpu.images),
        };
        let h = gpu.materials.add(PropMaterial {
            frame: gpu.light.frame.clone(),
            light: gpu.light.light.clone(),
            diffuse,
            tint: Vec3::from_array(m.tint).extend(m.alpha_cut),
            glow: Vec4::new(glow, if m.two_sided { 1.0 } else { 0.0 }, 0.0, 0.0),
        });
        self.materials.insert(key, h.clone());
        h
    }

    /// GPU parts of a prop as read.
    pub fn upload(&mut self, gpu: &mut Gpu, data: PropData) -> PropAsset {
        let parts = data.meshes.iter().map(|m| (gpu.meshes.add(bevy_mesh(m)), self.material(gpu, &m.material, 0.0))).collect();
        PropAsset { parts, shape: data.shape, mass: data.mass, surface: data.surface, min: data.min, max: data.max }
    }

    /// A prop by its path, read the first time it's asked for (a few milliseconds; `None` if it can't be read).
    pub fn prop(&mut self, gpu: &mut Gpu, path: &str) -> Option<&PropAsset> {
        if !self.props.contains_key(path) {
            let loaded = match load_prop(self.install.as_deref(), path) {
                Ok(data) => Some(self.upload(gpu, data)),
                Err(e) => {
                    warn!("sandbox: {path} can't be read: {e}");
                    None
                }
            };
            self.props.insert(path.to_string(), loaded);
        }
        self.props.get(path).and_then(|p| p.as_ref())
    }

    /// A sound by its path under `sound/`, decoded the first time; a plain synthesised one of its kind without the game.
    pub fn sound(&mut self, sources: &mut Assets<AudioSource>, path: &str) -> Option<Handle<AudioSource>> {
        if let Some(h) = self.sounds.get(path) {
            return h.clone();
        }
        let wav = match self.install.as_ref().map(|i| i.sound(path)) {
            Some(Ok(pcm)) => Some(pcm.to_wav()),
            Some(Err(e)) => {
                warn!("sandbox: the sound {path} can't be read: {e}");
                placeholder_sound(path)
            }
            None => placeholder_sound(path),
        };
        let h = wav.map(|bytes| sources.add(AudioSource { bytes: Arc::from(bytes) }));
        self.sounds.insert(path.to_string(), h.clone());
        h
    }
}

/// A plain sound of the kind the path names (a shot, a click, a step, a thud), for a computer without the game: noise and a
/// tone under an envelope, as a 16-bit mono WAV.
fn placeholder_sound(path: &str) -> Option<Vec<u8>> {
    // (seconds, the tone's pitch in Hz, how much is noise, how fast it dies away)
    let (secs, hz, noise, decay) = if path.contains("fire") || path.contains("launch") {
        (0.35, 110.0, 0.85, 14.0)
    } else if path.contains("footsteps") {
        (0.12, 90.0, 0.7, 30.0)
    } else if path.starts_with("physics/") || path.contains("impact") || path.contains("swing") {
        (0.18, 140.0, 0.6, 22.0)
    } else if path.contains("reload") || path.contains("cock") || path.contains("empty") || path.contains("ui_") || path.contains("button") || path.contains("wpn_") {
        (0.06, 900.0, 0.5, 60.0)
    } else if path.contains("loop") {
        return None;
    } else {
        (0.1, 440.0, 0.3, 30.0)
    };
    let rate = 22_050u32;
    let n = (secs * rate as f32) as usize;
    let mut seed = path.bytes().fold(0x9E37_79B9u32, |a, b| a.rotate_left(5) ^ b as u32);
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
            let tone = (t * hz * std::f32::consts::TAU).sin();
            ((white * noise + tone * (1.0 - noise)) * (-t * decay).exp() * 0.6 * 32767.0) as i16
        })
        .collect();
    Some(se_core::gmod::Pcm { rate, channels: 1, samples, loop_start: None }.to_wav())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_load_and_stand_on_their_z() {
        for (path, _) in PLACEHOLDERS {
            let p = load_prop(None, path).unwrap();
            assert!(p.mass > 0.0 && !p.meshes[0].indices.is_empty(), "{path}");
            assert!(p.max.cmpgt(p.min).all(), "{path}");
            assert_eq!(p.meshes[0].positions.len(), p.meshes[0].normals.len());
        }
        // The barrel is taller than it is wide: Bevy's cylinder was turned onto z.
        let b = load_prop(None, "placeholder/barrel").unwrap();
        assert!((b.max.z - b.min.z) > (b.max.x - b.min.x));
        assert!(load_prop(None, "models/props_c17/oildrum001.mdl").is_err());
        assert_eq!(prop_name("models/props_c17/oildrum001.mdl"), "oildrum001");
    }

    #[test]
    fn placeholder_sounds_are_wav_files() {
        for path in ["weapons/pistol/pistol_fire2.wav", "player/footsteps/grass1.wav", "physics/wood/wood_crate_impact_hard2.wav", "garrysmod/ui_click.wav"] {
            let wav = placeholder_sound(path).unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert!(wav.len() > 1000, "{path}");
        }
        assert!(placeholder_sound("weapons/physcannon/hold_loop.wav").is_none());
    }

    /// With the game installed: its stock props load with collision pieces that fit them.
    #[test]
    fn the_games_props_load() {
        let Some(root) = find_install() else {
            eprintln!("Garry's Mod is not installed: skipped");
            return;
        };
        let install = GmodInstall::open(&root).unwrap();
        let lists = prop_lists(&install, install.spawnlists());
        assert!(lists.len() >= 10 && lists.iter().any(|l| l.models.iter().any(|m| m.ends_with("oildrum001.mdl"))));
        for path in ["models/props_c17/oildrum001.mdl", "models/props_junk/wood_crate001a.mdl", "models/props_c17/furniturecouch001a.mdl", "models/props_junk/watermelon01.mdl"] {
            let p = load_prop(Some(&install), path).unwrap();
            let size = p.max - p.min;
            assert!(size.max_element() > 0.2 && size.max_element() < 3.0, "{path}: {size:?} m");
            assert!(p.mass >= 1.0, "{path}");
            assert!(matches!(p.shape, PropShape::Hulls(ref h) if !h.is_empty()), "{path}");
            assert!(p.meshes.iter().all(|m| m.material.texture.is_some()), "{path}: a texture");
            let image = bevy_image(p.meshes[0].material.texture.as_ref().unwrap()).unwrap();
            assert!(image.texture_descriptor.mip_level_count >= 1);
        }
        // An oil drum is 0.86 m tall at Valve's scale.
        let drum = load_prop(Some(&install), "models/props_c17/oildrum001.mdl").unwrap();
        assert!(((drum.max.z - drum.min.z) - 0.86).abs() < 0.08, "{}", drum.max.z - drum.min.z);
        assert_eq!(drum.surface, Surface::MetalBarrel);
    }
}
