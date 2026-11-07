//! The scatter's models: their meshes packed into one vertex and one index buffer, their
//! materials, the (model, LOD) buckets with their caps, the indirect draws, and the impostor atlas
//! baked on a worker thread (`impostor.rs`).
//!
//! The models, in order: Megaton Rainfall's 16 broadleaf trees; Earth's 8 generated species
//! (conifers, palms, an acacia, shrubs: `trees.rs`); the active planet's flora (8 generated species,
//! made again with its impostors when a generated life planet becomes the active body); 4 grass
//! tufts. The materials are fixed: MR's trees' own, then MR's 9 barks and 8 branch cards, the port's
//! conifer and palm cards, and MR's grass atlas.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use se_core::dds::decode_level;
use se_core::mr::MrInstall;

use super::impostor::{self, BakeModel, BakeSub, Baked};
use super::trees::{self, Leaf, Recipe};
use crate::mr_assets::MrAssets;

/// A vertex of the packed meshes (`scatter_draw.wesl` `VertexIn`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct LibVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// What a draw draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawKind {
    /// A mesh sub-mesh with its material.
    Mesh { material: u32, leaves: bool },
    /// The model's impostor quads.
    Impostor,
    /// The buildings' boxes (one instance a building, sized by its record).
    Building { material: u32 },
}

/// One indirect draw: a bucket's instances drawn with an index range.
#[derive(Clone, Copy, Debug)]
pub struct DrawEntry {
    pub bucket: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    pub kind: DrawKind,
}

/// A material: its colour texture and tint (linear, MR's `g_vDiffuse`).
#[derive(Clone, Debug)]
pub struct LibMaterial {
    pub color: Handle<Image>,
    pub tint: [f32; 4],
    pub leaves: bool,
    /// The buildings' box (its draws take the building pipeline: `scatter_draw.wesl` `vertex_building`).
    pub building: bool,
}

/// A model's numbers the shaders need (`scatter.wesl` / `scatter_draw.wesl` `Model`).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ModelGpu {
    /// The impostor quad's half size (m) and its centre's height above the base (m).
    pub half: f32,
    pub centre_y: f32,
    /// Bounding sphere radius (m) about the centre, for the view test.
    pub radius: f32,
    /// The first impostor layer of the model in the atlas.
    pub layer0: u32,
    /// The crown's centre height (m) for the leaves' ball normals, and the crown's radius.
    pub crown_y: f32,
    pub crown_r: f32,
    /// Mesh and impostor buckets.
    pub mesh_bucket: u32,
    pub impostor_bucket: u32,
    /// The mean colour of its impostor views (linear) and their coverage, for the tint that gives
    /// the trees the ground's forest colour (set when the bake is done).
    pub mean: [f32; 4],
    /// x 1 for a broadleaf tree (its leaves follow the seasons).
    pub flags: [u32; 4],
}

/// The library, extracted to the render world once ready (and when it changes).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct ScatterLibrary {
    /// Bumped when the content changes (the render world re-uploads it).
    pub generation: u32,
    /// The tree models (0..trees: MR's, Earth's species from `earth0`, the flora from `flora0`) and
    /// the grass tufts after them.
    pub trees: u32,
    pub grass: u32,
    pub earth0: u32,
    pub flora0: u32,
    /// The flora's key (`flora_key`) the flora models were made for.
    pub flora_key: u64,
    /// The buildings' box model (after the grass), `u32::MAX` when there is none.
    pub building: u32,
    pub vertices: Arc<Vec<LibVertex>>,
    pub indices: Arc<Vec<u32>>,
    pub materials: Vec<LibMaterial>,
    pub models: Vec<ModelGpu>,
    /// Per bucket: its capacity (instances).
    pub bucket_caps: Vec<u32>,
    pub draws: Vec<DrawEntry>,
    /// The impostor atlas: colour (sRGB, alpha cut-out) and object-space normals.
    pub impostor_color: Option<Handle<Image>>,
    pub impostor_normal: Option<Handle<Image>>,
}

impl ScatterLibrary {
    pub fn ready(&self) -> bool {
        self.generation > 0 && self.impostor_color.is_some()
    }
}

/// A flora the scatter wants for the active body (`Scatter::flora_want`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloraWant {
    pub key: u64,
    pub seed: u32,
    pub gravity: f32,
    pub pressure: f32,
}

/// The key of a flora: its seed and its gravity and air, rounded.
pub fn flora_key(seed: u32, gravity: f32, pressure: f32) -> u64 {
    (seed as u64) << 32 | ((gravity * 10.0).round() as u64 & 0xffff) << 16 | ((pressure * 100.0).round() as u64 & 0xffff)
}

pub struct LibraryPlugin;

impl Plugin for LibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScatterLibrary>()
            .add_plugins(ExtractResourcePlugin::<ScatterLibrary>::default())
            .add_systems(Startup, start_library)
            .add_systems(Update, (finish_bake, update_flora).chain());
    }
}

/// Instances per bucket (one model) at most: the meshes near the camera, the impostors farther out;
/// a grass tuft's meshes (about 2.5 a square metre within 120 m, over 4 tufts). Earth's and the
/// flora's species fill a forest with a few models (a taiga is three conifers): more each.
const MESH_CAP: u32 = 2048;
const IMPOSTOR_CAP: u32 = 16_384;
const SPECIES_MESH_CAP: u32 = 4096;
const SPECIES_IMPOSTOR_CAP: u32 = 40_000;
const GRASS_CAP: u32 = 40_000;
/// Buildings drawn at most (cities: the box class within its range).
const BUILDING_CAP: u32 = 120_000;
/// Huts and longhouses drawn at most (villages), each.
const VILLAGE_CAP: u32 = 30_000;

/// Buildings of level 3's cities (the future box, each tower, the skybridge) drawn at most.
const TOWER_CAP: u32 = 6000;
/// Flying lanes' segments drawn at most.
const LANE_CAP: u32 = 40_000;

/// The buildings' models in the library (`scatter.wesl` BUILDING_MODELS), the street objects after them.
pub const BUILDING_MODELS: usize = 15;
/// What the draw shader does with a street object (`ModelGpu::flags` y, `street_surface`).
const STREET_PLAIN: u32 = 1;
const STREET_CAR: u32 = 2;
const STREET_CAR_OWN: u32 = 3;
const STREET_LAMP: u32 = 4;
/// Megaton Rainfall's street furniture, with how many of each are drawn at most (within 250 m), in
/// `scatter.wesl`'s order (S_LAMP...). Its colours are its vertices' (or its material's).
pub(crate) const STREET_PROPS: [(&str, u32, u32); 10] = [
    ("streetlight_1", 512, STREET_LAMP),
    ("streetlight_3", 512, STREET_LAMP),
    ("streetlight_7", 512, STREET_LAMP),
    ("bench_0", 128, STREET_PLAIN),
    ("bench_4", 128, STREET_PLAIN),
    ("dustbin_0", 128, STREET_PLAIN),
    ("dustbin_2", 128, STREET_PLAIN),
    ("hydrant_0", 128, STREET_PLAIN),
    ("semaphore_1", 128, STREET_PLAIN),
    ("semaphore_3", 128, STREET_PLAIN),
];
/// Its cars (`scatter.wesl` S_CARS, S_CAR): each with its own texture; the taxi wears its own paint.
pub(crate) const STREET_CARS: [&str; 14] = [
    "vehicle_0", "vehicle_1", "vehicle_3", "vehicle_6", "vehicle_8", "vehicle_12", "vehicle_14", "vehicle_16", "vehicle_17", "vehicle_19",
    "vehicle_20", "vehicle_22", "vehicle_23", "vehicle_5",
];
const CAR_CAP: u32 = 128;

/// What a texture array's alpha holds: the file's own (none: opaque), the windows (the dark glass of
/// a facade), a glow (a neon sign's brightest channel).
#[derive(Clone, Copy, PartialEq)]
enum ArrayAlpha {
    Keep,
    Windows,
    Glow,
}

/// The villages' textures (`assets/textures/village/NAME.png`, made with the image generator): thatch,
/// mud brick, timber, stone, in a 512 px array with its mips; a blank array (flat colours) without.
fn village_array() -> Image {
    texture_array("village", &["thatch", "mud_brick", "timber", "stone"], 512, ArrayAlpha::Keep)
}

/// Level 3's facades and signs (`assets/textures/future/`, made with the image generator): a glass
/// curtain wall (8 storeys, 4 bays a tile), white panels with ribbon windows (7 storeys), gold fins
/// over dark glass (8 storeys) with their windows in alpha, and 16 neon signs (4 x 4) with their glow
/// in alpha.
fn future_array() -> Image {
    let mut a = texture_array("future", &["glass", "panels", "fins"], 1024, ArrayAlpha::Windows);
    if a.texture_descriptor.size.depth_or_array_layers != 3 {
        return a;
    }
    // The signs' layer (a blank one without them: no signs, the facades kept).
    let signs = texture_array("future", &["neon"], 1024, ArrayAlpha::Glow);
    let one = signs.data.filter(|_| signs.texture_descriptor.size.depth_or_array_layers == 1);
    if let Some(d) = a.data.as_mut() {
        let layer = d.len() / 3;
        d.extend(one.filter(|s| s.len() == layer).unwrap_or_else(|| vec![0; layer]));
        a.texture_descriptor.size.depth_or_array_layers = 4;
    }
    a
}

