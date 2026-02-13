use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use wgpu;

use super::frame::TEXTURE_FORMAT;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PostProcessUniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub intensity: f32,
}

pub struct PostProcessPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    name: String,
}

pub struct PostProcessChain {
    passes: Vec<PostProcessPass>,
    ping_texture: wgpu::Texture,
    pong_texture: wgpu::Texture,
    ping_view: wgpu::TextureView,
    pong_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl PostProcessChain {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        effects: &[String],
    ) -> Result<Self> {
        let make_texture = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TEXTURE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };

        let ping_texture = make_texture("pp_ping");
        let pong_texture = make_texture("pp_pong");
        let ping_view = ping_texture.create_view(&Default::default());
        let pong_view = pong_texture.create_view(&Default::default());

        let mut passes = Vec::new();

        // Expand presets
        let expanded = expand_effects(effects);

        for effect_name in &expanded {
            if let Some(shader_src) = get_effect_shader(effect_name) {
                let pass = PostProcessPass::new(device, &shader_src, effect_name)?;
                passes.push(pass);
            } else {
                log::warn!("Unknown effect: {}", effect_name);
            }
        }

        Ok(Self {
            passes,
            ping_texture,
            pong_texture,
            ping_view,
            pong_view,
            width,
            height,
        })
    }

    pub fn has_effects(&self) -> bool {
        !self.passes.is_empty()
    }

    /// Run the post-processing chain.
    /// Input texture is copied to ping, then ping-pong through passes.
    /// Returns the view of the final output texture.
    pub fn run<'a>(
        &'a self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input_texture: &'a wgpu::Texture,
        time: f32,
    ) -> &'a wgpu::Texture {
        if self.passes.is_empty() {
            return input_texture;
        }

        // Copy input to ping
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pp_copy_encoder"),
        });
        encoder.copy_texture_to_texture(
            input_texture.as_image_copy(),
            self.ping_texture.as_image_copy(),
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let textures = [&self.ping_texture, &self.pong_texture];
        let views = [&self.ping_view, &self.pong_view];

        for (i, pass) in self.passes.iter().enumerate() {
            let src_idx = i % 2;
            let dst_idx = (i + 1) % 2;

            let uniforms = PostProcessUniforms {
                resolution: [self.width as f32, self.height as f32],
                time,
                intensity: 1.0,
            };
            queue.write_buffer(&pass.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pp_bind_group"),
                layout: &pass.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pass.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(views[src_idx]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&pass.sampler),
                    },
                ],
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pp_encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pp_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: views[dst_idx],
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

                render_pass.set_pipeline(&pass.pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            queue.submit(std::iter::once(encoder.finish()));
        }

        // Return the texture that has the final result
        let final_idx = self.passes.len() % 2;
        textures[final_idx]
    }
}

impl PostProcessPass {
    fn new(device: &wgpu::Device, shader_source: &str, name: &str) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pp_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pp_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pp_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pp_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TEXTURE_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pp_uniform_buffer"),
            size: std::mem::size_of::<PostProcessUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            uniform_buffer,
            name: name.to_string(),
        })
    }
}

fn expand_effects(effects: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for e in effects {
        match e.as_str() {
            "none" => return Vec::new(),
            "crt" => {
                result.extend_from_slice(&[
                    "crt_scanlines".into(),
                    "chromatic_aberration".into(),
                    "vignette".into(),
                    "film_grain".into(),
                    "color_grading".into(),
                ]);
            }
            "all" => {
                result.extend_from_slice(&[
                    "bloom".into(),
                    "crt_scanlines".into(),
                    "chromatic_aberration".into(),
                    "vignette".into(),
                    "film_grain".into(),
                    "color_grading".into(),
                ]);
            }
            other => result.push(other.to_string()),
        }
    }
    result
}

