//! Drawing only the frames that differ (ARCHITECTURE.md A10): Ruffle marks every frame it
//! runs as needing a render, so a movie showing nothing new would be drawn 30 times a second. This
//! render backend stands between Ruffle's player and its wgpu backend and passes a frame on only when
//! it would draw something different from the last frame drawn into the target.
//!
//! The test is EXACT, not a hash: the frame's command list is compared with the last drawn one
//! command by command (the same shapes and bitmaps, by their handles; the same transforms, colours,
//! blends and masks, by value), and the last drawn list is kept alive, so a handle's address cannot
//! be reused by another shape meanwhile. What can change a frame outside its list forces the next
//! frame through: bitmap-cache work queued with the frame, a texture written (`update_texture`,
//! `render_offscreen`, `apply_filter`, a Pixel Bender run), a new viewport or quality, a Stage3D context.
//!
//! A frame can be rendered and held without being drawn (`Movie::changed`), and then thrown away
//! when a newer one is rendered. Ruffle marks a frame's bitmap caches clean as it renders it, so the
//! drawing of those caches is never thrown away: it goes in front of the next frame drawn.

use ruffle_core::swf::Color;
use ruffle_render::backend::{
    BitmapCacheEntry, Context3D, Context3DProfile, PixelBenderOutput, PixelBenderTarget, RenderBackend, ShapeHandle,
    ViewportDimensions,
};
use ruffle_render::bitmap::{Bitmap, BitmapHandle, BitmapSource, PixelRegion, RgbaBufRead, SyncHandle};
use ruffle_render::commands::{Command, CommandList, RenderBlendMode};
use ruffle_render::error::Error;
use ruffle_render::filters::Filter;
use ruffle_render::pixel_bender::{PixelBenderShader, PixelBenderShaderHandle};
use ruffle_render::pixel_bender_support::PixelBenderShaderArgument;
use ruffle_render::quality::StageQuality;
use ruffle_render::shape_utils::DistilledShape;
use ruffle_render_wgpu::backend::WgpuRenderBackend;
use std::borrow::Cow;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::target::Target;

/// A frame as Ruffle hands it over.
struct Frame {
    clear: Color,
    commands: CommandList,
    caches: Vec<BitmapCacheEntry>,
}

/// Ruffle's wgpu backend with the frame test in front of `submit_frame`.
pub struct Frames {
    inner: WgpuRenderBackend<Target>,
    /// The last frame drawn into the target: its clear colour and commands.
    last: Option<(Color, CommandList)>,
    /// Something outside the command list changed since the last frame drawn.
    dirty: bool,
    /// A Stage3D context exists (it draws outside the command list): every frame goes through.
    stage3d: bool,
    /// While set, `submit_frame` keeps the frame in `held` instead of drawing it (`Movie::changed`).
    holding: bool,
    held: Option<Frame>,
    /// The bitmap-cache drawing of frames rendered and thrown away undrawn, for the next frame drawn.
    orphans: Vec<BitmapCacheEntry>,
    /// Frames drawn into the target.
    drawn: u64,
    /// Whether frames that change nothing are skipped (always, but the tests compare with off).
    skipping: bool,
    /// Time spent comparing frames, and how many were compared (the tests' measure of the test's cost).
    #[cfg(test)]
    pub checking: std::cell::Cell<(std::time::Duration, u64)>,
    /// Bitmap-cache entries passed on to be drawn (the tests' proof that a movie has caches).
    #[cfg(test)]
    pub caches_drawn: u64,
}

impl Frames {
    pub fn new(inner: WgpuRenderBackend<Target>) -> Self {
        Self {
            inner,
            last: None,
            dirty: true,
            stage3d: false,
            holding: false,
            held: None,
            orphans: Vec::new(),
            drawn: 0,
            skipping: true,
            #[cfg(test)]
            checking: Default::default(),
            #[cfg(test)]
            caches_drawn: 0,
        }
    }

    /// Frames drawn into the target so far.
    pub fn drawn(&self) -> u64 {
        self.drawn
    }

    pub fn skipping(&self) -> bool {
        self.skipping
    }

    /// Turns the skip off (every frame is drawn) or on, for comparisons.
    #[cfg(test)]
    pub fn set_skipping(&mut self, skipping: bool) {
        self.skipping = skipping;
    }

    /// Keeps the next frame Ruffle submits instead of drawing it.
    pub fn hold(&mut self) {
        self.holding = true;
    }

    /// Stops holding; whether the frame held would change the target. One that would not is
    /// thrown away (nothing of it is needed: such a frame has no cache drawing).
    pub fn release(&mut self) -> bool {
        self.holding = false;
        match self.held.take() {
            Some(frame) if self.differs(&frame) => {
                self.held = Some(frame);
                true
            }
            Some(frame) => {
                self.throw_away(frame);
                false
            }
            None => false,
        }
    }

