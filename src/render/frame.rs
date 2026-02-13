use anyhow::Result;
use wgpu;

use super::gpu::GpuContext;

pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct FrameRenderer {
    pub render_texture: wgpu::Texture,
    pub render_texture_view: wgpu::TextureView,
    pub output_buffer: wgpu::Buffer,
    pub width: u32,
    pub height: u32,
    pub padded_bytes_per_row: u32,
    pub unpadded_bytes_per_row: u32,
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
        let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) / align * align;

        let output_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output_buffer"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            render_texture,
            render_texture_view,
            output_buffer,
            width,
            height,
            padded_bytes_per_row,
            unpadded_bytes_per_row,
        }
    }

    pub fn render_and_readback(
        &self,
        gpu: &GpuContext,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
    ) -> Result<Vec<u8>> {
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.render_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..3, 0..1); // fullscreen triangle
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.render_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.output_buffer,
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

        // Read back
        let buffer_slice = self.output_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        receiver.recv()??;

        let data = buffer_slice.get_mapped_range();

        // Strip row padding
        let mut pixels = Vec::with_capacity((self.unpadded_bytes_per_row * self.height) as usize);
        for row in 0..self.height {
            let start = (row * self.padded_bytes_per_row) as usize;
            let end = start + self.unpadded_bytes_per_row as usize;
            pixels.extend_from_slice(&data[start..end]);
        }

        drop(data);
        self.output_buffer.unmap();

        Ok(pixels)
    }

    /// Read back pixels from an arbitrary texture (e.g. post-processing output)
    pub fn readback_texture(
        &self,
        gpu: &GpuContext,
        texture: &wgpu::Texture,
    ) -> Result<Vec<u8>> {
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
                buffer: &self.output_buffer,
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

        let buffer_slice = self.output_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        receiver.recv()??;

        let data = buffer_slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((self.unpadded_bytes_per_row * self.height) as usize);
        for row in 0..self.height {
            let start = (row * self.padded_bytes_per_row) as usize;
            let end = start + self.unpadded_bytes_per_row as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
        drop(data);
        self.output_buffer.unmap();

        Ok(pixels)
    }
}