fn get_effect_shader(name: &str) -> Option<String> {
    // Shared fullscreen VS + postprocess-specific uniform struct used in all effects
    let common_header = r#"
struct PPUniforms {
    resolution: vec2<f32>,
    time: f32,
    intensity: f32,
};

@group(0) @binding(0) var<uniform> pp: PPUniforms;
@group(0) @binding(1) var input_tex: texture_2d<f32>;
@group(0) @binding(2) var input_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}
"#;

    let fragment = match name {
        "bloom" => r#"
fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel_size = 1.0 / pp.resolution;
    var color = textureSample(input_tex, input_sampler, in.uv).rgb;

    // Extract bright areas and blur
    var bloom_color = vec3<f32>(0.0);
    let radius = 4;
    var total_weight = 0.0;

    for (var x = -radius; x <= radius; x++) {
        for (var y = -radius; y <= radius; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size * 2.0;
            let sample_color = textureSample(input_tex, input_sampler, in.uv + offset).rgb;
            let lum = luminance(sample_color);
            let threshold = 0.6;
            if lum > threshold {
                let w = 1.0 / (1.0 + f32(x * x + y * y));
                bloom_color += sample_color * w;
                total_weight += w;
            }
        }
    }

    if total_weight > 0.0 {
        bloom_color /= total_weight;
    }

    color += bloom_color * 0.4 * pp.intensity;
    return vec4<f32>(color, 1.0);
}
"#,
        "chromatic_aberration" => r#"
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let dir = in.uv - center;
    let dist = length(dir);
    let offset = dir * dist * 0.008 * pp.intensity;

    let r = textureSample(input_tex, input_sampler, in.uv + offset).r;
    let g = textureSample(input_tex, input_sampler, in.uv).g;
    let b = textureSample(input_tex, input_sampler, in.uv - offset).b;

    return vec4<f32>(r, g, b, 1.0);
}
"#,
        "vignette" => r#"
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(input_tex, input_sampler, in.uv).rgb;

    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(in.uv, center) * 1.4142;
    let vignette = 1.0 - smoothstep(0.4, 1.2, dist) * 0.7 * pp.intensity;
    color *= vignette;

    return vec4<f32>(color, 1.0);
}
"#,
        "film_grain" => r#"
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(input_tex, input_sampler, in.uv).rgb;

    let noise = hash(in.uv * pp.resolution + vec2<f32>(pp.time * 1000.0, pp.time * 573.0));
    let grain = (noise - 0.5) * 0.08 * pp.intensity;
    color += vec3<f32>(grain);

    return vec4<f32>(color, 1.0);
}
"#,
        "crt_scanlines" => r#"
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Barrel distortion
    let center = in.uv - vec2<f32>(0.5, 0.5);
    let dist2 = dot(center, center);
    let barrel = 0.15 * pp.intensity;
    let distorted_uv = in.uv + center * dist2 * barrel;

    // Check bounds
    if distorted_uv.x < 0.0 || distorted_uv.x > 1.0 || distorted_uv.y < 0.0 || distorted_uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    var color = textureSample(input_tex, input_sampler, distorted_uv).rgb;

    // Scanlines
    let scanline_freq = pp.resolution.y * 0.5;
    let scanline = sin(distorted_uv.y * scanline_freq * 3.14159) * 0.5 + 0.5;
    let scanline_intensity = 0.15 * pp.intensity;
    color *= 1.0 - scanline_intensity * (1.0 - scanline);

    // Phosphor dot pattern
    let pixel_pos = distorted_uv * pp.resolution;
    let dot = sin(pixel_pos.x * 3.14159 * 1.0) * 0.5 + 0.5;
    color *= 0.95 + 0.05 * dot;

    return vec4<f32>(color, 1.0);
}
"#,
        "color_grading" => r#"
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(input_tex, input_sampler, in.uv).rgb;

    // Contrast boost
    let contrast = 1.15;
    color = (color - 0.5) * contrast + 0.5;

    // Slight warm tint
    color.r *= 1.02;
    color.b *= 0.98;

    // Saturation boost
    let gray = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let saturation = 1.1;
    color = mix(vec3<f32>(gray), color, saturation);

    // Clamp
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));

    return vec4<f32>(color, 1.0);
}
"#,
        _ => return None,
    };

    Some(format!("{}{}", common_header, fragment))
}