/// The images `assets/textures/DIR/NAME.png` in an array of `S` px layers with their mips; a blank
/// array (the shader's flat colours) when one is missing.
fn texture_array(dir: &str, names: &[&str], s: u32, alpha: ArrayAlpha) -> Image {
    let mut layers = Vec::new();
    for name in names {
        let rel = std::path::Path::new("textures").join(dir).join(format!("{name}.png"));
        let mut path = crate::config::asset_dir().join(&rel);
        if !path.is_file() {
            path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join(&rel);
        }
        match std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok()) {
            Some(img) => {
                let mut l = image::imageops::resize(&img.to_rgba8(), s, s, image::imageops::FilterType::Triangle);
                if alpha != ArrayAlpha::Keep {
                    for px in l.pixels_mut() {
                        let [r, g, b, _] = px.0;
                        px.0[3] = if alpha == ArrayAlpha::Glow {
                            r.max(g).max(b)
                        } else {
                            // The facade's dark glass: its windows.
                            let lum = (r as f32 + g as f32 + b as f32) / (3.0 * 255.0);
                            let t = ((lum - 0.12) / 0.18).clamp(0.0, 1.0);
                            (255.0 * (1.0 - t * t * (3.0 - 2.0 * t))) as u8
                        };
                    }
                }
                layers.push(l);
            }
            None => {
                warn!("scatter: building texture {} missing: flat colours", path.display());
                layers.clear();
                break;
            }
        }
    }
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
    if layers.is_empty() || layers.len() != names.len() {
        let mut blank = Image::new_fill(
            Extent3d { width: 1, height: 1, depth_or_array_layers: 2 },
            TextureDimension::D2,
            &[0, 0, 0, 0],
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        blank.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
        return blank;
    }
    // Every layer with its mip chain, layer after layer.
    let mips = s.trailing_zeros() + 1;
    let mut data = Vec::new();
    for l in &layers {
        let mut m = l.clone();
        for level in 0..mips {
            data.extend_from_slice(m.as_raw());
            if level + 1 < mips {
                let n = (s >> (level + 1)).max(1);
                m = image::imageops::resize(&m, n, n, image::imageops::FilterType::Triangle);
            }
        }
    }
    let mut image = Image::new_uninit(
        Extent3d { width: s, height: s, depth_or_array_layers: names.len() as u32 },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

pub(crate) type Mesh4 = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>);

/// A round hut in unit size (walls' diameter 1, height 1): walls to 0.42, a thatched cone from its
/// eave (radius 0.62: overhanging the walls, at 0.38) to the top, and the eave's underside.
fn round_hut() -> Mesh4 {
    // (The ring runs from +x towards +z: clockwise seen from above, so each strip's triangles are
    // wound as seen from outside: counter-clockwise.)
    const N: u32 = 16;
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let ring = |i: u32| {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        (a.cos(), a.sin())
    };
    // Walls.
    for i in 0..=N {
        let (c, s) = ring(i);
        for y in [0.0, 0.42] {
            p.push([0.5 * c, y, 0.5 * s]);
            n.push([c, 0.0, s]);
            uv.push([i as f32 / N as f32, y]);
        }
    }
    for i in 0..N {
        let b = i * 2;
        idx.extend([b, b + 1, b + 2, b + 1, b + 3, b + 2]);
    }
    // The cone: slope from (0.62, 0.38) to (0, 1).
    let (rr, y0) = (0.62f32, 0.38f32);
    let len = (rr * rr + (1.0 - y0) * (1.0 - y0)).sqrt();
    let (ny, nr) = (rr / len, (1.0 - y0) / len);
    let base = p.len() as u32;
    for i in 0..=N {
        let (c, s) = ring(i);
        p.push([rr * c, y0, rr * s]);
        n.push([nr * c, ny, nr * s]);
        uv.push([i as f32 / N as f32, 0.0]);
        p.push([0.0, 1.0, 0.0]);
        n.push([nr * c, ny, nr * s]);
        uv.push([i as f32 / N as f32, 1.0]);
    }
    for i in 0..N {
        let b = base + i * 2;
        idx.extend([b, b + 1, b + 2]);
    }
    // The eave's underside (a ring facing down).
    let base = p.len() as u32;
    for i in 0..=N {
        let (c, s) = ring(i);
        p.push([0.5 * c, y0, 0.5 * s]);
        p.push([rr * c, y0, rr * s]);
        n.extend([[0.0, -1.0, 0.0]; 2]);
        uv.extend([[0.0, 0.0]; 2]);
    }
    for i in 0..N {
        let b = base + i * 2;
        idx.extend([b, b + 2, b + 1, b + 1, b + 2, b + 3]);
    }
    (p, n, uv, idx)
}

/// A longhouse in unit size (x across 1, z along 1, height 1): walls to 0.5, a thatched gable roof
/// with its ridge along z at the top, its eaves at 0.45 overhanging to 0.6 across, and the gables.
fn longhouse() -> Mesh4 {
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut quad = |c: [[f32; 3]; 4], normal: [f32; 3]| {
        let b = p.len() as u32;
        for (i, v) in c.iter().enumerate() {
            p.push(*v);
            n.push(normal);
            uv.push([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i]);
        }
        idx.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    };
    // The long walls (x = +-0.5) and the end walls (z = +-0.5).
    quad([[0.5, 0.0, 0.5], [0.5, 0.0, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]], [1.0, 0.0, 0.0]);
    quad([[-0.5, 0.0, -0.5], [-0.5, 0.0, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]], [-1.0, 0.0, 0.0]);
    quad([[-0.5, 0.0, 0.5], [0.5, 0.0, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]], [0.0, 0.0, 1.0]);
    quad([[0.5, 0.0, -0.5], [-0.5, 0.0, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]], [0.0, 0.0, -1.0]);
    // The roof's two slopes, from the eaves (0.6 across, 0.45 up) to the ridge (0, 1).
    let (rx, ry) = (0.6f32, 1.0 - 0.45f32);
    let l = (rx * rx + ry * ry).sqrt();
    quad([[0.6, 0.45, 0.55], [0.6, 0.45, -0.55], [0.0, 1.0, -0.55], [0.0, 1.0, 0.55]], [ry / l, rx / l, 0.0]);
    quad([[-0.6, 0.45, -0.55], [-0.6, 0.45, 0.55], [0.0, 1.0, 0.55], [0.0, 1.0, -0.55]], [-ry / l, rx / l, 0.0]);
    // The gables (end walls' triangles up to the ridge).
    let b = p.len() as u32;
    for (z, nz) in [(0.5f32, 1.0f32), (-0.5, -1.0)] {
        let tri = if nz > 0.0 { [[-0.5, 0.5, z], [0.5, 0.5, z], [0.0, 0.95, z]] } else { [[0.5, 0.5, z], [-0.5, 0.5, z], [0.0, 0.95, z]] };
        for v in tri {
            p.push(v);
            n.push([0.0, 0.0, nz]);
            uv.push([0.0, 0.0]);
        }
    }
    idx.extend([b, b + 1, b + 2, b + 3, b + 4, b + 5]);
    (p, n, uv, idx)
}

/// A section of a level 3 tower: from height `y[0]` to `y[1]` (unit), its half-widths scaled from
/// `s[0]` to `s[1]` and turned from `a[0]` to `a[1]` (rad), in `rings` steps.
#[derive(Clone, Copy)]
struct Section {
    y: [f32; 2],
    s: [f32; 2],
    a: [f32; 2],
    rings: u32,
}

/// Adds the triangle (a, b, c) facing `out` (its winding counter-clockwise seen from that side, as
/// the building pipeline culls), with its normal and the uv's flag (see `tower`).
fn tri_facing(m: &mut Mesh4, v: [[f32; 3]; 3], uvx: [f32; 3], flag: f32, out: Vec3) {
    let [a, b, c] = v.map(Vec3::from);
    let mut nrm = (b - a).cross(c - a);
    if nrm.length_squared() < 1e-12 {
        return;
    }
    nrm = nrm.normalize();
    let order = if nrm.dot(out) < 0.0 {
        nrm = -nrm;
        [0, 2, 1]
    } else {
        [0, 1, 2]
    };
    let base = m.0.len() as u32;
    for i in order {
        m.0.push(v[i]);
        m.1.push(nrm.to_array());
        m.2.push([uvx[i], flag]);
    }
    m.3.extend([base, base + 1, base + 2]);
}

/// A level 3 tower in unit size (x and z -0.5..0.5 at the base, y 0..1): `sides` walls (4 square,
/// more round) through `sections` (each from where the last ended; a narrower one leaves a terrace),
/// the roof, and a spire to the top (its base's half-width) when the last section ends below 1. The
/// uv tells the shader what a face is: x the point across its face (footprint units: a wall's own
/// width; round ones along the perimeter), y 0 a wall scaled by the width, 1 by the depth, 2 the
/// spire, 3 a roof or terrace.
fn tower(sections: &[Section], sides: u32, spire: f32, floor: bool) -> Mesh4 {
    let mut m: Mesh4 = Default::default();
    let ring = |s: f32, a: f32, j: u32, y: f32| -> [f32; 3] {
        if sides == 4 {
            let ang = a + std::f32::consts::FRAC_PI_4 + j as f32 * std::f32::consts::FRAC_PI_2;
            let r = 0.5 * s * std::f32::consts::SQRT_2;
            [r * ang.cos(), y, r * ang.sin()]
        } else {
            let ang = a + j as f32 / sides as f32 * std::f32::consts::TAU;
            [0.5 * s * ang.cos(), y, 0.5 * s * ang.sin()]
        }
    };
    // Across a wall from its corner j to j + 1: a square's from -s/2 to s/2, a round one's along its
    // perimeter.
    let across = |s: f32, j: u32, end: bool| -> f32 {
        if sides == 4 {
            if end { 0.5 * s } else { -0.5 * s }
        } else {
            (j + end as u32) as f32 / sides as f32 * std::f32::consts::PI * s
        }
    };
    let up = Vec3::Y;
    // The floor (a skybridge's underside), facing down.
    if let (true, Some(s0)) = (floor, sections.first()) {
        for j in 0..sides {
            tri_facing(&mut m, [[0.0, s0.y[0], 0.0], ring(s0.s[0], s0.a[0], j, s0.y[0]), ring(s0.s[0], s0.a[0], j + 1, s0.y[0])], [0.0; 3], 3.0, -up);
        }
    }
    let mut prev: Option<(f32, f32, f32)> = None;
    for sec in sections {
        // A terrace where this section starts narrower than the last ended.
        if let Some((py, ps, pa)) = prev {
            if ps > sec.s[0] + 1e-4 {
                for j in 0..sides {
                    let (o0, o1) = (ring(ps, pa, j, py), ring(ps, pa, j + 1, py));
                    let (i0, i1) = (ring(sec.s[0], sec.a[0], j, py), ring(sec.s[0], sec.a[0], j + 1, py));
                    tri_facing(&mut m, [o0, o1, i1], [0.0; 3], 3.0, up);
                    tri_facing(&mut m, [o0, i1, i0], [0.0; 3], 3.0, up);
                }
            }
        }
        for r in 0..sec.rings {
            let t0 = r as f32 / sec.rings as f32;
            let t1 = (r + 1) as f32 / sec.rings as f32;
            let lerp = |v: [f32; 2], t: f32| v[0] + (v[1] - v[0]) * t;
            let (y0, y1) = (lerp(sec.y, t0), lerp(sec.y, t1));
            let (s0, s1) = (lerp(sec.s, t0), lerp(sec.s, t1));
            let (a0, a1) = (lerp(sec.a, t0), lerp(sec.a, t1));
            for j in 0..sides {
                let (pb, qb) = (ring(s0, a0, j, y0), ring(s0, a0, j + 1, y0));
                let (pt, qt) = (ring(s1, a1, j, y1), ring(s1, a1, j + 1, y1));
                let mid = (Vec3::from(pb) + Vec3::from(qb) + Vec3::from(pt) + Vec3::from(qt)) * 0.25;
                let out = Vec3::new(mid.x, 0.0, mid.z);
                let (ub0, ub1) = (across(s0, j, false), across(s0, j, true));
                let (ut0, ut1) = (across(s1, j, false), across(s1, j, true));
                let flag = if sides == 4 { (j % 2) as f32 } else { 0.0 };
                tri_facing(&mut m, [pb, qb, qt], [ub0, ub1, ut1], flag, out);
                tri_facing(&mut m, [pb, qt, pt], [ub0, ut1, ut0], flag, out);
            }
        }
        prev = Some((sec.y[1], sec.s[1], sec.a[1]));
    }
    // The roof.
    if let Some((y, s, a)) = prev {
        for j in 0..sides {
            tri_facing(&mut m, [[0.0, y, 0.0], ring(s, a, j, y), ring(s, a, j + 1, y)], [0.0; 3], 3.0, up);
        }
        // The spire: four faces up to the top.
        if y < 0.999 && spire > 0.0 {
            for j in 0..4 {
                let a = std::f32::consts::FRAC_PI_4 + j as f32 * std::f32::consts::FRAC_PI_2;
                let b = a + std::f32::consts::FRAC_PI_2;
                let r = spire * std::f32::consts::SQRT_2;
                let (p, q) = ([r * a.cos(), y, r * a.sin()], [r * b.cos(), y, r * b.sin()]);
                let out = Vec3::new(p[0] + q[0], 0.0, p[2] + q[2]);
                tri_facing(&mut m, [p, q, [0.0, 1.0, 0.0]], [0.0; 3], 2.0, out);
            }
        }
    }
    m
}

/// A hero model (`assets/models/hero/NAME.bin`, written by the port's Blender script): its triangles
/// in a unit box (x and z -0.5..0.5, y 0..1; the record's size scales it to its real one), their
/// normals, and uv in metres with the texture layer in v's ten-thousands. Header: `SEHM`, version 1,
/// the real width, height and depth (m), the vertex and index counts.
fn hero_model(name: &str) -> Option<Mesh4> {
    hero_file(name).map(|(m, _)| m)
}

/// A hero model's file (`hero_model`) and its real width, height and depth (m).
pub(crate) fn hero_file(name: &str) -> Option<(Mesh4, [f32; 3])> {
    let rel = std::path::Path::new("models").join("hero").join(format!("{name}.bin"));
    let mut path = crate::config::asset_dir().join(&rel);
    if !path.is_file() {
        path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join(&rel);
    }
    sehm_from_bytes(&std::fs::read(&path).ok()?)
}

/// A `SEHM` file's mesh (`hero_file`) from its bytes: also the Viltrumite mod's props (`viltrumite/impact.rs`).
pub(crate) fn sehm_from_bytes(b: &[u8]) -> Option<(Mesh4, [f32; 3])> {
    let u = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
    if b.get(0..4) != Some(b"SEHM".as_slice()) || u(4)? != 1 {
        return None;
    }
    let (nv, ni) = (u(20)? as usize, u(24)? as usize);
    if b.len() < 28 + nv * 32 + ni * 4 {
        return None;
    }
    let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let mut m: Mesh4 = Default::default();
    for i in 0..nv {
        let o = 28 + i * 32;
        m.0.push([f(o), f(o + 4), f(o + 8)]);
        m.1.push([f(o + 12), f(o + 16), f(o + 20)]);
        m.2.push([f(o + 24), f(o + 28)]);
    }
    let o = 28 + nv * 32;
    m.3 = (0..ni).map(|i| u32::from_le_bytes(b[o + i * 4..o + i * 4 + 4].try_into().unwrap())).collect();
    Some((m, [f(8), f(12), f(16)]))
}

/// A colony's dome in unit size: a half ellipsoid over x and z -0.5..0.5 up to y 1 (the instance
/// scales it to its radius and height), uv x around (0..1), y down from the top (0..1).
fn dome_mesh() -> Mesh4 {
    const AROUND: u32 = 48;
    const DOWN: u32 = 12;
    let mut m: Mesh4 = Default::default();
    for j in 0..=DOWN {
        let t = j as f32 / DOWN as f32 * std::f32::consts::FRAC_PI_2;
        for i in 0..=AROUND {
            let a = i as f32 / AROUND as f32 * std::f32::consts::TAU;
            let (x, y, z) = (0.5 * t.sin() * a.cos(), t.cos(), 0.5 * t.sin() * a.sin());
            m.0.push([x, y, z]);
            // The unit-space normal of the half ellipsoid (semi-axes 0.5, 1, 0.5: along 4x, y, 4z; the
            // shader takes it through the stretch).
            m.1.push(Vec3::new(x * 4.0, y, z * 4.0).normalize().to_array());
            m.2.push([i as f32 / AROUND as f32, j as f32 / DOWN as f32]);
        }
    }
    let row = AROUND + 1;
    for j in 0..DOWN {
        for i in 0..AROUND {
            let (a, b, c, d) = (j * row + i, j * row + i + 1, (j + 1) * row + i + 1, (j + 1) * row + i);
            // Counter-clockwise seen from outside.
            for t in [[a, c, b], [a, d, c]] {
                let [p0, p1, p2] = t.map(|k| Vec3::from(m.0[k as usize]));
                let mid = (p0 + p1 + p2) / 3.0;
                if (p1 - p0).cross(p2 - p0).dot(mid) >= 0.0 { m.3.extend(t) } else { m.3.extend([t[0], t[2], t[1]]) }
            }
        }
    }
    m
}

/// The flying lanes' segment: two ribbons (z -0.5 and 0.5: the instance's depth sets them 12 m apart)
/// along x -0.5..0.5, each a vertical and a flat strip of no width that the shader widens to a car or a
/// pixel (uv x the side, y 0 the vertical strip, 1 the flat one), both sides.
fn lane_mesh() -> Mesh4 {
    let mut m: Mesh4 = Default::default();
    for z in [-0.5f32, 0.5] {
        for part in [0.0f32, 1.0] {
            for back in [false, true] {
                let base = m.0.len() as u32;
                let s = if back { -1.0 } else { 1.0 };
                // (Widened along z the flat strip's winding faces down: its front is the underside.)
                let normal = if part == 0.0 { [0.0, 0.0, s] } else { [0.0, -s, 0.0] };
                for (x, side) in [(-0.5f32, -1.0f32), (0.5, -1.0), (0.5, 1.0), (-0.5, 1.0)] {
                    m.0.push([x, 0.0, z]);
                    m.1.push(normal);
                    m.2.push([side, part]);
                }
                if back {
                    m.3.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
                } else {
                    m.3.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }
    }
    m
}

/// The buildings' box: x and z -0.5..0.5, y 0..1 (the instance scales it to its footprint and
/// height), its four walls and roof (no floor); uv in metres-to-be: x across the face, y up.
fn building_box() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut quad = |corners: [[f32; 3]; 4], normal: [f32; 3]| {
        let b = p.len() as u32;
        for (i, c) in corners.iter().enumerate() {
            p.push(*c);
            n.push(normal);
            uv.push([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i]);
        }
        idx.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    };
    // Walls (counter-clockwise seen from outside), then the roof.
    quad([[-0.5, 0.0, 0.5], [0.5, 0.0, 0.5], [0.5, 1.0, 0.5], [-0.5, 1.0, 0.5]], [0.0, 0.0, 1.0]);
    quad([[0.5, 0.0, -0.5], [-0.5, 0.0, -0.5], [-0.5, 1.0, -0.5], [0.5, 1.0, -0.5]], [0.0, 0.0, -1.0]);
    quad([[0.5, 0.0, 0.5], [0.5, 0.0, -0.5], [0.5, 1.0, -0.5], [0.5, 1.0, 0.5]], [1.0, 0.0, 0.0]);
    quad([[-0.5, 0.0, -0.5], [-0.5, 0.0, 0.5], [-0.5, 1.0, 0.5], [-0.5, 1.0, -0.5]], [-1.0, 0.0, 0.0]);
    quad([[-0.5, 1.0, 0.5], [0.5, 1.0, 0.5], [0.5, 1.0, -0.5], [-0.5, 1.0, -0.5]], [0.0, 1.0, 0.0]);
    (p, n, uv, idx)
}

/// Megaton Rainfall's grass: its `billboard_grass` material (the 2x2 tuft atlas `Billboard_Grass`,
/// tint 0.45 0.50 0.29), one tuft per atlas cell as its `bunch_N` models (made by its code, not
/// stored: crossed quads); here three quads crossed at 60 degrees, 0.66 m high and wide (MR's tuft).
pub const GRASS_TUFTS: u32 = 4;
const GRASS_TEXTURE: &str = "Billboard_Grass";
const GRASS_TINT: [f32; 4] = [0.45, 0.50, 0.29, 1.0];
const GRASS_SIZE: f32 = 0.66;

/// Megaton Rainfall's 16 trees (temperate broadleaf; "Realtime 3D Trees XL", the pack's LOD1 meshes).
const TREES: [&str; 16] = [
    "tree_0", "tree_1", "tree_2", "tree_3", "tree_4", "tree_5", "tree_6", "tree_7", "tree_8", "tree_9", "tree_10", "tree_11", "tree_12",
    "tree_13", "tree_14", "tree_15",
];

/// A model's mesh and sub-meshes (material indices into the library's materials).
#[derive(Clone)]
pub struct ModelSrc {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// (first index, index count, material).
    pub subs: Vec<(u32, u32, u32)>,
    /// Mesh and impostor bucket caps.
    pub caps: (u32, u32),
    /// The crown (centre height, radius) for the leaves' ball normals; `None`: from the leaves.
    pub crown: Option<(f32, f32)>,
    /// A broadleaf tree: its leaves turn and fall with the seasons (MR's trees, the species with
    /// branch cards; not the conifers, palms, grass or buildings).
    pub deciduous: bool,
    /// A street object: its kind (`STREET_*`, 0 none) and its numbers for the shader (`ModelGpu::mean`).
    pub street: (u32, [f32; 4]),
}

/// The main world's copy of what the library was built from (for the flora's rebuilds).
#[derive(Resource)]
struct LibrarySources {
    models: Vec<ModelSrc>,
    /// Per material: the decoded texture the bake samples.
    decoded: Vec<impostor::Texture>,
    /// Material indices: MR's barks, its branch cards, the conifer and palm cards.
    bark_mats: Vec<u32>,
    branch_mats: Vec<u32>,
    conifer_mat: u32,
    palm_mat: u32,
    /// The whole atlas as baked (the flora's layers are replaced in it).
    baked: Option<Arc<Baked>>,
}

/// The impostor bake running on a worker thread, with the atlas images made there too (the whole
/// library, or a new flora's layers spliced into the stored atlas), and for a flora its key, its
/// models and the library assembled with it.
#[derive(Resource)]
struct BakeTask {
    task: Task<(Baked, Image, Image)>,
    flora: Option<(u64, Vec<ModelSrc>, Assembled)>,
}

/// The packed library.
#[derive(Clone)]
struct Assembled {
    vertices: Vec<LibVertex>,
    indices: Vec<u32>,
    models: Vec<ModelGpu>,
    caps: Vec<u32>,
    draws: Vec<DrawEntry>,
}

/// Packs `models` and lists what the bake needs (per model, in order).
fn assemble(models: &[ModelSrc], materials: &[LibMaterial], decoded: &[impostor::Texture]) -> (Assembled, Vec<BakeModel>) {
    let mut a = Assembled { vertices: Vec::new(), indices: vec![0, 1, 2, 2, 1, 3], models: Vec::new(), caps: Vec::new(), draws: Vec::new() };
    let mut bake = Vec::new();
    for (m, src) in models.iter().enumerate() {
        let (mesh_bucket, impostor_bucket) = (a.caps.len() as u32, a.caps.len() as u32 + 1);
        a.caps.extend([src.caps.0, src.caps.1]);
        let base_vertex = a.vertices.len() as i32;
        for i in 0..src.positions.len() {
            a.vertices.push(LibVertex {
                pos: src.positions[i],
                normal: src.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                uv: src.uvs.get(i).copied().unwrap_or([0.0; 2]),
            });
        }
        let mut subs = Vec::new();
        let mut leaf_lo = f32::MAX;
        let (mut leaf_hi, mut leaf_r) = (f32::MIN, 0.0f32);
        for &(first, count, mat) in &src.subs {
            let material = &materials[mat as usize];
            let first_index = a.indices.len() as u32;
            // The indices are absolute within the model: the draw's base vertex is the model's first.
            a.indices.extend(&src.indices[first as usize..(first + count) as usize]);
            let kind = if material.building { DrawKind::Building { material: mat } } else { DrawKind::Mesh { material: mat, leaves: material.leaves } };
            a.draws.push(DrawEntry { bucket: mesh_bucket, first_index, index_count: count, base_vertex, kind });
            subs.push(BakeSub { first_index: first, index_count: count, texture: decoded[mat as usize].clone(), tint: material.tint, leaves: material.leaves });
            if material.leaves {
                for &i in &src.indices[first as usize..(first + count) as usize] {
                    let p = src.positions[i as usize];
                    leaf_lo = leaf_lo.min(p[1]);
                    leaf_hi = leaf_hi.max(p[1]);
                    leaf_r = leaf_r.max((p[0] * p[0] + p[2] * p[2]).sqrt());
                }
            }
        }
        // (A model without impostors takes no draw and no layers of the atlas: buildings, street objects.)
        let has_impostor = src.caps.1 > 0;
        if has_impostor {
            a.draws.push(DrawEntry { bucket: impostor_bucket, first_index: 0, index_count: 6, base_vertex: 0, kind: DrawKind::Impostor });
        }
        let (lo, hi) = src.positions.iter().fold((f32::MAX, f32::MIN), |acc, p| (acc.0.min(p[1]), acc.1.max(p[1])));
        let height = hi - lo.min(0.0);
        let horiz = src.positions.iter().map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt()).fold(0.0f32, f32::max);
        // The crown: the leaves' vertices' bounds.
        let (cy, cr) = src.crown.unwrap_or(if leaf_hi > leaf_lo {
            ((leaf_lo + leaf_hi) * 0.5, ((leaf_hi - leaf_lo) * 0.5).max(leaf_r))
        } else {
            (height * 0.6, horiz)
        });
        let half = (horiz.max(height * 0.5)) * 1.04;
        let centre_y = height * 0.5;
        a.models.push(ModelGpu {
            half,
            centre_y,
            radius: (horiz * horiz + (height * 0.5).powi(2)).sqrt(),
            layer0: m as u32 * impostor::VIEWS,
            crown_y: cy,
            crown_r: cr,
            mesh_bucket,
            impostor_bucket,
            mean: if src.street.0 != 0 { src.street.1 } else { [0.25, 0.25, 0.25, 0.5] },
            flags: [src.deciduous as u32, src.street.0, 0, 0],
        });
        if has_impostor {
            bake.push(BakeModel {
                positions: src.positions.clone(),
                normals: src.normals.clone(),
                uvs: src.uvs.clone(),
                indices: src.indices.clone(),
                subs,
                half,
                centre_y,
                crown_y: cy,
                crown_r: cr,
            });
        }
    }
    (a, bake)
}

/// A generated species as a model.
fn species_model(r: &Recipe, src: &LibrarySources) -> ModelSrc {
    let g = trees::grow(r);
    let mut indices = g.bark.clone();
    let nb = indices.len() as u32;
    indices.extend(&g.leaves);
    let leaf_mat = match r.leaf {
        Leaf::Branch(i) => src.branch_mats[i as usize % src.branch_mats.len()],
        Leaf::Conifer => src.conifer_mat,
        Leaf::Palm => src.palm_mat,
    };
    let bark_mat = src.bark_mats[r.bark as usize % src.bark_mats.len()];
    ModelSrc {
        positions: g.positions,
        normals: g.normals,
        uvs: g.uvs,
        indices,
        subs: vec![(0, nb, bark_mat), (nb, g.leaves.len() as u32, leaf_mat)],
        caps: (SPECIES_MESH_CAP, SPECIES_IMPOSTOR_CAP),
        crown: None,
        deciduous: matches!(r.leaf, Leaf::Branch(_)),
        street: (0, [0.0; 4]),
    }
}

/// A model that draws nothing, keeping the others' places when a file is missing.
fn no_model(cap: u32) -> ModelSrc {
    ModelSrc { positions: vec![[0.0; 3]], normals: vec![[0.0, 1.0, 0.0]], uvs: vec![[0.0; 2]], indices: Vec::new(), subs: Vec::new(), caps: (cap.min(1), 0), deciduous: false, street: (STREET_PLAIN, [0.0; 4]), crown: Some((0.5, 0.0)) }
}

/// The street objects (`STREET_PROPS`, `STREET_CARS`). The furniture shares one material: a palette of
/// its colours, each vertex's uv on its own (so they add one texture, not one each). The cars are turned
/// nose to +x and centred on the ground (Megaton Rainfall's face -x, off centre), their sprites left out
/// (the shader draws their lights), with a dark patch under them.
fn street_models(install: &MrInstall, mr: &mut MrAssets, images: &mut Assets<Image>, materials: &mut Vec<LibMaterial>, decoded_all: &mut Vec<impostor::Texture>) -> Vec<ModelSrc> {
    let mut out = Vec::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut palette_of = |c: [u8; 4]| -> usize {
        palette.iter().position(|p| *p == c).unwrap_or_else(|| {
            palette.push(c);
            palette.len() - 1
        })
    };
    let palette_mat = materials.len() as u32;
    materials.push(LibMaterial { color: Handle::default(), tint: [1.0; 4], leaves: false, building: false });
    decoded_all.push(impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![128, 128, 128, 255]) });
    let mut entries: Vec<Vec<usize>> = Vec::new();
    for (name, cap, kind) in STREET_PROPS {
        let model = match install.model(name) {
            Ok(m) => m,
            Err(e) => {
                warn!("scatter: {e}");
                out.push(no_model(cap));
                entries.push(Vec::new());
                continue;
            }
        };
        let mut ids = vec![0; model.positions.len()];
        let mut subs = Vec::new();
        for s in model.submeshes.iter().filter(|s| s.index_count > 0) {
            let mat = install.resolve(&s.material);
            // The material's colour where its effect is `Constant`, else its vertices' (BGRA): the game
            // passes both on raw, so they are the same kind of value.
            let d = mat.diffuse();
            let raw = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let constant = [raw(d[0]), raw(d[1]), raw(d[2]), 255];
            for v in s.first_vertex as usize..(s.first_vertex + s.vertex_count) as usize {
                let c = match model.colors.get(v) {
                    Some(c) if mat.effect == "Vertex" => [c[2], c[1], c[0], 255],
                    _ => constant,
                };
                if let Some(id) = ids.get_mut(v) {
                    *id = palette_of(c);
                }
            }
            subs.push((s.first_index, s.index_count, palette_mat));
        }
        let top = model.positions.iter().fold(0.0f32, |m, p| m.max(p[1]));
        // A triangle of mixed colours takes its first corner's, on vertices of its own: its uv would
        // sweep across the palette through every colour between.
        let (mut positions, mut normals, mut indices) = (model.positions.clone(), model.normals.clone(), model.indices.clone());
        for &(first, count, _) in &subs {
            for tri in indices[first as usize..(first + count) as usize].chunks_exact_mut(3) {
                let c = ids[tri[0] as usize];
                if ids[tri[1] as usize] != c || ids[tri[2] as usize] != c {
                    for v in tri.iter_mut() {
                        let (p, n) = (positions[*v as usize], normals.get(*v as usize).copied().unwrap_or([0.0, 1.0, 0.0]));
                        *v = positions.len() as u32;
                        positions.push(p);
                        normals.push(n);
                        ids.push(c);
                    }
                }
            }
        }
        entries.push(ids);
        out.push(ModelSrc {
            positions,
            normals,
            uvs: Vec::new(),
            indices,
            subs,
            caps: (cap, 0),
            deciduous: false,
            street: (kind, [0.0; 4]),
            crown: Some((top * 0.5, 0.0)),
        });
    }
    let width = palette.len().max(1).next_power_of_two();
    for (m, ids) in out.iter_mut().zip(&entries) {
        m.uvs = ids.iter().map(|&i| [(i as f32 + 0.5) / width as f32, 0.5]).collect();
    }
    let mut data = Vec::with_capacity(width * 4);
    for i in 0..width {
        data.extend(palette.get(i).copied().unwrap_or([128, 128, 128, 255]));
    }
    materials[palette_mat as usize].color = images.add(Image::new(
        Extent3d { width: width as u32, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    for name in STREET_CARS {
        let car = (|| -> Option<ModelSrc> {
            let model = install.model(name).map_err(|e| warn!("scatter: {e}")).ok()?;
            let s = model.submeshes.iter().find(|s| s.index_count > 0)?;
            let mat = install.resolve(&s.material);
            let tex = mat.textures.first()?;
            let color = mr.image(images, tex, true)?;
            let material = materials.len() as u32;
            materials.push(LibMaterial { color, tint: [1.0; 4], leaves: false, building: false });
            decoded_all.push(impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![128, 128, 128, 255]) });
            let range = s.first_index as usize..(s.first_index + s.index_count) as usize;
            let src = &model.indices[range];
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            let mut map = vec![u32::MAX; model.positions.len()];
            let (mut positions, mut normals, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::with_capacity(src.len() + 6));
            for &i in src {
                let p = model.positions[i as usize];
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            let (cx, cz) = ((lo[0] + hi[0]) * 0.5, (lo[2] + hi[2]) * 0.5);
            for &i in src {
                let v = i as usize;
                if map[v] == u32::MAX {
                    map[v] = positions.len() as u32;
                    let p = model.positions[v];
                    let n = model.normals.get(v).copied().unwrap_or([0.0, 1.0, 0.0]);
                    positions.push([cx - p[0], p[1] - lo[1], cz - p[2]]);
                    normals.push([-n[0], n[1], -n[2]]);
                    uvs.push(model.uvs.get(v).copied().unwrap_or([0.0; 2]));
                }
                indices.push(map[v]);
            }
            let (hl, hw) = ((hi[0] - lo[0]) * 0.5, (hi[2] - lo[2]) * 0.5);
            let v0 = positions.len() as u32;
            for (x, z) in [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                positions.push([x * (hl + 0.1), 0.03, z * (hw + 0.1)]);
                normals.push([0.0, 1.0, 0.0]);
                uvs.push([0.0, 0.0]);
            }
            // Both windings: the patch shows whichever way the pipeline culls.
            indices.extend([v0, v0 + 1, v0 + 2, v0, v0 + 2, v0 + 3, v0, v0 + 2, v0 + 1, v0, v0 + 3, v0 + 2]);
            let painted = mat.textures.get(1).is_some_and(|t| t == "vehicles_colors");
            let n = indices.len() as u32;
            let height = hi[1] - lo[1];
            Some(ModelSrc {
                positions,
                normals,
                uvs,
                indices,
                subs: vec![(0, n, material)],
                caps: (CAR_CAP, 0),
                deciduous: false,
                street: (if painted { STREET_CAR } else { STREET_CAR_OWN }, [hl, 0.7 - lo[1], 0.8, hw]),
                crown: Some((height * 0.5, 0.0)),
            })
        })();
        out.push(car.unwrap_or_else(|| no_model(CAR_CAP)));
    }
    out
}

fn start_library(mut commands: Commands, mut mr: ResMut<MrAssets>, mut images: ResMut<Assets<Image>>, mut lib: ResMut<ScatterLibrary>) {
    let Some(install) = mr.install.clone() else { return };
    let mut materials: Vec<LibMaterial> = Vec::new();
    let mut decoded_all: Vec<impostor::Texture> = Vec::new();
    let mut models: Vec<ModelSrc> = Vec::new();
    // MR's trees.
    for name in TREES {
        let model = match install.model(name) {
            Ok(m) => m,
            Err(e) => {
                warn!("scatter: {e}");
                continue;
            }
        };
        let mut subs = Vec::new();
        for s in &model.submeshes {
            let mat = install.resolve(&s.material);
            let leaves = mat.effect.contains("leaves") || mat.effect.contains("alpha_test");
            let Some(tex) = mat.textures.first() else { continue };
            let Some(color) = mr.image(&mut images, tex, true) else { continue };
            let material = materials.len() as u32;
            materials.push(LibMaterial { color, tint: mat.diffuse(), leaves, building: false });
            decoded_all.push(decoded(&install, tex));
            subs.push((s.first_index, s.index_count, material));
        }
        models.push(ModelSrc {
            positions: model.positions.clone(),
            normals: model.normals.clone(),
            uvs: model.uvs.clone(),
            indices: model.indices.clone(),
            subs,
            caps: (MESH_CAP, IMPOSTOR_CAP),
            crown: None,
            deciduous: true,
            street: (0, [0.0; 4]),
        });
    }
    if models.is_empty() {
        return;
    }
    // The generated species' materials.
    let mut mat = |images: &mut Assets<Image>, mr: &mut MrAssets, name: &str, leaves: bool| -> Option<u32> {
        let color = mr.image(images, name, true)?;
        materials.push(LibMaterial { color, tint: [1.0; 4], leaves, building: false });
        decoded_all.push(decoded(&install, name));
        Some(materials.len() as u32 - 1)
    };
    let mut bark_mats: Vec<u32> = trees::BARKS.iter().filter_map(|b| mat(&mut images, &mut mr, b, false)).collect();
    let mut branch_mats: Vec<u32> = trees::BRANCHES.iter().filter_map(|b| mat(&mut images, &mut mr, b, true)).collect();
    // Without them, the generated species wear MR's trees' own bark and leaves.
    if bark_mats.is_empty() || branch_mats.is_empty() {
        warn!("scatter: Megaton Rainfall's bark or branch textures are missing: the generated trees wear its trees' own");
    }
    if bark_mats.is_empty() {
        bark_mats.extend(materials.iter().position(|m| !m.leaves).map(|i| i as u32));
    }
    if branch_mats.is_empty() {
        branch_mats.extend(materials.iter().position(|m| m.leaves).map(|i| i as u32));
    }
    let mut card = |name: &str| -> u32 {
        let (image, tex) = own_card(name);
        materials.push(LibMaterial { color: images.add(image), tint: [1.0; 4], leaves: true, building: false });
        decoded_all.push(tex);
        materials.len() as u32 - 1
    };
    let (conifer_mat, palm_mat) = (card("conifer_branch"), card("palm_frond"));
    if bark_mats.is_empty() || branch_mats.is_empty() {
        return;
    }
    let mut src = LibrarySources { models: Vec::new(), decoded: Vec::new(), bark_mats, branch_mats, conifer_mat, palm_mat, baked: None };
    let earth0 = models.len() as u32;
    for r in trees::earth_species() {
        models.push(species_model(&r, &src));
    }
    let flora0 = models.len() as u32;
    let key = flora_key(0, 9.81, 1.0);
    for r in trees::flora(0, 9.81, 1.0) {
        models.push(species_model(&r, &src));
    }
    let trees_n = models.len() as u32;
    // The grass tufts.
    let mut grass = 0;
    if let Some(color) = mr.image(&mut images, GRASS_TEXTURE, true) {
        let material = materials.len() as u32;
        materials.push(LibMaterial { color, tint: GRASS_TINT, leaves: true, building: false });
        decoded_all.push(decoded(&install, GRASS_TEXTURE));
        for t in 0..GRASS_TUFTS {
            let (positions, normals, uvs, idx) = tuft(t);
            let n = idx.len() as u32;
            // The tuft shades as the ground under it: its "crown" far below makes the leaves' ball
            // normals point up (scatter_draw.wesl).
            models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, material)], caps: (GRASS_CAP, 64), deciduous: false, street: (0, [0.0; 4]), crown: Some((-50.0, 50.0)) });
            grass += 1;
        }
    }
    // The buildings' box: one instance a building (cities, the plan's step 7), sized and turned by
    // its record; no impostor.
    let building = models.len() as u32;
    {
        let material = materials.len() as u32;
        // MR's window array (18 facade tiles: the windows in colour, their mask in alpha); without
        // it a blank array (walls alone: the shader's own windows).
        let windows = mr.image(&mut images, "windows", true).unwrap_or_else(|| {
            let mut blank = Image::new_fill(
                bevy::render::render_resource::Extent3d { width: 1, height: 1, depth_or_array_layers: 2 },
                bevy::render::render_resource::TextureDimension::D2,
                &[0, 0, 0, 0],
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::RENDER_WORLD,
            );
            blank.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
                dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
                ..default()
            });
            images.add(blank)
        });
        materials.push(LibMaterial { color: windows, tint: [1.0; 4], leaves: false, building: true });
        decoded_all.push(impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![255; 4]) });
        let (positions, normals, uvs, idx) = building_box();
        let n = idx.len() as u32;
        models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, material)], caps: (BUILDING_CAP, 0), deciduous: false, street: (0, [0.0; 4]), crown: Some((0.5, 0.0)) });
        // The villages' round hut and longhouse (level 1), the box's next two models, with their
        // own material: the port's thatch, mud brick, timber and stone (made with the image
        // generator) in an array.
        let village = materials.len() as u32;
        materials.push(LibMaterial { color: images.add(village_array()), tint: [1.0; 4], leaves: false, building: true });
        decoded_all.push(impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![160, 130, 90, 255]) });
        for (positions, normals, uvs, idx) in [round_hut(), longhouse()] {
            let n = idx.len() as u32;
            models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, village)], caps: (VILLAGE_CAP, 0), deciduous: false, street: (0, [0.0; 4]), crown: Some((0.5, 0.0)) });
        }
        // Level 3's cities, the box's next eight models (city.wesl M_FUTURE..M_LANE): the future
        // box, four towers (tapering, stepped, twisted, round), the arcology, the skybridge and the
        // flying lanes, with level 3's facades and signs.
        let future = materials.len() as u32;
        materials.push(LibMaterial { color: images.add(future_array()), tint: [1.0; 4], leaves: false, building: true });
        decoded_all.push(impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![40, 50, 60, 255]) });
        let sec = |y: [f32; 2], s: [f32; 2], a: [f32; 2], rings: u32| Section { y, s, a, rings };
        let quarter = std::f32::consts::FRAC_PI_2;
        let arco: Vec<Section> = (0..5).map(|i| {
            let (y0, s0) = (i as f32 * 0.184, 1.0 - 0.18 * i as f32);
            sec([y0, y0 + 0.184], [s0, s0 - 0.04], [0.0; 2], 1)
        }).collect();
        let shapes: [(Mesh4, u32); 8] = [
            (tower(&[sec([0.0, 1.0], [1.0; 2], [0.0; 2], 1)], 4, 0.0, false), BUILDING_CAP),
            (tower(&[sec([0.0, 0.9], [1.0, 0.55], [0.0; 2], 3)], 4, 0.06, false), TOWER_CAP),
            (tower(&[sec([0.0, 0.42], [1.0; 2], [0.0; 2], 1), sec([0.42, 0.7], [0.78; 2], [0.0; 2], 1), sec([0.7, 0.88], [0.56; 2], [0.0; 2], 1)], 4, 0.05, false), TOWER_CAP),
            (tower(&[sec([0.0, 0.95], [1.0, 0.8], [0.0, quarter], 16)], 4, 0.04, false), TOWER_CAP),
            (tower(&[sec([0.0, 0.84], [1.0, 0.82], [0.0; 2], 3), sec([0.84, 0.95], [0.82, 0.45], [0.0; 2], 2)], 20, 0.03, false), TOWER_CAP),
            (tower(&arco, 4, 0.03, false), 400),
            (tower(&[sec([0.0, 1.0], [1.0; 2], [0.0; 2], 1)], 4, 0.0, true), TOWER_CAP),
            (lane_mesh(), LANE_CAP),
        ];
        for ((positions, normals, uvs, idx), cap) in shapes {
            let n = idx.len() as u32;
            models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, future)], caps: (cap, 0), deciduous: false, street: (0, [0.0; 4]), crown: Some((0.5, 0.0)) });
        }
        // The hero models (city.wesl M_TEMPLE, M_WATCH, M_LANDMARK), made in Blender by script
        // (`assets/models/hero`): level 1's temple and watchtower with the villages' textures, level
        // 3's landmark tower with its facades. A missing one draws as a box.
        for (name, material, cap) in [("temple", village, 2000), ("watchtower", village, 2000), ("landmark", future, 200)] {
            let (positions, normals, uvs, idx) = hero_model(name).unwrap_or_else(|| {
                warn!("scatter: hero model {name} missing: a box instead");
                building_box()
            });
            let n = idx.len() as u32;
            models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, material)], caps: (cap, 0), deciduous: false, street: (0, [0.0; 4]), crown: Some((0.5, 0.0)) });
        }
        // A level 4 civilization's colonies' domes (city.wesl M_DOME), with level 3's material.
        let (positions, normals, uvs, idx) = dome_mesh();
        let n = idx.len() as u32;
        models.push(ModelSrc { positions, normals, uvs, indices: idx, subs: vec![(0, n, future)], caps: (4000, 0), deciduous: false, street: (0, [0.0; 4]), crown: Some((0.5, 0.0)) });
        models.extend(street_models(&install, &mut mr, &mut images, &mut materials, &mut decoded_all));
    }
    let (a, bake) = assemble(&models, &materials, &decoded_all);
    info!("scatter: library of {} models ({} generated), {} vertices, {} draws; baking impostors", a.models.len(), trees_n - earth0, a.vertices.len(), a.draws.len());
    *lib = ScatterLibrary {
        generation: 0,
        trees: trees_n,
        grass,
        earth0,
        flora0,
        flora_key: key,
        building,
        vertices: Arc::new(a.vertices),
        indices: Arc::new(a.indices),
        materials,
        models: a.models,
        bucket_caps: a.caps,
        draws: a.draws,
        impostor_color: None,
        impostor_normal: None,
    };
    src.models = models;
    src.decoded = decoded_all;
    commands.insert_resource(src);
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let b = impostor::bake(&bake);
        let (c, n) = impostor::images(&b);
        (b, c, n)
    });
    commands.insert_resource(BakeTask { task, flora: None });
}

