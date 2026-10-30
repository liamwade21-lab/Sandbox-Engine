//! Offscreen views of the scene from the camera's position: the machinery shared by SE's
//! high-resolution screenshot tiles (seScreenshotTool, `screenshot.rs`), its skybox export
//! (seExportSkyboxDialog, here) and the cube faces of its fish-eye / cylindrical / cubic projections
//! (`projection.rs`). Each view is one more `Camera3d` rendering into its own image (never a pass on
//! the main camera's textures), turned by a fixed rotation from the main camera and copying its near
//! plane, layers and, for finished images, its exposure, bloom and colour correction. Views exist only
//! while something uses them, so they cost nothing otherwise.
//!
//! Cube faces use the DDS / Direct3D cubemap convention in the camera's frame (+X right, +Y up, +Z
//! ahead), so each face is an upright picture: +X looks right, +Y up (its top edge behind the camera),
//! +Z ahead. SE's skybox export (FUN_1402b49c0) writes the faces as `sky_<face>.<ext>` (`pos_x`,
//! `neg_x`, `neg_y`, `pos_y`, `pos_z`, `neg_z`) into `export/`, or all six into `sky_cubemap.dds` for
//! its "dds cubemap" format; the port writes into `<Pictures>/SpaceEngine-rs/export/`.

use std::sync::{Arc, Mutex};

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::{ColorGrading, Msaa};
use bevy::render::RenderApp;
use bevy::transform::TransformSystems;

use crate::bloom::SeBloom;
use crate::camera::{CommandQueue, FlyCamera, MainCamera};
use crate::color_correction::ColorCorrection;
use crate::screenshot::{Depth, Picture};
use crate::universe::UniverseSet;

/// The format of every view's image (the main camera's HDR format).
pub const FORMAT: TextureFormat = TextureFormat::Rgba16Float;

/// What a view runs after the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Post {
    /// SE's bloom (`SeBloom`, copied from the main camera; no pseudo lens flare).
    pub bloom: bool,
    /// The exposure, SE's tone shader and colour correction (the finished, LDR image).
    pub correct: bool,
}

impl Post {
    /// The raw HDR scene (for the projections, which post-process the projected frame).
    pub const HDR: Post = Post { bloom: false, correct: false };
}

/// A view of the scene from the main camera, `rel` turned from it. Extracted so the auto exposure
/// does not meter these views.
#[derive(Component, ExtractComponent, Clone, Copy, Debug)]
#[extract_app(RenderApp)]
pub struct CaptureView {
    /// Rotation from the main camera's frame (camera space: x right, y up, -z ahead).
    pub rel: Quat,
    /// Vertical field of view (rad); `None` follows the main camera.
    pub fov: Option<f32>,
    pub post: Post,
}

/// While `projection.rs` hides the scene from the main camera, the layers it showed (what the views
/// render).
#[derive(Resource, Default)]
pub struct SceneLayers(pub Option<RenderLayers>);

/// The six cube faces in SE's / D3D's order (+X, -X, +Y, -Y, +Z, -Z of the camera frame above), as
/// (forward, up) in camera space.
pub const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::NEG_Z, Vec3::Y),
    (Vec3::Z, Vec3::Y),
];

/// SE's face file names (FUN_1402b49c0's table), for [`FACES`] in order.
pub const FACE_NAMES: [&str; 6] = ["pos_x", "neg_x", "pos_y", "neg_y", "pos_z", "neg_z"];

/// The rotation of a face camera from the main camera.
pub fn face_rotation(face: usize) -> Quat {
    let (f, u) = FACES[face];
    Transform::IDENTITY.looking_to(f, u).rotation
}

