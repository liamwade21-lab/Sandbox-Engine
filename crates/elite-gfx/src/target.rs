//! The texture a movie draws into: the caller's, never replaced (a material's bind group keeps
//! pointing at it, ARCHITECTURE.md A10 UI3).
//!
//! Format `Rgba8Unorm` only: Ruffle draws in the target's own format and Flash composes in sRGB
//! space, so the texels are sRGB-encoded colours with premultiplied alpha (transparent where the
//! movie draws nothing). An sRGB view format would make the GPU blend in linear light instead,
//! which is not how the movies were authored; whoever samples the texture decodes the sRGB itself.

use ruffle_render_wgpu::target::{RenderTarget, RenderTargetFrame};
use ruffle_render_wgpu::wgpu;

/// The one format a movie's target may have.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Debug)]
pub struct Target {
    texture: wgpu::Texture,
}

impl Target {
    /// Checks the caller's texture can be drawn into.
    pub fn new(texture: wgpu::Texture) -> Result<Self, String> {
        if texture.format() != FORMAT {
            return Err(format!("the target texture is {:?}; a movie needs {FORMAT:?}", texture.format()));
        }
        if !texture.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            return Err("the target texture needs RENDER_ATTACHMENT usage".into());
        }
        if texture.dimension() != wgpu::TextureDimension::D2 || texture.sample_count() != 1 {
            return Err("the target texture must be a plain 2D texture".into());
        }
        Ok(Self { texture })
    }
}

#[derive(Debug)]
pub struct Frame(wgpu::TextureView);

impl RenderTargetFrame for Frame {
    fn into_view(self) -> wgpu::TextureView {
        self.0
    }

    fn view(&self) -> &wgpu::TextureView {
        &self.0
    }
}

impl RenderTarget for Target {
    type Frame = Frame;

    /// The texture is the caller's and keeps its size: the player's viewport is set to it once.
    fn resize(&mut self, _device: &wgpu::Device, _width: u32, _height: u32) {}

    fn format(&self) -> wgpu::TextureFormat {
        FORMAT
    }

    fn width(&self) -> u32 {
        self.texture.width()
    }

    fn height(&self) -> u32 {
        self.texture.height()
    }

    fn get_next_texture(&mut self) -> Option<Frame> {
        Some(Frame(self.texture.create_view(&Default::default())))
    }

    fn submit<I: IntoIterator<Item = wgpu::CommandBuffer>>(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        command_buffers: I,
        _frame: Frame,
    ) -> wgpu::SubmissionIndex {
        queue.submit(command_buffers)
    }
}
