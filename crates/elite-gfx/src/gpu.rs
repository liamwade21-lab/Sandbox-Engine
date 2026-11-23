//! One set of Ruffle's GPU objects (pipelines, samplers, shaders) on the engine's own device,
//! shared by every movie.

use ruffle_render_wgpu::descriptors::Descriptors;
use ruffle_render_wgpu::wgpu;
use std::sync::Arc;

pub struct Gpu {
    pub(crate) descriptors: Arc<Descriptors>,
}

impl Gpu {
    /// From clones of the engine's own instance, adapter, device and queue.
    pub fn new(instance: wgpu::Instance, adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            descriptors: Arc::new(Descriptors::new(instance, adapter, device, queue)),
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.descriptors.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.descriptors.queue
    }
}