/// The face a camera-space direction falls on and its (s, t) in 0..1 (s right, t down).
pub fn dir_to_face(d: Vec3) -> (usize, Vec2) {
    // The cube frame: x right, y up, z ahead.
    let e = Vec3::new(d.x, d.y, -d.z);
    let a = e.abs();
    let (face, sc, tc, ma) = if a.x >= a.y && a.x >= a.z {
        if e.x > 0.0 { (0, -e.z, -e.y, a.x) } else { (1, e.z, -e.y, a.x) }
    } else if a.y >= a.z {
        if e.y > 0.0 { (2, e.x, e.z, a.y) } else { (3, e.x, -e.z, a.y) }
    } else if e.z > 0.0 {
        (4, e.x, -e.y, a.z)
    } else {
        (5, -e.x, -e.y, a.z)
    };
    let ma = ma.max(1e-30);
    (face, Vec2::new((sc / ma + 1.0) * 0.5, (tc / ma + 1.0) * 0.5))
}

/// SE's cylindrical (360 x 180 degree) map, `proj_cylinder.glsl`: longitude `(u * 2 - 0.5) pi`, latitude
/// `(0.5 - v) pi`, its cube coordinate `(cos lat sin lon, sin lat, cos lat cos lon)` in SE's projection
/// frame (x ahead, y up, z left): the image centre looks ahead. `uv` from the top left, 0..1.
pub fn cylinder_dir(uv: Vec2) -> Vec3 {
    let lon = (uv.x * 2.0 - 0.5) * std::f32::consts::PI;
    let lat = (0.5 - uv.y) * std::f32::consts::PI;
    let (sx, cx) = lon.sin_cos();
    let (sy, cy) = lat.sin_cos();
    // ahead = -z, up = y, left = -x in camera space.
    Vec3::new(-cy * cx, sy, -cy * sx)
}