    /// Draws the frame `release` kept.
    pub fn draw_held(&mut self) {
        if let Some(frame) = self.held.take() {
            self.draw(frame);
        }
    }

    /// A frame that will never be drawn: its bitmap-cache drawing waits for the next one drawn,
    /// which therefore is drawn.
    fn throw_away(&mut self, frame: Frame) {
        if !frame.caches.is_empty() {
            self.orphans = supersede(std::mem::take(&mut self.orphans), frame.caches);
            self.dirty = true;
        }
    }

    /// Whether drawing `frame` would change the target.
    fn differs(&self, frame: &Frame) -> bool {
        #[cfg(test)]
        let start = std::time::Instant::now();
        let differs = !self.skipping
            || self.dirty
            || self.stage3d
            || !frame.caches.is_empty()
            || self.last.as_ref().is_none_or(|(clear, commands)| *clear != frame.clear || !same_list(commands, &frame.commands));
        #[cfg(test)]
        {
            let (spent, count) = self.checking.get();
            self.checking.set((spent + start.elapsed(), count + 1));
        }
        differs
    }

    /// Draws `frame` (after the cache drawing of frames thrown away) if it differs from the last
    /// one drawn.
    fn draw(&mut self, frame: Frame) {
        if !self.differs(&frame) {
            return;
        }
        #[cfg(test)]
        let start = std::time::Instant::now();
        let commands = frame.commands.clone();
        #[cfg(test)]
        {
            let (spent, count) = self.checking.get();
            self.checking.set((spent + start.elapsed(), count));
        }
        let caches = supersede(std::mem::take(&mut self.orphans), frame.caches);
        #[cfg(test)]
        {
            self.caches_drawn += caches.len() as u64;
        }
        self.inner.submit_frame(frame.clear, commands, caches);
        self.last = Some((frame.clear, frame.commands));
        self.dirty = false;
        self.drawn += 1;
    }
}

/// Cache drawing `older` then `newer`, without what `newer` draws again: a cache is drawn whole (its
/// texture cleared first), so its last drawing alone counts, and the list stays as long as the
/// caches on screen however many frames are thrown away. (A cache inside a cached parent is drawn
/// before it, and Ruffle redraws the parent whenever the child changes, so no parent loses a child.)
fn supersede(mut older: Vec<BitmapCacheEntry>, newer: Vec<BitmapCacheEntry>) -> Vec<BitmapCacheEntry> {
    older.retain(|old| !newer.iter().any(|new| new.handle == old.handle));
    older.extend(newer);
    older
}

fn same_list(a: &CommandList, b: &CommandList) -> bool {
    a.commands.len() == b.commands.len() && a.commands.iter().zip(&b.commands).all(|(x, y)| same_command(x, y))
}

fn same_shape(a: &ShapeHandle, b: &ShapeHandle) -> bool {
    Arc::ptr_eq(&a.0, &b.0)
}

fn same_blend(a: &RenderBlendMode, b: &RenderBlendMode) -> bool {
    match (a, b) {
        (RenderBlendMode::Builtin(x), RenderBlendMode::Builtin(y)) => x == y,
        (RenderBlendMode::Shader(x), RenderBlendMode::Shader(y)) => Arc::ptr_eq(&x.0, &y.0),
        _ => false,
    }
}

/// Every variant by name: a new one in Ruffle stops the build here.
fn same_command(a: &Command, b: &Command) -> bool {
    match (a, b) {
        (
            Command::RenderBitmap { bitmap, transform, smoothing, pixel_snapping, region },
            Command::RenderBitmap {
                bitmap: bitmap2,
                transform: transform2,
                smoothing: smoothing2,
                pixel_snapping: pixel_snapping2,
                region: region2,
            },
        ) => {
            bitmap == bitmap2
                && transform == transform2
                && smoothing == smoothing2
                && pixel_snapping == pixel_snapping2
                && region == region2
        }
        (
            Command::RenderStage3D { bitmap, transform },
            Command::RenderStage3D { bitmap: bitmap2, transform: transform2 },
        ) => bitmap == bitmap2 && transform == transform2,
        (Command::RenderShape { shape, transform }, Command::RenderShape { shape: shape2, transform: transform2 }) => {
            same_shape(shape, shape2) && transform == transform2
        }
        (
            Command::RenderAlphaMask { maskee_commands, mask_commands },
            Command::RenderAlphaMask { maskee_commands: maskee2, mask_commands: mask2 },
        ) => same_list(maskee_commands, maskee2) && same_list(mask_commands, mask2),
        (Command::DrawRect { color, matrix }, Command::DrawRect { color: color2, matrix: matrix2 })
        | (Command::DrawLine { color, matrix }, Command::DrawLine { color: color2, matrix: matrix2 })
        | (Command::DrawLineRect { color, matrix }, Command::DrawLineRect { color: color2, matrix: matrix2 }) => {
            color == color2 && matrix == matrix2
        }
        (Command::PushMask, Command::PushMask)
        | (Command::ActivateMask, Command::ActivateMask)
        | (Command::DeactivateMask, Command::DeactivateMask)
        | (Command::PopMask, Command::PopMask) => true,
        (Command::Blend(list, mode), Command::Blend(list2, mode2)) => same_list(list, list2) && same_blend(mode, mode2),
        (
            Command::RenderBitmap { .. }
            | Command::RenderStage3D { .. }
            | Command::RenderShape { .. }
            | Command::RenderAlphaMask { .. }
            | Command::DrawRect { .. }
            | Command::DrawLine { .. }
            | Command::DrawLineRect { .. }
            | Command::PushMask
            | Command::ActivateMask
            | Command::DeactivateMask
            | Command::PopMask
            | Command::Blend(..),
            _,
        ) => false,
    }
}