/// Makes the flora the scatter wants: its 8 species grown, the library assembled with them, and
/// their impostors baked on a worker thread (the library switches to it when they are done).
fn update_flora(
    mut commands: Commands,
    scatter: Res<super::Scatter>,
    lib: Res<ScatterLibrary>,
    src: Option<Res<LibrarySources>>,
    task: Option<Res<BakeTask>>,
) {
    let (Some(src), Some(want)) = (src, scatter.flora_want) else { return };
    if task.is_some() || src.baked.is_none() || want.key == lib.flora_key {
        return;
    }
    let mut models = src.models.clone();
    let f0 = lib.flora0 as usize;
    for (i, r) in trees::flora(want.seed, want.gravity, want.pressure).iter().enumerate() {
        models[f0 + i] = species_model(r, &src);
    }
    let (a, mut bake) = assemble(&models, &lib.materials, &src.decoded);
    let flora_bake: Vec<BakeModel> = bake.drain(f0..f0 + trees::SPECIES as usize).collect();
    info!("scatter: a flora for the body (gravity {:.1} m/s^2, air {:.2} atm): baking its impostors", want.gravity, want.pressure);
    // The flora's layers and mean colours spliced into a copy of the whole atlas, and its images
    // made, on the worker (a copy of about 150 MB).
    let Some(old) = src.baked.clone() else { return };
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let fresh = impostor::bake(&flora_bake);
        let mut b = Baked { layers: old.layers, color: old.color.clone(), normal: old.normal.clone(), means: old.means.clone() };
        let l0 = f0 * impostor::VIEWS as usize;
        for (i, (c, n)) in fresh.color.into_iter().zip(fresh.normal).enumerate() {
            b.color[l0 + i] = c;
            b.normal[l0 + i] = n;
        }
        for (i, m) in fresh.means.into_iter().enumerate() {
            b.means[f0 + i] = m;
        }
        let (c, n) = impostor::images(&b);
        (b, c, n)
    });
    commands.insert_resource(BakeTask { task, flora: Some((want.key, models, a)) });
}