/// Spawn a view rendering into a new `size` image; returns it and its image.
pub fn spawn_view(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    size: UVec2,
    view: CaptureView,
    order: isize,
    transparent: bool,
) -> (Entity, Handle<Image>) {
    let mut image = Image::new_target_texture(size.x.max(1), size.y.max(1), FORMAT, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image.data = None;
    image.copy_on_resize = false;
    let handle = images.add(image);
    let tonemapping = if view.post.correct { Tonemapping::Linear } else { Tonemapping::None };
    let clear = if transparent { Color::NONE } else { Color::BLACK };
    let e = commands
        .spawn((
            Camera3d::default(),
            Camera { order, clear_color: ClearColorConfig::Custom(clear), ..default() },
            RenderTarget::Image(handle.clone().into()),
            Hdr,
            tonemapping,
            DebandDither::Disabled,
            Msaa::Off,
            Projection::Perspective(PerspectiveProjection {
                fov: view.fov.unwrap_or(1.0),
                aspect_ratio: size.x.max(1) as f32 / size.y.max(1) as f32,
                near: 0.05,
                ..default()
            }),
            Transform::IDENTITY,
            view,
        ))
        .id();
    (e, handle)
}

/// Despawn views and drop their images.
pub fn despawn_views(commands: &mut Commands, images: &mut Assets<Image>, views: &[(Entity, Handle<Image>)]) {
    for (e, h) in views {
        commands.entity(*e).despawn();
        images.remove(h);
    }
}

/// Turn the views with the main camera and give them its near plane, layers and post-processing.
#[allow(clippy::type_complexity)]
fn follow_main(
    mut commands: Commands,
    scene: Res<SceneLayers>,
    main: Query<
        (&Transform, &Projection, Option<&RenderLayers>, &ColorGrading, Option<&ColorCorrection>, Option<&SeBloom>),
        (With<MainCamera>, Without<CaptureView>),
    >,
    mut views: Query<
        (Entity, &CaptureView, &mut Transform, &mut Projection, Option<&RenderLayers>, Option<&ColorCorrection>, Option<&SeBloom>),
        Without<MainCamera>,
    >,
) {
    if views.is_empty() {
        return;
    }
    let Ok((tf, proj, layers, grading, cc, bloom)) = main.single() else { return };
    let Projection::Perspective(mp) = proj else { return };
    let layers = scene.0.clone().or_else(|| layers.cloned()).unwrap_or_default();
    for (e, v, mut vtf, mut vproj, vl, vcc, vbloom) in &mut views {
        let rot = tf.rotation * v.rel;
        if vtf.rotation != rot || vtf.translation != Vec3::ZERO {
            vtf.rotation = rot;
            vtf.translation = Vec3::ZERO;
        }
        if let Projection::Perspective(p) = vproj.as_mut() {
            let fov = v.fov.unwrap_or(mp.fov);
            if p.fov != fov || p.near != mp.near {
                p.fov = fov;
                p.near = mp.near;
            }
        }
        let mut ec = commands.entity(e);
        if vl != Some(&layers) {
            ec.insert(layers.clone());
        }
        if v.post.correct {
            ec.insert(grading.clone());
            if let Some(c) = cc
                && vcc.map(|x| x.bcsg != c.bcsg || x.mode != c.mode).unwrap_or(true)
            {
                ec.insert(*c);
            }
        }
        match bloom.filter(|_| v.post.bloom) {
            Some(b) => {
                // The pseudo lens flare is placed from the main camera's view: off here.
                let mut b = *b;
                b.flare_bright.w = 0.0;
                if vbloom != Some(&b) {
                    ec.insert(b);
                }
            }
            None if vbloom.is_some() => {
                ec.remove::<SeBloom>();
            }
            None => {}
        }
    }
}

/// Readbacks of view images, filled in by their observers.
#[derive(Clone, Default)]
pub struct Grabs(Arc<Mutex<Vec<Option<Vec<u8>>>>>);

impl Grabs {
    pub fn new(n: usize) -> Self {
        Self(Arc::new(Mutex::new(vec![None; n])))
    }

    /// Read `image` back into slot `i` (this frame's render).
    pub fn request(&self, commands: &mut Commands, i: usize, image: Handle<Image>) {
        let store = self.0.clone();
        commands.spawn(ReadbackOnce::texture(image)).observe(move |ev: On<ReadbackComplete>, mut commands: Commands| {
            if let Ok(mut s) = store.lock()
                && let Some(slot) = s.get_mut(i)
            {
                *slot = Some(ev.data.clone());
            }
            commands.entity(ev.entity).despawn();
        });
    }

    pub fn take(&self, i: usize) -> Option<Vec<u8>> {
        self.0.lock().ok().and_then(|mut s| s.get_mut(i).and_then(Option::take))
    }

    pub fn all_ready(&self) -> bool {
        self.0.lock().map(|s| s.iter().all(Option::is_some)).unwrap_or(false)
    }
}

/// IEEE half to f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match exp {
        0 if man == 0 => sign,
        0 => {
            // Subnormal: normalise.
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        31 => sign | 0x7f80_0000 | (man << 13),
        _ => sign | ((exp + 127 - 15) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

/// A read-back [`FORMAT`] image as RGBA f32, rows unpadded (wgpu pads them to 256 bytes).
pub fn decode_rgba16f(data: &[u8], w: u32, h: u32) -> Vec<[f32; 4]> {
    let row = w as usize * 8;
    let stride = if h > 1 { row.div_ceil(256) * 256 } else { row };
    let mut out = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        let Some(r) = data.get(y * stride..y * stride + row) else {
            out.resize(w as usize * h as usize, [0.0; 4]);
            break;
        };
        for p in r.chunks_exact(8) {
            let c = |i: usize| f16_to_f32(u16::from_le_bytes([p[i], p[i + 1]]));
            out.push([c(0), c(2), c(4), c(6)]);
        }
    }
    out
}

/// The sRGB transfer (the views' values are linear, as the swapchain's).
pub fn srgb_encode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c > 0.003_130_8 { 1.055 * c.powf(1.0 / 2.4) - 0.055 } else { c * 12.92 }
}

/// Linear RGBA pixels as a picture of `depth`: integer depths are sRGB-encoded (as the screen shows
/// them), float keeps the linear values.
pub fn to_picture(w: u32, h: u32, px: &[[f32; 4]], alpha: bool, depth: Depth) -> Picture {
    let n = if alpha { 4 } else { 3 };
    let enc = |p: &[f32; 4], i: usize| if i == 3 { p[3].clamp(0.0, 1.0) } else { srgb_encode(p[i]) };
    let samples = match depth {
        Depth::U8 => crate::screenshot::Samples::U8(px.iter().flat_map(|p| (0..n).map(move |i| (enc(p, i) * 255.0 + 0.5) as u8)).collect()),
        Depth::U16 => crate::screenshot::Samples::U16(px.iter().flat_map(|p| (0..n).map(move |i| (enc(p, i) * 65535.0 + 0.5) as u16)).collect()),
        Depth::F32 => crate::screenshot::Samples::F32(px.iter().flat_map(|p| (0..n).map(move |i| if i == 3 { p[3] } else { p[i].max(0.0) })).collect()),
    };
    Picture { w, h, alpha, samples }
}

/// Bilinear sample of a face (clamped at its edges).
fn sample_face(face: &[[f32; 4]], res: u32, st: Vec2) -> [f32; 4] {
    let n = res as f32;
    let x = (st.x * n - 0.5).clamp(0.0, n - 1.0);
    let y = (st.y * n - 0.5).clamp(0.0, n - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(res as usize - 1), (y0 + 1).min(res as usize - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |xx: usize, yy: usize| face[yy * res as usize + xx];
    let mut out = [0.0; 4];
    for (i, o) in out.iter_mut().enumerate() {
        let top = at(x0, y0)[i] * (1.0 - fx) + at(x1, y0)[i] * fx;
        let bot = at(x0, y1)[i] * (1.0 - fx) + at(x1, y1)[i] * fx;
        *o = top * (1.0 - fy) + bot * fy;
    }
    out
}

/// The six faces resampled into SE's cylindrical panorama of `w` x `h`.
pub fn cylinder_from_faces(faces: &[Vec<[f32; 4]>; 6], res: u32, w: u32, h: u32) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            let uv = Vec2::new((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let (f, st) = dir_to_face(cylinder_dir(uv));
            out.push(sample_face(&faces[f], res, st));
        }
    }
    out
}

/// SE's terrain loading still running (its "Wait loading completeness").
pub fn terrain_loading(terrains: &Query<&crate::terrain::Terrain>) -> usize {
    terrains.iter().map(|t| t.loading_nodes()).sum()
}

// ---------------------------------------------------------------------------
// Skybox export (seExportSkyboxDialog)

/// SE's skybox formats (main-def.cfg `Export` `SkyboxFormat`; FUN_1402b49c0's "dds cubemap" writes one
/// cubemap DDS).
pub const SKYBOX_FORMATS: [&str; 6] = ["tga", "jpg", "png", "dds", "tif", "dds cubemap"];

/// SE's export dialog: "Cubemap" / "Cylindrical", "Resolution", "Image format", "Pixel format"
/// (8-bit, 16-bit, 32-bit (floating-point)), "Post-process", "Color correction (no HDR!)", "Save
/// transparency".
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct SkyboxSettings {
    /// `SkyboxFormat`, one of [`SKYBOX_FORMATS`].
    pub format: String,
    pub cylindrical: bool,
    /// The face size (cubemap) or the panorama height (cylindrical, twice as wide).
    pub resolution: u32,
    pub depth: Depth,
    /// SE's bloom on the faces (off by default: each face blooms on its own, so the face edges show).
    pub post_process: bool,
    /// The exposure, tone shader and colour correction: an LDR image.
    pub color_correction: bool,
    pub save_alpha: bool,
}

impl Default for SkyboxSettings {
    fn default() -> Self {
        Self {
            format: "tga".into(),
            cylindrical: false,
            resolution: 1024,
            depth: Depth::U8,
            post_process: false,
            color_correction: true,
            save_alpha: false,
        }
    }
}

/// Largest face (the read-back faces of a panorama are kept in memory).
pub const MAX_SKYBOX_RES: u32 = 8192;

/// A requested export (the dialog's Export button or the `skybox` dev command).
#[derive(Resource, Default)]
pub struct SkyboxRequest(pub bool);

struct SkyboxJob {
    settings: SkyboxSettings,
    res: u32,
    views: Vec<(Entity, Handle<Image>)>,
    grabs: Grabs,
    frames: u32,
    requested: bool,
}

#[derive(Resource, Default)]
struct SkyboxState(Option<SkyboxJob>);

/// The export folder, `<Pictures>/SpaceEngine-rs/export`.
pub fn export_folder() -> std::path::PathBuf {
    crate::screenshot::folder().join("export")
}

/// The face size rendered for an export.
pub fn skybox_face_res(s: &SkyboxSettings) -> u32 {
    let r = if s.cylindrical { s.resolution.div_ceil(2) } else { s.resolution };
    r.clamp(16, MAX_SKYBOX_RES)
}

/// Frames a view renders before it is read: the bloom chain and temporal state settle.
pub const SETTLE_FRAMES: u32 = 4;
/// Longest wait for the terrain (frames), then the capture goes on anyway.
pub const LOAD_TIMEOUT: u32 = 1800;

#[allow(clippy::too_many_arguments)]
fn run_skybox(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut req: ResMut<SkyboxRequest>,
    mut state: ResMut<SkyboxState>,
    settings: Res<SkyboxSettings>,
    main: Query<Option<&SeBloom>, With<MainCamera>>,
    terrains: Query<&crate::terrain::Terrain>,
    (mut msg, i18n): (ResMut<crate::messages::Messages>, Res<crate::i18n::I18n>),
) {
    if std::mem::take(&mut req.0) && state.0.is_none() {
        let s = settings.clone();
        let res = skybox_face_res(&s);
        let bloom = s.post_process && main.iter().next().flatten().is_some();
        let post = Post { bloom, correct: s.color_correction };
        let views = (0..6)
            .map(|i| {
                let v = CaptureView { rel: face_rotation(i), fov: Some(std::f32::consts::FRAC_PI_2), post };
                spawn_view(&mut commands, &mut images, UVec2::splat(res), v, -70 - i as isize, s.save_alpha)
            })
            .collect();
        info!("exporting the skybox: {} faces of {res} px, {}", if s.cylindrical { "cylindrical from" } else { "6" }, s.format);
        msg.show(i18n.tr("Generating"));
        state.0 = Some(SkyboxJob { settings: s, res, views, grabs: Grabs::new(6), frames: 0, requested: false });
        return;
    }
    let Some(job) = state.0.as_mut() else { return };
    job.frames += 1;
    if !job.requested {
        let loading = terrain_loading(&terrains) > 0 && job.frames < LOAD_TIMEOUT;
        if job.frames >= SETTLE_FRAMES && !loading {
            for (i, (_, h)) in job.views.iter().enumerate() {
                job.grabs.request(&mut commands, i, h.clone());
            }
            job.requested = true;
        }
        return;
    }
    if !job.grabs.all_ready() {
        return;
    }
    let job = state.0.take().expect("job");
    let faces: Vec<Vec<u8>> = (0..6).map(|i| job.grabs.take(i).unwrap_or_default()).collect();
    despawn_views(&mut commands, &mut images, &job.views);
    msg.show(i18n.tr("Saving files"));
    let (s, res) = (job.settings, job.res);
    std::thread::spawn(move || {
        if let Err(e) = save_skybox(&s, res, &faces) {
            warn!("cannot save the skybox: {e}");
        }
    });
}

/// Decode the read-back faces and write them as `s` says.
fn save_skybox(s: &SkyboxSettings, res: u32, faces: &[Vec<u8>]) -> std::io::Result<()> {
    let dir = export_folder();
    std::fs::create_dir_all(&dir)?;
    let decoded: [Vec<[f32; 4]>; 6] = std::array::from_fn(|i| decode_rgba16f(&faces[i], res, res));
    let (ext, cube_dds) = if s.format == "dds cubemap" { ("dds", true) } else { (s.format.as_str(), false) };
    let depth = crate::screenshot::best_depth(ext, s.depth);
    let alpha = s.save_alpha && ext != "jpg";
    if s.cylindrical {
        let (w, h) = (s.resolution.max(2) * 2, s.resolution.max(1));
        let px = cylinder_from_faces(&decoded, res, w, h);
        let path = dir.join(format!("sky_cylindrical.{ext}"));
        crate::screenshot::encode(&path, ext, 95, &to_picture(w, h, &px, alpha, depth))?;
        info!("skybox -> {}", path.display());
    } else if cube_dds {
        let pics: Vec<Picture> = decoded.iter().map(|f| to_picture(res, res, f, true, depth)).collect();
        let path = dir.join("sky_cubemap.dds");
        std::fs::write(&path, crate::screenshot::dds_cube(&pics))?;
        info!("skybox -> {}", path.display());
    } else {
        for (f, name) in decoded.iter().zip(FACE_NAMES) {
            let path = dir.join(format!("sky_{name}.{ext}"));
            crate::screenshot::encode(&path, ext, 95, &to_picture(res, res, f, alpha, depth))?;
        }
        info!("skybox -> {}/sky_*.{ext}", dir.display());
    }
    Ok(())
}

fn load_skybox_format(cfg: Res<crate::config::EngineConfig>, mut s: ResMut<SkyboxSettings>) {
    let text = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).unwrap_or_default();
    let def = se_core::sc::parse(&text);
    if let Some(f) = def.find_deep("SkyboxFormat").and_then(|i| i.str()).map(str::to_ascii_lowercase).filter(|f| SKYBOX_FORMATS.contains(&f.as_str())) {
        s.format = f;
    }
}

// ---------------------------------------------------------------------------
// Dev commands

/// The capture dev commands, taken off the `--run` queue before `camera.rs` sees them:
/// `projection MODE` (0 perspective, 1 cubic, 2 fish eye, 3 cylinder), `cubeformat N`,
/// `fisheye FOV [ZOOM]`, `hires W H [TILE]` (the HD screenshot tool), `skybox RES [cyl] [FORMAT]
/// [8|16|32]`.
#[allow(clippy::too_many_arguments)]
fn capture_commands(
    mut q: ResMut<CommandQueue>,
    cam: Res<FlyCamera>,
    mut proj: ResMut<crate::projection::ProjectionSettings>,
    mut hires: ResMut<crate::screenshot::HiresSettings>,
    mut hires_req: ResMut<crate::screenshot::HiresRequest>,
    mut sky: ResMut<SkyboxSettings>,
    mut sky_req: ResMut<SkyboxRequest>,
) {
    if q.wait_frames > 0 || cam.travel.is_some() {
        return;
    }
    let Some(front) = q.queue.front() else { return };
    let mut it = front.split_whitespace();
    let verb = it.next().unwrap_or("").to_ascii_lowercase();
    let args: Vec<String> = it.map(str::to_string).collect();
    let num = |i: usize| args.get(i).and_then(|a| a.parse::<f32>().ok());
    match verb.as_str() {
        "projection" => proj.mode = num(0).unwrap_or(0.0).clamp(0.0, 3.0) as u8,
        "cubeformat" => proj.cubemap_format = num(0).unwrap_or(0.0).clamp(0.0, 5.0) as u8,
        "fisheye" => {
            proj.fisheye_fov = num(0).unwrap_or(proj.fisheye_fov);
            proj.fisheye_zoom = num(1).unwrap_or(proj.fisheye_zoom);
        }
        "hires" => {
            hires.target = UVec2::new(num(0).unwrap_or(8000.0) as u32, num(1).unwrap_or(4000.0) as u32);
            if let Some(t) = num(2) {
                hires.tile_res = t as u32;
            }
            hires_req.0 = true;
        }
        "skybox" => {
            sky.resolution = num(0).unwrap_or(1024.0) as u32;
            sky.cylindrical = args.iter().any(|a| a.eq_ignore_ascii_case("cyl"));
            if args.iter().any(|a| a.eq_ignore_ascii_case("cubemap")) {
                sky.format = "dds cubemap".into();
            } else if let Some(f) = args.iter().find(|a| SKYBOX_FORMATS.contains(&a.to_ascii_lowercase().as_str())) {
                sky.format = f.to_ascii_lowercase();
            }
            sky.depth = match args.iter().find_map(|a| a.parse::<u32>().ok().filter(|b| [8, 16, 32].contains(b))) {
                Some(16) => Depth::U16,
                Some(32) => Depth::F32,
                _ => sky.depth,
            };
            sky.post_process = args.iter().any(|a| a.eq_ignore_ascii_case("bloom"));
            sky.color_correction = sky.depth != Depth::F32;
            sky_req.0 = true;
        }
        _ => return,
    }
    info!("command: {front}");
    q.queue.pop_front();
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<CaptureView>::default())
            .init_resource::<SceneLayers>()
            .init_resource::<SkyboxSettings>()
            .init_resource::<SkyboxRequest>()
            .init_resource::<SkyboxState>()
            .add_systems(Startup, load_skybox_format)
            .add_systems(PreUpdate, capture_commands)
            .add_systems(Update, run_skybox)
            .add_systems(PostUpdate, follow_main.after(UniverseSet::Camera).before(TransformSystems::Propagate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_look_where_they_are_named() {
        for (i, (f, u)) in FACES.iter().enumerate() {
            // The face camera's own centre maps back to that face's centre.
            let (face, st) = dir_to_face(face_rotation(i) * Vec3::NEG_Z);
            assert_eq!(face, i);
            assert!((st - Vec2::splat(0.5)).length() < 1e-5);
            // Its top edge is `up`, its right edge forward x up (a non-mirrored picture).
            let rot = face_rotation(i);
            assert!((rot * Vec3::Y - *u).length() < 1e-5);
            let right = rot * Vec3::X;
            let (_, st_r) = dir_to_face(*f + right * 0.5);
            let (_, st_t) = dir_to_face(*f + *u * 0.5);
            assert!(st_r.x > 0.7 && (st_r.y - 0.5).abs() < 1e-4, "face {i} right {st_r}");
            assert!(st_t.y < 0.3 && (st_t.x - 0.5).abs() < 1e-4, "face {i} top {st_t}");
        }
        // +Z is ahead (camera -z), +X right, +Y up.
        assert_eq!(dir_to_face(Vec3::NEG_Z).0, 4);
        assert_eq!(dir_to_face(Vec3::X).0, 0);
        assert_eq!(dir_to_face(Vec3::Y).0, 2);
    }

    #[test]
    fn cylinder_centre_looks_ahead() {
        assert!((cylinder_dir(Vec2::new(0.5, 0.5)) - Vec3::NEG_Z).length() < 1e-5);
        assert!((cylinder_dir(Vec2::new(0.75, 0.5)) - Vec3::X).length() < 1e-5);
        assert!((cylinder_dir(Vec2::new(0.25, 0.5)) - Vec3::NEG_X).length() < 1e-5);
        assert!((cylinder_dir(Vec2::new(0.3, 0.0)) - Vec3::Y).length() < 1e-5);
        assert!((cylinder_dir(Vec2::new(0.0, 0.5)) - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn halves_and_padding() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!((f16_to_f32(0x3555) - 0.333_25).abs() < 1e-4);
        assert!((f16_to_f32(0x0001) - 5.96e-8).abs() < 1e-9);
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        // 3 x 2 pixels: rows of 24 bytes padded to 256.
        let mut data = vec![0u8; 256 + 24];
        data[256..258].copy_from_slice(&0x3c00u16.to_le_bytes());
        let px = decode_rgba16f(&data, 3, 2);
        assert_eq!(px.len(), 6);
        assert_eq!(px[3][0], 1.0);
        assert_eq!(px[0][0], 0.0);
    }

    #[test]
    fn panorama_from_uniform_faces() {
        let faces: [Vec<[f32; 4]>; 6] = std::array::from_fn(|i| vec![[i as f32, 0.0, 0.0, 1.0]; 16]);
        let px = cylinder_from_faces(&faces, 4, 8, 4);
        // The middle row: back, left, ahead, right going across (SE's map is centred ahead).
        let row: Vec<f32> = px[8..16].iter().map(|p| p[0]).collect();
        assert_eq!(row, vec![5.0, 1.0, 1.0, 4.0, 4.0, 0.0, 0.0, 5.0]);
        assert_eq!(px[0][0], 2.0);
        assert_eq!(px[31][0], 3.0);
        assert!((srgb_encode(1.0) - 1.0).abs() < 1e-6 && srgb_encode(-1.0) == 0.0);
    }
}
