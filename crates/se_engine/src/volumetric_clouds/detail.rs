//! The detail cover (docs/spec-clouds-v3.md step 4): each layer's cover over a stack of square
//! patches around the camera, each twice as wide as the one inside it (Earth: 800, 1,600 and 3,200 km
//! across, 400 m to 1.6 km a texel, where the cover map's are about 5 km), so the clouds keep the
//! shapes of SE's flat shells (its finer tiles, its generator at pixel size) from the ground up to a
//! few thousand km. A level is made once the camera is low enough for its texels to show. A patch
//! lies in its layer's frame (it drifts with the clouds), centred on the camera's point when made and
//! made again once the camera is a quarter of its half-width from that centre: from the catalog map's
//! deepest tiles on a worker thread, or from the cover generator on the GPU (one a frame). The trace
//! reads the finest patch that holds a point at its footprint, blending into the next towards its edge
//! and into the cover map last (`volumetric_clouds.wesl` `cover_at`).

use std::collections::HashMap;

use se_core::planet_tiles::TileKey;

use super::*;

/// A patch's texels a side; each layer's levels, the finest's half-width on the ground (m), each next
/// twice as wide, at most this angle of the body (a level as wide as the one inside it is left out).
pub(super) const DETAIL_SIZE: u32 = 2048;
pub(super) const DETAIL_LEVELS: usize = 3;
const DETAIL_HALF_M: f64 = 400_000.0;
const DETAIL_MAX_ANGLE: f64 = 0.6;
const DETAIL_FORMAT: TextureFormat = TextureFormat::R8Unorm;
/// A level is made while the camera is lower than this many times its half-width above the ground
/// (straight down its texels are then finer than about four cloud pixels at Ultra).
const DETAIL_NEAR: f64 = 3.5;

/// A patch's axes around its centre `c` (as the shader's `detail_axes`): texel (x, y) of the patch
/// lies towards `c + (u e1 + v e2) tan_a`, u and v from -1 to 1 across it (v down).
fn frame_axes(c: DVec3) -> (DVec3, DVec3) {
    let a = if c.y.abs() > 0.99 { DVec3::X } else { DVec3::Y };
    let e1 = a.cross(c).normalize();
    (e1, c.cross(e1))
}

/// `SE_DETAIL_COVER=0` leaves the patches out (comparisons).
fn detail_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_DETAIL_COVER").map_or(true, |v| v.trim() != "0"))
}

/// A layer's patch for the render world.
#[derive(Clone, Default)]
pub(super) struct DetailSlot {
    /// Bumped for each new patch; the render world makes it once.
    id: u32,
    /// xyz the centre (the layer's frame, unit), w the tangent of the half-width; w 0: no patch.
    frame: Vec4,
    /// 1: from the catalog map (`data`), 2: from the generator on the GPU.
    kind: u8,
    data: Option<Arc<Vec<u8>>>,
}

/// The active body's patches (`CloudBody::id`), `DETAIL_LEVELS` a layer, finest first.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub(super) struct DetailCover {
    body_id: u64,
    slots: [DetailSlot; MAX_LAYERS * DETAIL_LEVELS],
}

impl DetailCover {
    /// The patches' frames for `CloudView::detail`.
    pub(super) fn frames(&self) -> [Vec4; MAX_LAYERS * DETAIL_LEVELS] {
        self.slots.each_ref().map(|s| s.frame)
    }
}

/// The catalog maps' patches being made (each with its centre).
#[derive(Default)]
pub(super) struct DetailJobs([Option<Task<(DVec3, Vec<u8>)>>; MAX_LAYERS * DETAIL_LEVELS]);

