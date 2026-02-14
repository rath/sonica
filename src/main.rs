mod cli;
mod config;
mod audio;
mod render;
mod templates;
mod encode;
#[cfg(feature = "subtitles")]
mod subtitle;

use anyhow::{Context, Result};
use bytemuck;
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use wgpu;

use cli::Cli;
use render::gpu::GpuContext;
use render::pipeline::{ComputePipelineWrapper, FrameUniforms, RenderPipeline};
use render::frame::{FrameRenderer, TEXTURE_FORMAT};
use render::postprocess::PostProcessChain;
use render::text::{load_font_from_url, TextOverlay};
use encode::ffmpeg::FfmpegEncoder;
use audio::features::SmoothedFrame;
use templates::loader;

struct TemplateSlot {
    pipeline: RenderPipeline,
    bind_group: wgpu::BindGroup,
    compute_pipeline: Option<ComputePipelineWrapper>,
    name: String,
    end_frame: usize,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let mut cli = Cli::parse();

    // Load config: explicit --config path, or auto-detect sonica.toml / global config
    let config_path = cli.config.clone().or_else(|| {
        let local = std::path::PathBuf::from("sonica.toml");
        if local.exists() {
            return Some(local);
        }
        if let Some(home) = dirs::home_dir() {
            let xdg = home.join(".config").join("sonica").join("config.toml");
            if xdg.exists() {
                return Some(xdg);
            }
        }
        if let Some(config_dir) = dirs::config_dir() {
            let platform = config_dir.join("sonica").join("config.toml");
            if platform.exists() {
                return Some(platform);
            }
        }
        None
    });
    if let Some(ref path) = config_path {
        if let Some(cfg) = config::load_config(path) {
            log::info!("Loaded config from {}", path.display());
            // Merge: config values apply only when CLI is at its default
            if cli.width == 1920 { cli.width = cfg.output.width; }
            if cli.height == 1080 { cli.height = cfg.output.height; }
            if cli.fps == 30 { cli.fps = cfg.output.fps; }
            if cli.crf == 18 { cli.crf = cfg.output.crf; }
            if cli.codec == "libx264" { cli.codec = cfg.output.codec; }
            if cli.smoothing == 0.85 { cli.smoothing = cfg.audio.smoothing; }
            if cli.effects.is_empty() && !cfg.effects.is_empty() {
                cli.effects = cfg.effects;
            }
            if cli.font.is_none() {
                cli.font = cfg.output.font;
            }
            if cli.font_url.is_none() {
                cli.font_url = cfg.output.font_url;
            }
            if cli.whisper_model == "base" {
                cli.whisper_model = cfg.subtitle.whisper_model;
            }
            if cli.subtitle_lang.is_none() {
                cli.subtitle_lang = cfg.subtitle.language;
            }
            if cli.subtitle_font_size == 48.0 {
                cli.subtitle_font_size = cfg.subtitle.font_size;
            }
            if cli.subtitle_max_chars == 42 {
                cli.subtitle_max_chars = cfg.subtitle.max_chars_per_line;
            }
        } else {
            log::warn!("Failed to load config from {}", path.display());
        }
    }

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

    // 1b. Transcribe audio (if subtitles enabled)
    #[cfg(feature = "subtitles")]
    let subtitle_cues = if cli.subtitles {
        log::info!("Transcribing audio for subtitles...");
        let model_path = subtitle::model::resolve_model_path(&cli.whisper_model)?;
        let transcriber = subtitle::transcribe::WhisperTranscriber::new(
            &model_path,
            cli.subtitle_lang.as_deref(),
        )?;
        let words = transcriber.transcribe(&audio_data.samples, audio_data.sample_rate)?;
        log::info!("Whisper returned {} word segments:", words.len());
        for (i, w) in words.iter().enumerate() {
            log::info!("  [{:3}] {:.2}s - {:.2}s  {:?}", i, w.start_time, w.end_time, w.text);
        }
        let cues = subtitle::cue::group_words(words, cli.subtitle_max_chars);
        log::info!("Grouped into {} subtitle cues:", cues.len());
        for (i, c) in cues.iter().enumerate() {
            log::info!("  [{:3}] {:.2}s - {:.2}s  {:?}", i, c.start_time, c.end_time, c.text);
        }
        Some(cues)
    } else {
        None
    };

    #[cfg(not(feature = "subtitles"))]
    if cli.subtitles {
        anyhow::bail!(
            "Subtitle support requires the 'subtitles' feature. \
             Rebuild with: cargo build --features subtitles"
        );
    }

