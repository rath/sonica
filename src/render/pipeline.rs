use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use wgpu;

/// Run GPU-object creation under a validation error scope.
///
/// WGSL compile errors and pipeline-layout mismatches surface as *uncaptured*
/// wgpu errors that abort the process with no context about which template or
/// effect caused them. Wrapping the creation in an error scope turns them back
/// into a contextual [`Result`] instead.
pub(crate) fn with_error_scope<T>(
    device: &wgpu::Device,
    context: impl FnOnce() -> String,
    build: impl FnOnce() -> T,
) -> Result<T> {
    let guard = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = build();
    match pollster::block_on(guard.pop()) {
        Some(err) => Err(anyhow::anyhow!("{}: {err}", context())),
        None => Ok(result),
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FrameUniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    pub frame: u32,
    pub fps: f32,
    pub duration: f32,
    pub rms: f32,
    pub spectral_centroid: f32,
    pub spectral_flux: f32,
    pub beat_intensity: f32,
    pub beat_phase: f32,
    pub is_beat: f32,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub _padding: f32,
}

impl Default for FrameUniforms {
    fn default() -> Self {
        Self {
            resolution: [1920.0, 1080.0],
            time: 0.0,
            frame: 0,
            fps: 30.0,
            duration: 0.0,
            rms: 0.0,
            spectral_centroid: 0.0,
            spectral_flux: 0.0,
            beat_intensity: 0.0,
            beat_phase: 0.0,
            is_beat: 0.0,
            bass: 0.0,
            mid: 0.0,
            high: 0.0,
            _padding: 0.0,
        }
    }
}

pub struct RenderPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl RenderPipeline {
    pub fn new(device: &wgpu::Device, shader_source: &str, texture_format: wgpu::TextureFormat) -> Result<Self> {
        let context = || {
            let head = shader_source
                .lines()
                .find(|line| line.starts_with("const PARAM_"))
                .map(|line| format!(" ({}..)", line.split('=').next().unwrap_or(line)))
                .unwrap_or_default();
            format!("Template shader failed to compile{head}")
        };
        with_error_scope(device, context, || {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("template_shader"),
                    source: wgpu::ShaderSource::Wgsl(shader_source.into()),
                });

            let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("main_bind_group_layout"),
                entries: &[
                    // @binding(0): FrameUniforms
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // @binding(1): FFT bins (storage)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // @binding(2): waveform samples (storage)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("render_pipeline_layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("main_render_pipeline"),
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
                        format: texture_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

            Self {
                pipeline,
                bind_group_layout,
            }
        })
    }
}

pub struct ComputePipelineWrapper {
    #[allow(dead_code)]
    pub pipeline: wgpu::ComputePipeline,
    #[allow(dead_code)]
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl ComputePipelineWrapper {
    pub fn new(device: &wgpu::Device, shader_source: &str) -> Result<Self> {
        let shader_source_owned = shader_source.to_string();
        with_error_scope(
            device,
            || "Template compute shader failed to compile".to_string(),
            || {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("compute_shader"),
                    source: wgpu::ShaderSource::Wgsl(shader_source_owned.into()),
                });

                let bind_group_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("compute_bind_group_layout"),
                        entries: &[
                            // @binding(0): FrameUniforms (uniform)
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::COMPUTE,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Uniform,
                                    has_dynamic_offset: false,
                                    min_binding_size: None,
                                },
                                count: None,
                            },
                            // @binding(1): FFT bins (storage, read-only)
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::COMPUTE,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                                    has_dynamic_offset: false,
                                    min_binding_size: None,
                                },
                                count: None,
                            },
                            // @binding(2): Waveform (storage, read-only)
                            wgpu::BindGroupLayoutEntry {
                                binding: 2,
                                visibility: wgpu::ShaderStages::COMPUTE,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                                    has_dynamic_offset: false,
                                    min_binding_size: None,
                                },
                                count: None,
                            },
                            // @binding(3): Output (storage, read-write)
                            wgpu::BindGroupLayoutEntry {
                                binding: 3,
                                visibility: wgpu::ShaderStages::COMPUTE,
                                ty: wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                                    has_dynamic_offset: false,
                                    min_binding_size: None,
                                },
                                count: None,
                            },
                        ],
                    });

                let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("compute_pipeline_layout"),
                    bind_group_layouts: &[Some(&bind_group_layout)],
                    immediate_size: 0,
                });

                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("compute_pipeline"),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some("cs_main"),
                    compilation_options: Default::default(),
                    cache: None,
                });

                Self {
                    pipeline,
                    bind_group_layout,
                }
            },
        )
    }
}
