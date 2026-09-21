mod cli;
mod config;
mod audio;
mod render;
mod templates;
mod encode;
#[cfg(feature = "subtitles")]
mod subtitle;

use anyhow::{Context, Result};
use clap::parser::ValueSource;
use clap::{ ArgMatches, CommandFactory, FromArgMatches };
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;

use cli::Cli;
use render::gpu::GpuContext;
use render::pipeline::{ComputePipelineWrapper, FrameUniforms, RenderPipeline};
use render::frame::{FrameRenderer, TEXTURE_FORMAT};
use render::postprocess::PostProcessChain;
use render::text::{load_font_from_url, TextOverlay};
use encode::ffmpeg::FfmpegEncoder;
use audio::features::SmoothedFrame;
use templates::loader;

/// True when the user passed the flag on the command line, so a config file
/// must not override it.
fn from_command_line(matches: &ArgMatches, key: &str) -> bool {
    matches.value_source(key) == Some(ValueSource::CommandLine)
}

/// Template name paired with its manifest description, falling back to an empty
/// description when a manifest is unreadable (mirrors `--list-templates`).
fn template_catalog() -> Vec<(String, String)> {
    loader::list_templates()
        .unwrap_or_default()
        .into_iter()
        .map(|name| {
            let description = loader::load_template(&name)
                .map(|t| t.manifest.description)
                .unwrap_or_default();
            (name, description)
        })
        .collect()
}

/// Built at runtime rather than hardcoded, so `--help` also shows any custom
/// templates found on the filesystem and can never drift from what loads.
fn template_long_help() -> String {
    let mut help = String::from(
        "Visual template -- the picture itself, not a post-processing effect.\n\n\
         Available templates:\n",
    );
    for (name, description) in template_catalog() {
        help.push_str(&format!("  {name:<20} {description}\n"));
    }
    help.push_str("\nCustom templates in ./templates/<name>/ are picked up automatically.");
    help
}

