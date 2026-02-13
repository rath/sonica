mod cli;
#[allow(dead_code)]
mod config;
mod audio;
mod render;
mod templates;
mod encode;

use anyhow::{Context, Result};
use bytemuck;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use wgpu;

use cli::Cli;
use render::gpu::GpuContext;
use render::pipeline::{FrameUniforms, RenderPipeline};
use render::frame::{FrameRenderer, TEXTURE_FORMAT};
use render::postprocess::PostProcessChain;
use encode::ffmpeg::FfmpegEncoder;
use audio::features::SmoothedFrame;
use templates::loader;

struct TemplateSlot {
    pipeline: RenderPipeline,
    bind_group: wgpu::BindGroup,
    name: String,
    start_frame: usize,
    end_frame: usize,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let cli = Cli::parse();

    // List templates mode
    if cli.list_templates {
        let templates = loader::list_templates()?;
        println!("Available templates:");
        for name in &templates {
            if let Ok(t) = loader::load_template(name) {
                println!("  {:<20} {}", t.manifest.display_name, t.manifest.description);
            } else {
                println!("  {}", name);
            }
        }
        return Ok(());
    }

    let input = cli.input.as_ref().context("Input audio file is required")?;
    if !input.exists() {
        anyhow::bail!("Input file not found: {}", input.display());
    }

    log::info!("sonica - GPU-accelerated audio visualizer");
    log::info!("Input: {}", input.display());
    log::info!("Output: {}", cli.output.display());
    log::info!("Template: {}", cli.template);
    log::info!("Resolution: {}x{} @ {}fps", cli.width, cli.height, cli.fps);

    // 1. Decode audio
    log::info!("Decoding audio...");
    let audio_data = audio::decode::decode_audio(input)?;

    // 2. Analyze audio (3-pass pipeline)
    log::info!("Analyzing audio...");
    let (global, frames) = audio::analysis::analyze(&audio_data, cli.fps)?;

    let total_frames = frames.len();
    log::info!("Total frames: {}, Duration: {:.1}s", total_frames, global.duration);

    // 3. Resolve template names
    let template_names: Vec<String> = if cli.template == "all" {
        loader::list_templates()?
    } else {
        vec![cli.template.clone()]
    };

    if template_names.is_empty() {
        anyhow::bail!("No templates found");
    }

    // Determine effects: "none" disables all, CLI > template defaults
    let first_template = loader::load_template(&template_names[0])?;
    let effects = if cli.effects.iter().any(|e| e == "none") {
        Vec::new()
    } else if cli.effects.is_empty() {
        first_template.manifest.default_effects.clone()
    } else {
        cli.effects.clone()
    };
    drop(first_template);

    // 4. Initialize GPU
    log::info!("Initializing GPU...");
    let gpu = GpuContext::new()?;
    let frame_renderer = FrameRenderer::new(&gpu, cli.width, cli.height);