/// Keeps the active body's patches around the camera. `cam` is the camera in sphere space, `spins`
/// each layer's turn, `on` whether the body gets them.
#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    detail: &mut DetailCover,
    jobs: &mut DetailJobs,
    body_id: u64,
    specs: &[LayerSpec],
    spins: &[f64; MAX_LAYERS],
    cam: DVec3,
    radius: f64,
    on: bool,
    vfs: &Arc<se_core::vfs::Vfs>,
) {
    if detail.body_id != body_id {
        *detail = DetailCover { body_id, ..default() };
        *jobs = DetailJobs::default();
    }
    let on = on && detail_on();
    let height = cam.length() - radius;
    // The GPU makes one generated patch a frame (`build`): the rest keep their old one meanwhile.
    let mut gpu_free = true;
    for i in 0..MAX_LAYERS {
        let kind = match specs.get(i) {
            Some(s) if s.cover.kind.x as i32 == 1 && s.map.is_some() => 1,
            Some(s) if matches!(s.cover.kind.x as i32, 2 | 3) => 2,
            _ => 0,
        };
        // The camera's point in the layer's frame (as the shader's `cover_at`).
        let (c, s) = (spins[i].cos(), spins[i].sin());
        let p = cam.normalize_or(DVec3::Y);
        let at = DVec3::new(c * p.x - s * p.z, p.y, s * p.x + c * p.z);
        let mut inner = 0.0;
        for k in 0..DETAIL_LEVELS {
            let n = i * DETAIL_LEVELS + k;
            let half = DETAIL_HALF_M * (1u32 << k) as f64;
            let angle = (half / radius).min(DETAIL_MAX_ANGLE);
            let tan_a = angle.tan();
            let slot = &mut detail.slots[n];
            let wanted = on && kind != 0 && height < DETAIL_NEAR * half && angle > inner * 1.01;
            inner = angle;
            if !wanted {
                if slot.frame.w > 0.0 {
                    slot.frame = Vec4::ZERO;
                    slot.data = None;
                }
                jobs.0[n] = None;
                continue;
            }
            if let Some((c, bytes)) = jobs.0[n].as_mut().and_then(check_ready) {
                jobs.0[n] = None;
                *slot = DetailSlot { id: slot.id + 1, frame: c.as_vec3().extend(tan_a as f32), kind: 1, data: Some(Arc::new(bytes)) };
            }
            let off = if slot.frame.w > 0.0 && slot.kind == kind { at.angle_between(slot.frame.truncate().as_dvec3()) } else { f64::INFINITY };
            if off <= angle * 0.25 {
                continue;
            }
            match kind {
                1 if jobs.0[n].is_none() => {
                    let (vfs, layer) = (vfs.clone(), specs[i].map.clone().expect("a mapped layer"));
                    jobs.0[n] = Some(AsyncComputeTaskPool::get().spawn(async move { (at, compose_patch(&vfs, &layer, at, tan_a)) }));
                }
                2 if gpu_free => {
                    *slot = DetailSlot { id: slot.id + 1, frame: at.as_vec3().extend(tan_a as f32), kind: 2, data: None };
                    gpu_free = false;
                }
                _ => {}
            }
        }
    }
}