/// Generated from the effect registry in `postprocess`, so a new shader shows
/// up here as soon as it is registered.
fn effects_long_help() -> String {
    let mut help = String::from(
        "Post-processing applied on top of the template, comma-separated and\n\
         run in the order given. Omit to use the template's own defaults.\n\n\
         Available effects:\n",
    );
    for (name, description) in render::postprocess::EFFECTS {
        help.push_str(&format!("  {name:<22} {description}\n"));
    }
    help.push_str("\nPresets:\n");
    for (name, expansion) in render::postprocess::EFFECT_PRESETS {
        help.push_str(&format!("  {name:<22} {expansion}\n"));
    }
    help.push_str("\nExample: --effects bloom,vignette");
    help
}

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

    // Attach the runtime-generated value lists before parsing so `--help`
    // documents the templates and effects this binary actually supports.
    let command = Cli::command()
        .mut_arg("template", |arg| arg.long_help(template_long_help()))
        .mut_arg("effects", |arg| arg.long_help(effects_long_help()));
    let matches = command.get_matches();
    let mut cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    };

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
        let cfg = config::load_config(path).map_err(|err| {
            anyhow::anyhow!("Failed to load config from {}: {err:#}", path.display())
        })?;
        log::info!("Loaded config from {}", path.display());

        // Config values apply only to flags the user did not pass explicitly;
        // comparing against hardcoded defaults would override an explicit
        // `--width 1920` whenever the config sets 1280.
        if !from_command_line(&matches, "width") {
            cli.width = cfg.output.width;
        }
        if !from_command_line(&matches, "height") {
            cli.height = cfg.output.height;
        }
        if !from_command_line(&matches, "fps") {
            cli.fps = cfg.output.fps;
        }
        if !from_command_line(&matches, "crf") {
            cli.crf = cfg.output.crf;
        }
        if !from_command_line(&matches, "codec") {
            cli.codec = cfg.output.codec;
        }
        if !from_command_line(&matches, "smoothing") {
            cli.smoothing = cfg.audio.smoothing;
        }
        if !from_command_line(&matches, "effects") && !cfg.effects.is_empty() {
            cli.effects = cfg.effects;
        }
        if !from_command_line(&matches, "font") {
            cli.font = cfg.output.font;
        }
        if !from_command_line(&matches, "font_url") {
            cli.font_url = cfg.output.font_url;
        }
        if !from_command_line(&matches, "font_family") {
            cli.font_family = cfg.output.font_family;
        }
        if !from_command_line(&matches, "whisper_model") {
            cli.whisper_model = cfg.subtitle.whisper_model;
        }
        if !from_command_line(&matches, "subtitle_lang") {
            cli.subtitle_lang = cfg.subtitle.language;
        }
        if !from_command_line(&matches, "subtitle_font_size") {
            cli.subtitle_font_size = cfg.subtitle.font_size;
        }
        if !from_command_line(&matches, "subtitle_max_chars") {
            cli.subtitle_max_chars = cfg.subtitle.max_chars_per_line;
        }
        if !from_command_line(&matches, "subtitle_gap") {
            cli.subtitle_gap = cfg.subtitle.gap;
        }
        if !from_command_line(&matches, "subtitle_min_duration") {
            cli.subtitle_min_duration = cfg.subtitle.min_duration;
        }
        if !from_command_line(&matches, "subtitle_font") {
            cli.subtitle_font = cfg.subtitle.font;
        }
        if !from_command_line(&matches, "subtitle_font_url") {
            cli.subtitle_font_url = cfg.subtitle.font_url;
        }
        if !from_command_line(&matches, "subtitle_font_family") {
            cli.subtitle_font_family = cfg.subtitle.font_family;
        }
        if !from_command_line(&matches, "subtitle_background_opacity") {
            cli.subtitle_background_opacity = cfg.subtitle.background_opacity;
        }
        if !from_command_line(&matches, "subtitle_dim_opacity") {
            cli.subtitle_dim_opacity = cfg.subtitle.dim_opacity;
        }
        if !from_command_line(&matches, "subtitle_text_color") {
            cli.subtitle_text_color = cfg.subtitle.text_color;
        }
        if !from_command_line(&matches, "subtitle_highlight_color") {
            cli.subtitle_highlight_color = cfg.subtitle.highlight_color;
        }
        if !from_command_line(&matches, "subtitle_outline_color") {
            cli.subtitle_outline_color = cfg.subtitle.outline_color;
        }
        if !from_command_line(&matches, "subtitle_outline_width") {
            cli.subtitle_outline_width = cfg.subtitle.outline_width;
        }
        if !from_command_line(&matches, "subtitle_margin_bottom") {
            cli.subtitle_margin_bottom = cfg.subtitle.margin_bottom;
        }
        if !from_command_line(&matches, "no_subtitle_karaoke") && !cfg.subtitle.karaoke {
            cli.no_subtitle_karaoke = true;
        }
    }

    // Validate once, after the merge, so config-sourced values get the same
    // checks as CLI-passed ones.
    cli::validate(&cli)?;

    // List templates mode. Prints the name you pass to -t, not the manifest's
    // display name, which is not a valid value for the flag.
    if cli.list_templates {
        println!("Available templates (pass with -t/--template):");
        for (name, description) in template_catalog() {
            println!("  {name:<20} {description}");
        }
        return Ok(());
    }

    // List effects mode
    if cli.list_effects {
        println!("Available effects (pass with --effects, comma-separated):");
        for (name, description) in render::postprocess::EFFECTS {
            println!("  {name:<22} {description}");
        }
        println!("\nPresets:");
        for (name, expansion) in render::postprocess::EFFECT_PRESETS {
            println!("  {name:<22} {expansion}");
        }
        return Ok(());
    }

    // Fail on unknown effect names now rather than warning mid-render and
    // producing a video that is silently missing the effect.
    render::postprocess::validate_effects(&cli.effects)?;

    let input = cli.input.as_ref().context("Input audio file is required")?;
    if !input.exists() {
        anyhow::bail!("Input file not found: {}", input.display());
    }

    if cli.subtitle_file.is_some()
        && (cli.subtitles || cli.write_subtitles.is_some() || cli.transcribe_only)
    {
        anyhow::bail!(
            "--subtitle-file cannot be combined with --subtitles, --write-subtitles, or --transcribe-only"
        );
    }

    let title_font_sources = [
        cli.font.is_some(),
        cli.font_url.is_some(),
        cli.font_family.is_some(),
    ];
    if title_font_sources.into_iter().filter(|selected| *selected).count() > 1 {
        anyhow::bail!("Use only one of --font, --font-url, or --font-family");
    }

    let subtitle_font_sources = [
        cli.subtitle_font.is_some(),
        cli.subtitle_font_url.is_some(),
        cli.subtitle_font_family.is_some(),
    ];
    if subtitle_font_sources
        .into_iter()
        .filter(|selected| *selected)
        .count()
        > 1
    {
        anyhow::bail!(
            "Use only one of --subtitle-font, --subtitle-font-url, or --subtitle-font-family"
        );
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
    let subtitle_cues = if let Some(ref subtitle_path) = cli.subtitle_file {
        let cues = subtitle::srt::read_srt(subtitle_path)?;
        log::info!(
            "Loaded {} subtitle cues from {}",
            cues.len(),
            subtitle_path.display()
        );
        Some(cues)
    } else if cli.subtitles || cli.write_subtitles.is_some() {
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
        let cue_timing = subtitle::cue::CueTiming {
            break_gap: cli.subtitle_gap,
            min_duration: cli.subtitle_min_duration,
        };
        let cues = subtitle::cue::group_words(words, cli.subtitle_max_chars, cue_timing);
        log::info!("Grouped into {} subtitle cues:", cues.len());
        for (i, c) in cues.iter().enumerate() {
            log::info!("  [{:3}] {:.2}s - {:.2}s  {:?}", i, c.start_time, c.end_time, c.text);
        }
        if let Some(ref subtitle_path) = cli.write_subtitles {
            subtitle::srt::write_srt(subtitle_path, &cues)?;
            log::info!("Wrote subtitles to {}", subtitle_path.display());
        }
        if cli.transcribe_only {
            log::info!("Transcription complete; skipping video render");
            return Ok(());
        }
        Some(cues)
    } else {
        None
    };

    #[cfg(not(feature = "subtitles"))]
    if cli.subtitles
        || cli.subtitle_file.is_some()
        || cli.write_subtitles.is_some()
        || cli.transcribe_only
    {
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

    // Validate the resolved list, not just the CLI args: a typo in a template's
    // `default_effects` used to slip past validation and only warn mid-render,
    // silently producing a video missing that effect.
    render::postprocess::validate_effects(&effects)?;

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
        let shader_src =
            loader::inject_params(&tmpl.fragment_shader, &tmpl.manifest, &param_overrides)?;
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
            let compute_src =
                loader::inject_params(compute_src, &tmpl.manifest, &param_overrides)?;
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

    #[cfg(feature = "subtitles")]
    let subtitle_font_bytes = if let Some(ref font_url) = cli.subtitle_font_url {
        match load_font_from_url(font_url) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log::warn!("Failed to load subtitle font from URL: {}", err);
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
            cli.font_family.as_deref(),
        ))
    } else {
        None
    };

    // 8b. Subtitle renderer
    #[cfg(feature = "subtitles")]
    let subtitle_renderer = subtitle_cues.map(|cues| -> Result<_> {
        let has_subtitle_font = cli.subtitle_font.is_some()
            || cli.subtitle_font_url.is_some()
            || cli.subtitle_font_family.is_some();
        let font_path = if has_subtitle_font {
            cli.subtitle_font.as_deref()
        } else {
            cli.font.as_deref()
        };
        let font_data = if has_subtitle_font {
            subtitle_font_bytes.as_deref()
        } else {
            font_bytes.as_deref()
        };
        let font_family = if has_subtitle_font {
            cli.subtitle_font_family.as_deref()
        } else {
            cli.font_family.as_deref()
        };
        let sub_overlay = TextOverlay::new(
            cli.subtitle_font_size,
            font_path,
            font_data,
            font_family,
        );
        let style = subtitle::render::SubtitleStyle::from_options(
            cli.subtitle_background_opacity,
            cli.subtitle_dim_opacity,
            &cli.subtitle_text_color,
            &cli.subtitle_highlight_color,
            &cli.subtitle_outline_color,
            cli.subtitle_outline_width,
            cli.subtitle_margin_bottom,
            !cli.no_subtitle_karaoke,
        )?;
        Ok(subtitle::render::SubtitleRenderer::new(
            cues,
            sub_overlay,
            cli.subtitle_max_chars,
            style,
        ))
    }).transpose()?;

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

        // Render. With post-processing, skip reading the template pass result
        // back to the CPU — only the post-processed texture is read back, and
        // all GPU work since the last readback is drained by that one wait.
        let mut pixels = if pp_chain.has_effects() {
            frame_renderer.render(&gpu, &slot.pipeline.pipeline, &slot.bind_group)?;
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