/// Everything forwarded to Ruffle's wgpu backend; what writes outside the command list marks the
/// next frame as changed.
impl RenderBackend for Frames {
    fn viewport_dimensions(&self) -> ViewportDimensions {
        self.inner.viewport_dimensions()
    }

    fn set_viewport_dimensions(&mut self, dimensions: ViewportDimensions) {
        self.dirty = true;
        self.inner.set_viewport_dimensions(dimensions)
    }

    fn register_shape(&mut self, shape: DistilledShape, bitmap_source: &dyn BitmapSource) -> ShapeHandle {
        self.inner.register_shape(shape, bitmap_source)
    }

    fn register_shape_with_scale(
        &mut self,
        shape: DistilledShape,
        bitmap_source: &dyn BitmapSource,
        scale: f32,
    ) -> ShapeHandle {
        self.inner.register_shape_with_scale(shape, bitmap_source, scale)
    }

    fn render_offscreen(
        &mut self,
        handle: BitmapHandle,
        commands: CommandList,
        quality: StageQuality,
        bounds: PixelRegion,
    ) -> Option<Box<dyn SyncHandle>> {
        self.dirty = true;
        self.inner.render_offscreen(handle, commands, quality, bounds)
    }

    fn apply_filter(
        &mut self,
        source: BitmapHandle,
        source_point: (u32, u32),
        source_size: (u32, u32),
        destination: BitmapHandle,
        dest_point: (i32, i32),
        filter: Filter,
    ) -> Option<Box<dyn SyncHandle>> {
        self.dirty = true;
        self.inner.apply_filter(source, source_point, source_size, destination, dest_point, filter)
    }

    fn is_filter_supported(&self, filter: &Filter) -> bool {
        self.inner.is_filter_supported(filter)
    }

    fn is_offscreen_supported(&self) -> bool {
        self.inner.is_offscreen_supported()
    }

    fn submit_frame(&mut self, clear: Color, commands: CommandList, caches: Vec<BitmapCacheEntry>) {
        // A frame held by an earlier `Movie::changed` and never drawn is out of date now.
        if let Some(old) = self.held.take() {
            self.throw_away(old);
        }
        let frame = Frame { clear, commands, caches };
        if self.holding {
            self.held = Some(frame);
        } else {
            self.draw(frame);
        }
    }

    fn create_empty_texture(&mut self, width: NonZeroU32, height: NonZeroU32) -> Result<BitmapHandle, Error> {
        self.inner.create_empty_texture(width, height)
    }

    fn register_bitmap(&mut self, bitmap: Bitmap<'_>) -> Result<BitmapHandle, Error> {
        self.inner.register_bitmap(bitmap)
    }

    fn update_texture(&mut self, handle: &BitmapHandle, bitmap: Bitmap<'_>, region: PixelRegion) -> Result<(), Error> {
        self.dirty = true;
        self.inner.update_texture(handle, bitmap, region)
    }

    fn create_context3d(&mut self, profile: Context3DProfile) -> Result<Box<dyn Context3D>, Error> {
        self.stage3d = true;
        self.inner.create_context3d(profile)
    }

    fn debug_info(&self) -> Cow<'static, str> {
        self.inner.debug_info()
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn set_quality(&mut self, quality: StageQuality) {
        self.dirty = true;
        self.inner.set_quality(quality)
    }

    fn compile_pixelbender_shader(&mut self, shader: PixelBenderShader) -> Result<PixelBenderShaderHandle, Error> {
        self.inner.compile_pixelbender_shader(shader)
    }

    fn run_pixelbender_shader(
        &mut self,
        handle: PixelBenderShaderHandle,
        arguments: &[PixelBenderShaderArgument],
        target: &PixelBenderTarget,
    ) -> Result<PixelBenderOutput, Error> {
        self.dirty = true;
        self.inner.run_pixelbender_shader(handle, arguments, target)
    }

    fn resolve_sync_handle(&mut self, handle: Box<dyn SyncHandle>, with_rgba: RgbaBufRead) -> Result<(), Error> {
        self.inner.resolve_sync_handle(handle, with_rgba)
    }
}