fn finish_bake(
    mut commands: Commands,
    task: Option<ResMut<BakeTask>>,
    src: Option<ResMut<LibrarySources>>,
    scatter: Res<super::Scatter>,
    mut lib: ResMut<ScatterLibrary>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(mut task), Some(mut src)) = (task, src) else { return };
    let Some((baked, color, normal)) = check_ready(&mut task.task) else { return };
    let flora = task.flora.take();
    commands.remove_resource::<BakeTask>();
    if let Some((key, models, a)) = flora {
        // The camera came back to a body whose flora the library holds already: keep it.
        if scatter.flora_want.is_some_and(|w| w.key == lib.flora_key && w.key != key) {
            return;
        }
        lib.vertices = Arc::new(a.vertices);
        lib.indices = Arc::new(a.indices);
        lib.models = a.models;
        lib.bucket_caps = a.caps;
        lib.draws = a.draws;
        lib.flora_key = key;
        src.models = models;
    }
    for (m, mean) in lib.models.iter_mut().zip(&baked.means) {
        m.mean = *mean;
    }
    lib.impostor_color = Some(images.add(color));
    lib.impostor_normal = Some(images.add(normal));
    lib.generation += 1;
    info!("scatter: impostors baked ({} layers of {} px)", baked.layers, impostor::SIZE);
    src.baked = Some(Arc::new(baked));
}

