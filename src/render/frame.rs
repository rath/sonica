use anyhow::Result;
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use super::gpu::GpuContext;

pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// How many frames may be in flight between GPU render and CPU readback.
///
/// Without pipelining, every frame waits for its own copy, serializing the GPU
/// and the CPU on the readback (the profiled bottleneck). Two buffers let the
/// next frame render while the current frame's copy drains; one instant of
/// extra latency, doubled throughput.
const FRAME_LAG: usize = 2;

/// Identifies a queued render so the CPU can composite overlays onto the
/// correct frame when its pixels finally read back.
#[derive(Clone, Copy, Debug)]
pub struct FrameStamp {
    /// Frame number within the whole render (matches the loop index order).
    pub index: u32,
    /// Timestamp of this frame's audio slice, in seconds.
    pub time: f32,
}

pub struct FrameRenderer {
    pub render_texture: wgpu::Texture,
    pub render_texture_view: wgpu::TextureView,
    /// Ring of GPU→CPU staging buffers; `slot` rotates across submissions.
    output_buffers: Vec<wgpu::Buffer>,
    pending: VecDeque<PendingReadback>,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
    unpadded_bytes_per_row: u32,
}

struct PendingReadback {
    slot: usize,
    receiver: Receiver<Result<(), wgpu::BufferAsyncError>>,
    stamp: FrameStamp,
}

impl FrameRenderer {
    pub fn new(gpu: &GpuContext, width: u32, height: u32) -> Self {
        let render_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let render_texture_view = render_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let output_buffers = (0..FRAME_LAG)
            .map(|slot| {
                gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("readback_buffer_{slot}")),
                    size: (padded_bytes_per_row * height) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();

        Self {
            render_texture,
            render_texture_view,
            output_buffers,
            pending: VecDeque::new(),
            width,
            height,
            padded_bytes_per_row,
            unpadded_bytes_per_row,
        }
    }

    /// True when queue_readback can be called without exhausting the ring;
    /// otherwise one collect_oldest must run first.
    pub fn has_free_slot(&self) -> bool {
        self.pending.len() < self.output_buffers.len()
    }

    /// Render the template into `render_texture` without reading it back.
    pub fn render(
        &self,
        gpu: &GpuContext,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
    ) -> Result<()> {
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.render_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..3, 0..1); // fullscreen triangle
        }

        gpu.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// Queue an async copy of `texture` into the next ring slot.
    ///
    /// The readback does not block; call [`FrameRenderer::collect_oldest`] to
    /// drain completed frames in FIFO order.
    pub fn queue_readback(
        &mut self,
        gpu: &GpuContext,
        texture: &wgpu::Texture,
        stamp: FrameStamp,
    ) -> Result<()> {
        if self.pending.len() >= self.output_buffers.len() {
            anyhow::bail!(
                "readback ring exhausted ({} frames in flight); collect_oldest before queueing more",
                self.pending.len()
            );
        }

        // wgpu validation errors on a size/format mismatch surface as panics far
        // from the cause, so reject it here with a descriptive error instead.
        if texture.width() != self.width
            || texture.height() != self.height
            || texture.format() != TEXTURE_FORMAT
        {
            anyhow::bail!(
                "readback_texture: texture is {}x{} {:?}, but the frame renderer expects {}x{} {:?}",
                texture.width(),
                texture.height(),
                texture.format(),
                self.width,
                self.height,
                TEXTURE_FORMAT
            );
        }

        let slot = self.pending.len();
        let slot = if let Some(oldest_free) = (0..self.output_buffers.len())
            .find(|slot| self.pending.iter().all(|p| p.slot != *slot))
        {
            oldest_free
        } else {
            slot
        };

        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback_encoder"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.output_buffers[slot],
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        gpu.queue.submit(std::iter::once(encoder.finish()));

        let buffer_slice = self.output_buffers[slot].slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            // Never unwrap inside a map callback: a dropped receiver would
            // abort the whole process during an unrelated poll.
            let _ = sender.send(result);
        });

        self.pending.push_back(PendingReadback {
            slot,
            receiver,
            stamp,
        });

        Ok(())
    }

    /// Collect the oldest frame whose copy has completed, writing its pixels
    /// into `out` and returning its stamp.
    ///
    /// Waits by spinning short non-blocking polls — the GPU is still busy
    /// rendering the following frames, so a blocking poll here would stall
    /// the pipeline and defeat the point of the ring.
    pub fn collect_oldest(
        &mut self,
        gpu: &GpuContext,
        out: &mut Vec<u8>,
    ) -> Result<Option<FrameStamp>> {
        let Some(pending) = self.pending.front() else {
            return Ok(None);
        };
        let slot = pending.slot;

        // The map callback fires only while the device is being polled, so
        // drive it with non-blocking polls instead of a blocking wait.
        let map_result = loop {
            match pending.receiver.try_recv() {
                Ok(result) => break result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    gpu.device.poll(wgpu::PollType::Poll)?;
                    std::thread::sleep(Duration::from_micros(200));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The callback always sends exactly once, so a disconnect
                    // means the callback panicked; surface it as an error.
                    anyhow::bail!("readback map callback never produced a result");
                }
            }
        };
        map_result?;

        let buffer_slice = self.output_buffers[slot].slice(..);
        let data = buffer_slice.get_mapped_range()?;

        out.clear();
        out.reserve((self.unpadded_bytes_per_row * self.height) as usize);
        for row in 0..self.height {
            let start = (row * self.padded_bytes_per_row) as usize;
            let end = start + self.unpadded_bytes_per_row as usize;
            out.extend_from_slice(&data[start..end]);
        }

        drop(data);
        self.output_buffers[slot].unmap();

        Ok(self.pending.pop_front().map(|p| p.stamp))
    }
}

impl FrameRenderer {
    /// Queue a readback of this renderer's own render target. A borrowed
    /// variant exists in [`Self::queue_readback`], but calling that with
    /// `&self.render_texture` would keep an immutable borrow alive during the
    /// internal `&mut` bookkeeping.
    pub fn queue_render_target_readback(
        &mut self,
        gpu: &GpuContext,
        stamp: FrameStamp,
    ) -> Result<()> {
        // Deref the field before the &mut call so the borrow checker sees the
        // two operations as sequential.
        let texture = self.render_texture.clone();
        self.queue_readback(gpu, &texture, stamp)
    }
}