/// A catalog map's patch around `c` (its layer's frame): SE's cover after its 5% cut (as the cover
/// map holds it), one byte a texel, from the layer's deepest tiles as fine as the patch.
fn compose_patch(vfs: &se_core::vfs::Vfs, layer: &TileLayer, c: DVec3, tan_a: f64) -> Vec<u8> {
    let started = std::time::Instant::now();
    let n = DETAIL_SIZE as usize;
    let (e1, e2) = frame_axes(c);
    let texel = 2.0 * tan_a / n as f64;
    let want = (std::f64::consts::FRAC_PI_2 / (252.0 * texel * 1.5)).log2().ceil().max(0.0) as u8;
    let level = want.min(layer.max_level);
    let tiles = 1u32 << level;
    let mut cache: HashMap<TileKey, Option<image::RgbaImage>> = HashMap::new();
    let mut out = vec![0u8; n * n];
    for y in 0..n {
        for x in 0..n {
            let u = (x as f64 + 0.5) / n as f64 * 2.0 - 1.0;
            let v = (y as f64 + 0.5) / n as f64 * 2.0 - 1.0;
            let (face, fu, fv) = CubeFace::from_dir((c + (e1 * u + e2 * v) * tan_a).normalize());
            let col = ((fu * tiles as f64).floor() as u32).min(tiles - 1);
            let row = ((fv * tiles as f64).floor() as u32).min(tiles - 1);
            let key = TileKey { face, level, row, col };
            let img = cache.entry(key).or_insert_with(|| vfs.read(&layer.tile_path(key)).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8()));
            let Some(img) = img else { continue };
            let (w, h) = img.dimensions();
            let k = w as f64 / 256.0;
            // The tile's inner 252 texels (as clouds.wesl `tile_uv`), bilinear.
            let tx = ((2.0 + (fu * tiles as f64 - col as f64) * 252.0) * k - 0.5).clamp(0.0, w as f64 - 1.0);
            let ty = ((2.0 + (fv * tiles as f64 - row as f64) * 252.0) * k - 0.5).clamp(0.0, h as f64 - 1.0);
            let (x0, y0) = (tx.floor() as u32, ty.floor() as u32);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (tx - x0 as f64, ty - y0 as f64);
            let p = |x: u32, y: u32| img.get_pixel(x, y)[0] as f64 / 255.0;
            let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
            let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
            let raw = top + (bot - top) * fy;
            out[y * n + x] = ((raw - 0.05).max(0.0) / 0.95 * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    info!("volumetric clouds: detail cover (level {level}, {} tiles) in {:.0} ms", cache.len(), started.elapsed().as_secs_f64() * 1000.0);
    out
}

/// The generator's patch pass: the patch's frame and which layer. Layout must match
/// `volumetric_clouds.wesl` `DetailGen`.
#[derive(Clone, Copy, ShaderType, Default, Debug)]
pub(super) struct DetailGen {
    frame: Vec4,
    /// x the layer, y the patch's texels a side.
    info: Vec4,
}

/// The patches on the GPU: an array layer each (`DetailCover::slots`' order), and which patch each
/// holds.
#[derive(Resource, Default)]
pub(super) struct DetailGpu {
    tex: Option<(Texture, TextureView, Vec<TextureView>)>,
    built: [(u64, u32); MAX_LAYERS * DETAIL_LEVELS],
}

impl DetailGpu {
    /// The patches (all layers), for the trace.
    pub(super) fn view(&self) -> Option<&TextureView> {
        self.tex.as_ref().map(|t| &t.1)
    }
}

/// Makes the patches the main world asked for this frame; false while one cannot be made yet (the
/// trace would read a stale one).
#[allow(clippy::too_many_arguments)]
pub(super) fn build(gpu: &mut DetailGpu, detail: &DetailCover, body: &CloudBody, pipelines: &CloudPipelines, cache: &PipelineCache, device: &RenderDevice, encoder: &mut CommandEncoder) -> bool {
    let (texture, _, layers) = gpu.tex.get_or_insert_with(|| {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("vol_clouds_detail"),
            size: Extent3d { width: DETAIL_SIZE, height: DETAIL_SIZE, depth_or_array_layers: (MAX_LAYERS * DETAIL_LEVELS) as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: DETAIL_FORMAT,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let array = texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
        let layers = (0..(MAX_LAYERS * DETAIL_LEVELS) as u32)
            .map(|l| texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2), base_array_layer: l, array_layer_count: Some(1), ..default() }))
            .collect();
        (texture, array, layers)
    });
    let mut all = true;
    for (i, slot) in detail.slots.iter().enumerate() {
        let key = (detail.body_id, slot.id);
        if slot.frame.w <= 0.0 || gpu.built[i] == key {
            continue;
        }
        match (slot.kind, &slot.data) {
            (1, Some(data)) => {
                let staging = device.create_buffer_with_data(&BufferInitDescriptor { label: Some("vol_clouds_detail_upload"), contents: data, usage: BufferUsages::COPY_SRC });
                encoder.copy_buffer_to_texture(
                    TexelCopyBufferInfo { buffer: &staging, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(DETAIL_SIZE), rows_per_image: Some(DETAIL_SIZE) } },
                    TexelCopyTextureInfo { texture, mip_level: 0, origin: Origin3d { x: 0, y: 0, z: i as u32 }, aspect: TextureAspect::All },
                    Extent3d { width: DETAIL_SIZE, height: DETAIL_SIZE, depth_or_array_layers: 1 },
                );
            }
            (2, _) => {
                let Some(pipe) = cache.get_render_pipeline(pipelines.cover_patch) else {
                    all = false;
                    continue;
                };
                let dg = DetailGen { frame: slot.frame, info: Vec4::new((i / DETAIL_LEVELS) as f32, DETAIL_SIZE as f32, 0.0, 0.0) };
                let (Some(gen_buf), Some(dg_buf)) = (uniform_buffer_of(device, "vol_clouds_cover_gen", &body.cover), uniform_buffer_of(device, "vol_clouds_detail_gen", &dg)) else {
                    all = false;
                    continue;
                };
                let group = device.create_bind_group(
                    "vol_clouds_detail_gen",
                    &cache.get_bind_group_layout(&pipelines.patch_layout),
                    &BindGroupEntries::with_indices(((20, gen_buf.as_entire_binding()), (25, dg_buf.as_entire_binding()))),
                );
                fullscreen_pass(encoder, "vol_clouds_detail_gen", &[color_attachment(&layers[i])], pipe, &group, &[], UVec2::splat(DETAIL_SIZE));
            }
            _ => {
                all = false;
                continue;
            }
        }
        gpu.built[i] = key;
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_axes_are_orthonormal_everywhere() {
        for c in [DVec3::X, DVec3::Y, -DVec3::Y, DVec3::new(0.3, 0.9, -0.2).normalize()] {
            let (e1, e2) = frame_axes(c);
            assert!((e1.length() - 1.0).abs() < 1e-9 && (e2.length() - 1.0).abs() < 1e-9);
            assert!(e1.dot(c).abs() < 1e-9 && e2.dot(c).abs() < 1e-9 && e1.dot(e2).abs() < 1e-9);
        }
    }
}