/// Grass tuft `t`: three quads crossed at 60 degrees about the up axis, textured with atlas cell `t`
/// (positions, normals, uvs, indices). The normals point up: the tuft is lit as the ground.
#[allow(clippy::type_complexity)]
fn tuft(t: u32) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    let (u0, v0) = ((t % 2) as f32 * 0.5, (t / 2) as f32 * 0.5);
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let h = GRASS_SIZE * 0.5;
    for q in 0..3 {
        let a = q as f32 * std::f32::consts::PI / 3.0;
        let (dx, dz) = (a.cos() * h, a.sin() * h);
        let b = p.len() as u32;
        for (x, y, z, s, tt) in [(-dx, 0.0, -dz, 0.0, 1.0), (dx, 0.0, dz, 1.0, 1.0), (-dx, GRASS_SIZE, -dz, 0.0, 0.0), (dx, GRASS_SIZE, dz, 1.0, 0.0)] {
            p.push([x, y, z]);
            n.push([0.0, 1.0, 0.0]);
            uv.push([u0 + s * 0.5, v0 + tt * 0.5]);
        }
        idx.extend([b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
    (p, n, uv, idx)
}

/// A texture decoded to RGBA8 at its top level (the bake samples it), or a grey square.
fn decoded(mr: &Arc<MrInstall>, name: &str) -> impostor::Texture {
    if let Ok((bytes, info)) = mr.texture(name) {
        if let Ok(rgba) = decode_level(&bytes, &info, 0, 0) {
            return impostor::Texture { width: info.width, height: info.height, rgba: Arc::new(rgba) };
        }
    }
    impostor::Texture { width: 1, height: 1, rgba: Arc::new(vec![128, 128, 128, 255]) }
}

/// One of the port's own leaf cards (`assets/textures/vegetation/NAME.png`, made with the image
/// generator: see the vegetation plan): a GPU image with its mips (coverage-kept alpha, as the
/// impostors') and the decoded top level for the bake. A grey square if the file is missing.
fn own_card(name: &str) -> (Image, impostor::Texture) {
    // (Beside the exe's own assets first; a copy of the exe with only its shaders beside it finds
    // the cards in the source tree.)
    let rel = std::path::Path::new("textures").join("vegetation").join(format!("{name}.png"));
    let mut path = crate::config::asset_dir().join(&rel);
    if !path.is_file() {
        path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join(&rel);
    }
    let rgba = std::fs::read(&path).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8());
    let (w, h, px) = match rgba {
        Some(i) if i.width() == i.height() && i.width().is_power_of_two() => (i.width(), i.height(), i.into_raw()),
        _ => {
            warn!("scatter: leaf card {} missing or not a power-of-two square", path.display());
            (1, 1, vec![128, 128, 128, 255])
        }
    };
    let top: Vec<[f32; 4]> = px.chunks_exact(4).map(|p| [impostor::srgb_to_linear(p[0]), impostor::srgb_to_linear(p[1]), impostor::srgb_to_linear(p[2]), p[3] as f32 / 255.0]).collect();
    let levels = impostor::mips(top, w as usize);
    let mut data = Vec::new();
    for l in &levels {
        data.extend(l.iter().flat_map(|p| [impostor::linear_to_srgb(p[0]), impostor::linear_to_srgb(p[1]), impostor::linear_to_srgb(p[2]), (p[3] * 255.0).round() as u8]));
    }
    let mut image = Image::new_uninit(Extent3d { width: w, height: h, depth_or_array_layers: 1 }, TextureDimension::D2, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels.len() as u32;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    (image, impostor::Texture { width: w, height: h, rgba: Arc::new(px) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level 3's towers: every wall faces away from the axis and every roof up, with its normal on
    /// the side its winding faces (the building pipeline culls back faces: a reversed triangle would
    /// leave a hole).
    #[test]
    fn towers_face_out() {
        let sec = |y: [f32; 2], s: [f32; 2], a: [f32; 2], rings: u32| Section { y, s, a, rings };
        let meshes = [
            tower(&[sec([0.0, 0.9], [1.0, 0.55], [0.0; 2], 3)], 4, 0.06, false),
            tower(&[sec([0.0, 0.42], [1.0; 2], [0.0; 2], 1), sec([0.42, 0.7], [0.78; 2], [0.0; 2], 1), sec([0.7, 0.88], [0.56; 2], [0.0; 2], 1)], 4, 0.05, false),
            tower(&[sec([0.0, 0.95], [1.0, 0.8], [0.0, std::f32::consts::FRAC_PI_2], 16)], 4, 0.04, false),
            tower(&[sec([0.0, 0.84], [1.0, 0.82], [0.0; 2], 3), sec([0.84, 0.95], [0.82, 0.45], [0.0; 2], 2)], 20, 0.03, false),
            tower(&[sec([0.0, 1.0], [1.0; 2], [0.0; 2], 1)], 4, 0.0, true),
        ];
        for (k, m) in meshes.iter().enumerate() {
            assert!(!m.3.is_empty());
            for t in m.3.chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.0[i as usize]));
                let n = (b - a).cross(c - a);
                assert!(n.dot(Vec3::from(m.1[t[0] as usize])) > 0.0, "model {k}: normal against the winding");
                let mid = (a + b + c) / 3.0;
                let floor = k == 4 && mid.y == 0.0 && n.y < 0.0;
                assert!(n.dot(Vec3::new(mid.x, 0.0, mid.z)) > 0.0 || n.y > 0.0 || floor, "model {k}: a face turned inwards at {mid}");
            }
        }
    }

    /// Copies the Megaton Rainfall files the port uses from the user's install (`MR_PATH`, else
    /// `se_core::mr::DEFAULT_MR_PATH`) into `assets/mr` (the user's choice of 2026-11-05: the port
    /// must work without the game; the copy is for private use, not for sharing the port): its 16
    /// trees, every texture their materials name, the generated species' barks and branch cards, the
    /// grass and the window array, and `Data/global.txt`, as the game's own bytes. Run
    /// `cargo test -p se_engine --release copy_megaton_rainfall -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn copy_megaton_rainfall() {
        use std::path::PathBuf;
        let root = std::env::var("MR_PATH").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(se_core::mr::DEFAULT_MR_PATH));
        let mr = se_core::mr::MrInstall::open(&root).expect("Megaton Rainfall's install");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mr");
        let mut files = std::collections::BTreeSet::new();
        files.insert("Data/global.txt".to_string());
        let mut textures = Vec::new();
        for name in TREES {
            files.insert(format!("Models/{name}.bin"));
            let model = mr.model(name).expect("tree model");
            for s in &model.submeshes {
                textures.extend(mr.resolve(&s.material).textures.iter().cloned());
            }
        }
        for name in STREET_PROPS.iter().map(|p| p.0).chain(STREET_CARS) {
            files.insert(format!("Models/{name}.bin"));
            let model = mr.model(name).expect("street model");
            for s in model.submeshes.iter().filter(|s| s.index_count > 0) {
                // (A car's second texture, its paint's palette, is the shader's own: `ct_paint`.)
                textures.extend(mr.resolve(&s.material).textures.first().cloned());
            }
        }
        textures.extend(trees::BARKS.iter().chain(trees::BRANCHES.iter()).map(|t| t.to_string()));
        textures.extend([GRASS_TEXTURE.to_string(), "windows".to_string()]);
        for t in &textures {
            if mr.has_texture(t) {
                files.insert(se_core::mr::texture_path(t));
            } else {
                println!("not in the game's files: {t}");
            }
        }
        let mut total = 0;
        for f in &files {
            let bytes = mr.vfs().read(f).expect("file");
            let path = out.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            total += bytes.len();
        }
        println!("{} files, {:.1} MB, in {}", files.len(), total as f64 / 1e6, out.display());
        // The copy opens as the install does, with every file the library reads.
        let copy = se_core::mr::MrInstall::open_copy(&out).expect("the copy");
        assert!(TREES.iter().all(|t| copy.model(t).is_ok()));
        assert!(STREET_PROPS.iter().map(|p| p.0).chain(STREET_CARS).all(|t| copy.model(t).is_ok()));
        assert!(textures.iter().filter(|t| mr.has_texture(t)).all(|t| copy.has_texture(t)));
    }

    /// The hero models (made in Blender) load, inside the unit box.
    #[test]
    fn hero_models_load() {
        for name in ["temple", "watchtower", "landmark"] {
            let m = hero_model(name).unwrap_or_else(|| panic!("{name} missing"));
            assert!(!m.3.is_empty() && m.0.len() == m.1.len() && m.0.len() == m.2.len());
            for p in &m.0 {
                assert!(p[0].abs() <= 0.5001 && p[2].abs() <= 0.5001 && (-1e-4..=1.0001).contains(&p[1]), "{name}: {p:?}");
            }
        }
    }

    /// The Viltrumite mod's question Q6 (step 9): can each building model be solid as 8 boxes stacked by height (F12.8)?
    /// Per slice: its box from the mesh's points inside it (the triangles clipped to the slice), and how much of that box
    /// the mesh's cross-section fills, at four heights in it (the cut's segments, by their winding: closed walls give the
    /// enclosed area). In the unit box, so the shares hold at any size. Run
    /// `cargo test -p se_engine --release print_building_slices -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_building_slices() {
        let sec = |y: [f32; 2], s: [f32; 2], a: [f32; 2], rings: u32| Section { y, s, a, rings };
        let quarter = std::f32::consts::FRAC_PI_2;
        let arco: Vec<Section> = (0..5)
            .map(|i| {
                let (y0, s0) = (i as f32 * 0.184, 1.0 - 0.18 * i as f32);
                sec([y0, y0 + 0.184], [s0, s0 - 0.04], [0.0; 2], 1)
            })
            .collect();
        let mut models: Vec<(&str, Mesh4)> = vec![
            ("0 box", building_box()),
            ("1 hut", round_hut()),
            ("2 longhouse", longhouse()),
            ("3 future box", tower(&[sec([0.0, 1.0], [1.0; 2], [0.0; 2], 1)], 4, 0.0, false)),
            ("4 tapering", tower(&[sec([0.0, 0.9], [1.0, 0.55], [0.0; 2], 3)], 4, 0.06, false)),
            ("5 stepped", tower(&[sec([0.0, 0.42], [1.0; 2], [0.0; 2], 1), sec([0.42, 0.7], [0.78; 2], [0.0; 2], 1), sec([0.7, 0.88], [0.56; 2], [0.0; 2], 1)], 4, 0.05, false)),
            ("6 twisted", tower(&[sec([0.0, 0.95], [1.0, 0.8], [0.0, quarter], 16)], 4, 0.04, false)),
            ("7 round", tower(&[sec([0.0, 0.84], [1.0, 0.82], [0.0; 2], 3), sec([0.84, 0.95], [0.82, 0.45], [0.0; 2], 2)], 20, 0.03, false)),
            ("8 arcology", tower(&arco, 4, 0.03, false)),
            ("9 skybridge", tower(&[sec([0.0, 1.0], [1.0; 2], [0.0; 2], 1)], 4, 0.0, true)),
        ];
        for (n, name) in [("11 temple", "temple"), ("12 watchtower", "watchtower"), ("13 landmark", "landmark")] {
            models.push((n, hero_model(name).unwrap_or_else(|| panic!("{name} missing"))));
        }
        models.push(("14 dome", dome_mesh()));
        const SLICES: usize = 8;
        for (name, m) in &models {
            let tris: Vec<[Vec3; 3]> = m.3.chunks(3).map(|t| [0, 1, 2].map(|k| Vec3::from(m.0[t[k] as usize]))).collect();
            let (ylo, yhi) = m.0.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
            let mut cells = Vec::new();
            let mut worst: f32 = 1.0;
            for s in 0..SLICES {
                let (y0, y1) = (ylo + (yhi - ylo) * s as f32 / SLICES as f32, ylo + (yhi - ylo) * (s + 1) as f32 / SLICES as f32);
                // The box: points of the triangles inside the band, and where their edges cross its two planes.
                let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                for t in &tris {
                    for k in 0..3 {
                        let (a, b) = (t[k], t[(k + 1) % 3]);
                        if (y0..=y1).contains(&a.y) {
                            lo = lo.min(a.xz());
                            hi = hi.max(a.xz());
                        }
                        for y in [y0, y1] {
                            if (a.y - y) * (b.y - y) < 0.0 {
                                let p = a.lerp(b, (y - a.y) / (b.y - a.y));
                                lo = lo.min(p.xz());
                                hi = hi.max(p.xz());
                            }
                        }
                    }
                }
                if lo.x > hi.x {
                    cells.push("      -      ".to_string());
                    continue;
                }
                let area = (hi - lo).x * (hi - lo).y;
                // The cross-section's area at four heights: each triangle's cut segment, oriented by its normal.
                let mut fill = 0.0;
                for q in 0..4 {
                    let y = y0 + (y1 - y0) * (q as f32 + 0.5) / 4.0;
                    let mut a2 = 0.0;
                    for t in &tris {
                        let mut pts = Vec::new();
                        for k in 0..3 {
                            let (a, b) = (t[k], t[(k + 1) % 3]);
                            if (a.y - y) * (b.y - y) < 0.0 {
                                pts.push(a.lerp(b, (y - a.y) / (b.y - a.y)).xz());
                            }
                        }
                        if pts.len() == 2 {
                            let n = (t[1] - t[0]).cross(t[2] - t[0]).xz();
                            let (mut p, mut r) = (pts[0], pts[1]);
                            // Outward normal on the right of p -> r (counter-clockwise loops seen from +y in x, z... as
                            // the cross product below takes it).
                            let d = r - p;
                            if Vec2::new(d.y, -d.x).dot(n) < 0.0 {
                                std::mem::swap(&mut p, &mut r);
                            }
                            a2 += p.x * r.y - r.x * p.y;
                        }
                    }
                    fill += (0.5 * a2).abs() / area.max(1e-9) / 4.0;
                }
                worst = worst.min(fill);
                cells.push(format!("{:>4.2}x{:<4.2} {:>3.0}%", (hi - lo).x, (hi - lo).y, fill * 100.0));
            }
            println!("{name:14} slices (width x depth of the unit box, fill): {}  worst {:.0}%", cells.join(" | "), worst * 100.0);
        }
    }
}