    // 2. Analyze audio (3-pass pipeline)
    log::info!("Analyzing audio...");
    let (global, frames) = audio::analysis::analyze(&audio_data, cli.fps, cli.smoothing)?;

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

    // 6. Parse template parameter overrides
    let param_overrides: HashMap<String, String> = cli
        .params
        .iter()
        .filter_map(|s| {
            let mut parts = s.splitn(2, '=');
            let key = parts.next()?.to_string();
            let val = parts.next()?.to_string();
            Some((key, val))
        })
        .collect();

    // 7. Build per-template pipelines and bind groups, assign frame ranges
    let num_templates = template_names.len();
    let frames_per_template = total_frames / num_templates;
    let mut slots: Vec<TemplateSlot> = Vec::with_capacity(num_templates);

    for (i, name) in template_names.iter().enumerate() {
        let tmpl = loader::load_template(name)?;
        let shader_src = loader::inject_params(&tmpl.fragment_shader, &tmpl.manifest, &param_overrides);
        let pipeline = RenderPipeline::new(&gpu.device, &shader_src, TEXTURE_FORMAT)?;

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

        let compute_pipeline = if let Some(ref compute_src) = tmpl.compute_shader {
            let compute_src = loader::inject_params(compute_src, &tmpl.manifest, &param_overrides);
            Some(ComputePipelineWrapper::new(&gpu.device, &compute_src)?)
        } else {
            None
        };

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
            compute_pipeline,
            name: tmpl.manifest.display_name.clone(),
            end_frame,
        });
    }

    // 7b. Post-processing chain
    let pp_chain = PostProcessChain::new(&gpu.device, cli.width, cli.height, &effects)?;
    if pp_chain.has_effects() {
        log::info!("Post-processing effects: {:?}", effects);
    }

    // 8. Start FFmpeg encoder
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

    // 8. Text overlay
    let font_bytes = if let Some(ref font_url) = cli.font_url {
        match load_font_from_url(font_url) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log::warn!("Failed to load font from URL: {}", err);
                None
            }
        }
    } else {
        None
    };

    let text_overlay = if cli.title.is_some() || cli.show_time {
        let shorter = cli.width.min(cli.height) as f32;
        let font_size = (shorter * 0.046).max(24.0);
        Some(TextOverlay::new(
            font_size,
            cli.font.as_deref(),
            font_bytes.as_deref(),
        ))
    } else {
        None
    };

    // 8b. Subtitle renderer
    #[cfg(feature = "subtitles")]
    let subtitle_renderer = subtitle_cues.map(|cues| {
        let sub_overlay = TextOverlay::new(
            cli.subtitle_font_size,
            cli.font.as_deref(),
            font_bytes.as_deref(),
        );
        subtitle::render::SubtitleRenderer::new(cues, sub_overlay, cli.subtitle_max_chars)
    });

    // 9. Render loop
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

        // Compute dispatch (if template has a compute shader)
        if let Some(ref _compute) = slot.compute_pipeline {
            // TODO: create compute bind group, dispatch, and submit
            // Requires output buffer binding and workgroup size configuration
        }

        // Render
        let mut pixels = if pp_chain.has_effects() {
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

        // Text overlay compositing
        if let Some(ref overlay) = text_overlay {
            let color = [255u8, 255, 255, 220];
            let shorter = cli.width.min(cli.height) as f32;
            let margin = (shorter * 0.07) as u32;

            if let Some(ref title) = cli.title {
                let tw = overlay.measure_width(title);
                let tx = cli.width - margin - tw;
                let ty = margin;
                overlay.composite(&mut pixels, cli.width, cli.height, title, tx, ty, color);
            }

            if cli.show_time {
                let total_secs = frame.time as u64;
                let centis = ((frame.time - total_secs as f32) * 100.0) as u64;
                let time_str = if total_secs >= 3600 {
                    format!("{:02}:{:02}:{:02}.{:02}", total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60, centis)
                } else {
                    format!("{:02}:{:02}.{:02}", total_secs / 60, total_secs % 60, centis)
                };
                let tw = overlay.measure_width(&time_str);
                let tx = cli.width - margin - tw;
                let ty = cli.height - margin - overlay.line_height();
                overlay.composite(&mut pixels, cli.width, cli.height, &time_str, tx, ty, color);
            }
        }

        // Subtitle overlay
        #[cfg(feature = "subtitles")]
        if let Some(ref sub) = subtitle_renderer {
            sub.render_frame(&mut pixels, cli.width, cli.height, frame.time);
        }

        encoder.write_frame(&pixels)?;
        pb.set_position(frame_idx as u64 + 1);
    }

    pb.finish_with_message("Rendering complete");

    // 10. Finish encoding
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