    // 5. Create shared GPU buffers
    let uniform_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniform_buffer"),
        size: std::mem::size_of::<FrameUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let num_fft_bins = if frames.is_empty() { 1024 } else { frames[0].fft_bins.len() };
    let fft_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fft_buffer"),
        size: (num_fft_bins * std::mem::size_of::<f32>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let num_waveform = if frames.is_empty() { 512 } else { frames[0].waveform.len() };
    let waveform_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("waveform_buffer"),
        size: (num_waveform * std::mem::size_of::<f32>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // 6. Build per-template pipelines and bind groups, assign frame ranges
    let num_templates = template_names.len();
    let frames_per_template = total_frames / num_templates;
    let mut slots: Vec<TemplateSlot> = Vec::with_capacity(num_templates);

    for (i, name) in template_names.iter().enumerate() {
        let tmpl = loader::load_template(name)?;
        let pipeline = RenderPipeline::new(&gpu.device, &tmpl.fragment_shader, TEXTURE_FORMAT)?;

        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("main_bind_group"),
            layout: &pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: fft_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: waveform_buffer.as_entire_binding(),
                },
            ],
        });

        let start_frame = i * frames_per_template;
        let end_frame = if i == num_templates - 1 {
            total_frames
        } else {
            (i + 1) * frames_per_template
        };

        log::info!(
            "Template [{}]: {} (frames {}-{})",
            i, tmpl.manifest.display_name, start_frame, end_frame - 1
        );

        slots.push(TemplateSlot {
            pipeline,
            bind_group,
            name: tmpl.manifest.display_name.clone(),
            start_frame,
            end_frame,
        });
    }

    // 6b. Post-processing chain
    let pp_chain = PostProcessChain::new(&gpu.device, cli.width, cli.height, &effects)?;
    if pp_chain.has_effects() {
        log::info!("Post-processing effects: {:?}", effects);
    }

    // 7. Start FFmpeg encoder
    log::info!("Starting FFmpeg encoder...");
    let mut encoder = FfmpegEncoder::new(
        &cli.output,
        input,
        cli.width,
        cli.height,
        cli.fps,
        &cli.codec,
        &cli.pix_fmt,
        cli.crf,
        cli.bitrate.as_deref(),
    )?;

    // 8. Render loop
    let pb = ProgressBar::new(total_frames as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} frames ({eta} remaining)")
            .unwrap()
            .progress_chars("=>-"),
    );

    let mut current_slot_idx = 0;

    for (frame_idx, frame) in frames.iter().enumerate() {
        // Advance to the correct template slot
        while current_slot_idx + 1 < slots.len()
            && frame_idx >= slots[current_slot_idx].end_frame
        {
            current_slot_idx += 1;
            log::info!("Switching to template: {}", slots[current_slot_idx].name);
        }
        let slot = &slots[current_slot_idx];

        // Update uniforms
        let uniforms = build_uniforms(frame, frame_idx as u32, cli.width, cli.height, cli.fps, global.duration);
        gpu.queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        gpu.queue.write_buffer(&fft_buffer, 0, bytemuck::cast_slice(&frame.fft_bins));
        gpu.queue.write_buffer(&waveform_buffer, 0, bytemuck::cast_slice(&frame.waveform));

        // Render
        let pixels = if pp_chain.has_effects() {
            frame_renderer.render_and_readback(&gpu, &slot.pipeline.pipeline, &slot.bind_group)?;
            let final_texture = pp_chain.run(
                &gpu.device,
                &gpu.queue,
                &frame_renderer.render_texture,
                frame.time,
            );
            frame_renderer.readback_texture(&gpu, final_texture)?
        } else {
            frame_renderer.render_and_readback(&gpu, &slot.pipeline.pipeline, &slot.bind_group)?
        };

        encoder.write_frame(&pixels)?;
        pb.set_position(frame_idx as u64 + 1);
    }

    pb.finish_with_message("Rendering complete");

    // 9. Finish encoding
    log::info!("Finishing encoding...");
    encoder.finish()?;

    log::info!("Done! Output: {}", cli.output.display());
    Ok(())
}

fn build_uniforms(
    frame: &SmoothedFrame,
    frame_idx: u32,
    width: u32,
    height: u32,
    fps: u32,
    duration: f32,
) -> FrameUniforms {
    FrameUniforms {
        resolution: [width as f32, height as f32],
        time: frame.time,
        frame: frame_idx,
        fps: fps as f32,
        duration,
        rms: frame.rms,
        spectral_centroid: frame.spectral_centroid,
        spectral_flux: frame.spectral_flux,
        beat_intensity: frame.beat_intensity,
        beat_phase: frame.beat_phase,
        is_beat: if frame.is_beat { 1.0 } else { 0.0 },
        bass: frame.bass,
        mid: frame.mid,
        high: frame.high,
        _padding: 0.0,
    }
}
