//! Sandbox Engine patch g: registered shapes' vertices, indices and gradient uniforms, pooled in a
//! few large buffers.
//!
//! Each kind lives in blocks owned by the backend, and a shape holds a range of one block for
//! each kind it has, where it would otherwise own buffers of its own: wgpu sizes every pass's
//! resource tracking by the most buffers ever alive at once, so hundreds of shapes (glyphs
//! included) made every later frame of the host slower. Blocks start at `FIRST_BLOCK` and double
//! up to `LARGEST_BLOCK` (a larger shape gets a block of its own size); they live as long as the
//! backend. A dropped shape's ranges are reused only after the work recorded so far has been
//! submitted (`ShapeBuffers::recycle`), so no draw recorded before then reads another shape's
//! data, and a new shape's data is written through the queue, which orders it after everything
//! submitted before.

use std::ops::Range;
use std::sync::{Arc, Mutex};

const FIRST_BLOCK: u64 = 64 * 1024;
const LARGEST_BLOCK: u64 = 4 * 1024 * 1024;

pub struct ShapeBuffers {
    pub vertices: Arena,
    pub indices: Arena,
    pub uniforms: Arena,
}

impl ShapeBuffers {
    pub fn new(limits: &wgpu::Limits) -> Self {
        Self {
            vertices: Arena::new(
                "Shape vertices",
                wgpu::BufferUsages::VERTEX,
                wgpu::COPY_BUFFER_ALIGNMENT,
            ),
            indices: Arena::new(
                "Shape indices",
                wgpu::BufferUsages::INDEX,
                wgpu::COPY_BUFFER_ALIGNMENT,
            ),
            uniforms: Arena::new(
                "Shape uniforms",
                wgpu::BufferUsages::UNIFORM,
                u64::from(limits.min_uniform_buffer_offset_alignment),
            ),
        }
    }

    /// Makes the ranges of shapes dropped so far free for new shapes. Called once the work
    /// recorded so far has been submitted.
    pub fn recycle(&self) {
        for arena in [&self.vertices, &self.indices, &self.uniforms] {
            for block in &arena.blocks {
                block.ranges.lock().expect("shape buffer ranges").recycle();
            }
        }
    }
}

pub struct Arena {
    label: &'static str,
    usage: wgpu::BufferUsages,
    alignment: u64,
    blocks: Vec<Arc<Block>>,
}

#[derive(Debug)]
struct Block {
    buffer: wgpu::Buffer,
    ranges: Mutex<Ranges>,
}

/// A block's free ranges (sorted, never touching) and the ranges given back since the last
/// submission.
#[derive(Debug)]
struct Ranges {
    free: Vec<Range<u64>>,
    given_back: Vec<Range<u64>>,
}

impl Ranges {
    fn take(&mut self, size: u64) -> Option<Range<u64>> {
        let index = self.free.iter().position(|r| r.end - r.start >= size)?;
        let range = &mut self.free[index];
        let taken = range.start..range.start + size;
        range.start = taken.end;
        if range.is_empty() {
            self.free.remove(index);
        }
        Some(taken)
    }

    fn recycle(&mut self) {
        for range in std::mem::take(&mut self.given_back) {
            let index = self.free.partition_point(|r| r.start < range.start);
            let joins_previous = index > 0 && self.free[index - 1].end == range.start;
            let joins_next = index < self.free.len() && self.free[index].start == range.end;
            match (joins_previous, joins_next) {
                (true, true) => {
                    self.free[index - 1].end = self.free[index].end;
                    self.free.remove(index);
                }
                (true, false) => self.free[index - 1].end = range.end,
                (false, true) => self.free[index].start = range.start,
                (false, false) => self.free.insert(index, range),
            }
        }
    }
}

/// A shape's range in one block; it goes back to the block when dropped.
#[derive(Debug)]
pub struct Slice {
    block: Arc<Block>,
    range: Range<u64>,
}

impl Slice {
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.block.buffer
    }

    pub fn offset(&self) -> wgpu::BufferAddress {
        self.range.start
    }
}

impl Drop for Slice {
    fn drop(&mut self) {
        self.block
            .ranges
            .lock()
            .expect("shape buffer ranges")
            .given_back
            .push(self.range.clone());
    }
}

impl Arena {
    fn new(label: &'static str, usage: wgpu::BufferUsages, alignment: u64) -> Self {
        Self {
            label,
            usage,
            alignment,
            blocks: Vec::new(),
        }
    }

    /// `data` in a range of one block, or `None` for no data. `data` is whole 4-byte words, as
    /// a queue write needs (vertices and indices are 4-byte values, uniforms 16-byte records).
    pub fn add(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
    ) -> Option<Slice> {
        if data.is_empty() {
            return None;
        }
        let size = (data.len() as u64).next_multiple_of(self.alignment);
        let found = self.blocks.iter().find_map(|block| {
            let range = block
                .ranges
                .lock()
                .expect("shape buffer ranges")
                .take(size)?;
            Some((block.clone(), range))
        });
        let (block, range) = found.unwrap_or_else(|| self.new_block(device, size));
        queue.write_buffer(&block.buffer, range.start, data);
        Some(Slice { block, range })
    }

    /// A new block holding at least `size` bytes, its first `size` taken.
    fn new_block(&mut self, device: &wgpu::Device, size: u64) -> (Arc<Block>, Range<u64>) {
        let doubled = (0..self.blocks.len()).fold(FIRST_BLOCK, |s, _| (s * 2).min(LARGEST_BLOCK));
        let block_size = doubled.max(size);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: block_size,
            usage: self.usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let block = Arc::new(Block {
            buffer,
            ranges: Mutex::new(Ranges {
                free: (size < block_size)
                    .then_some(size..block_size)
                    .into_iter()
                    .collect(),
                given_back: Vec::new(),
            }),
        });
        self.blocks.push(block.clone());
        (block, 0..size)
    }
}
