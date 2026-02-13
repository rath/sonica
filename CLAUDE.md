# CLAUDE.md - Sonica Project Guide

## Overview

Sonica is a GPU-accelerated audio visualizer that generates MP4 videos from audio files. It uses Rust + wgpu (Metal backend on macOS) for headless GPU rendering and pipes raw RGBA frames to ffmpeg for encoding.

## Architecture

```
Audio File → symphonia decode → 3-pass analysis → Vec<SmoothedFrame>
                                                        ↓
                              wgpu headless render (WGSL shaders)
                                                        ↓
                              post-process chain (ping-pong textures)
                                                        ↓
                              ffmpeg stdin pipe → MP4 output
```

### Data Flow Per Frame

1. `SmoothedFrame` → `FrameUniforms` uniform buffer + FFT/waveform storage buffers
2. Template WGSL shader renders to texture via fullscreen triangle (3 vertices, no vertex buffer)
3. Optional post-processing passes (ping-pong between two textures)
4. `copy_texture_to_buffer` → CPU readback (with 256-byte row alignment stripping)
5. Raw RGBA bytes written to ffmpeg's stdin pipe

## Key Source Files

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI parsing, orchestration loop |
| `src/cli.rs` | clap derive struct for all CLI args |
| `src/config.rs` | TOML config schema, loaded from `sonica.toml` or `--config` |
| `src/audio/decode.rs` | symphonia → `Vec<f32>` mono PCM |
| `src/audio/analysis.rs` | 3-pass pipeline: global stats → per-frame FFT (rayon) → bidirectional smoothing |
| `src/audio/features.rs` | `FrameFeatures`, `SmoothedFrame`, `GlobalAnalysis` structs |
| `src/render/gpu.rs` | `GpuContext`: headless wgpu init (Metal/Vulkan/DX12) |
| `src/render/pipeline.rs` | `FrameUniforms` (repr(C) Pod), `RenderPipeline` builder |
| `src/render/frame.rs` | `FrameRenderer`: render target texture + output buffer + readback |
| `src/render/postprocess.rs` | `PostProcessChain`: ping-pong effect chain, 6 built-in effects |
| `src/templates/loader.rs` | Template discovery from `templates/` dir, shader loading |
| `src/templates/manifest.rs` | `manifest.json` serde schema |
| `src/encode/ffmpeg.rs` | `FfmpegEncoder`: subprocess with piped stdin |

## Template System

Each template is a directory under `templates/` containing:
- `manifest.json` — metadata, default effects, parameter definitions
- `main.wgsl` — fragment shader (must export `vs_main` and `fs_main`)

### Shader Contract

All templates receive the same bind group layout:
- `@group(0) @binding(0)` — `FrameUniforms` (uniform buffer, 16 floats)
- `@group(0) @binding(1)` — `array<f32>` FFT magnitude bins (storage, read-only)
- `@group(0) @binding(2)` — `array<f32>` waveform samples (storage, read-only)

The vertex shader uses a fullscreen triangle trick: `draw(0..3, 0..1)` with no vertex buffer, vertex positions computed from `vertex_index`.

### Available Templates

| Template | Description |
|----------|-------------|
| `frequency_bars` | Classic equalizer bars, log frequency mapping |
| `waveform_scope` | PCM oscilloscope with glow |
| `circular_spectrum` | Radial spectrum, beat-reactive radius |
| `spectrogram` | Scrolling time-frequency heatmap |
| `particle_burst` | Procedural particles driven by beats |
| `kaleidoscope` | Audio-reactive fractal patterns |

## Post-Processing Effects

Effects are WGSL fragment shaders with their own bind group:
- `@binding(0)` — `PPUniforms` (resolution, time, intensity)
- `@binding(1)` — input texture (from previous pass)
- `@binding(2)` — linear sampler

Available effects: `bloom`, `chromatic_aberration`, `vignette`, `film_grain`, `crt_scanlines`, `color_grading`

Preset `crt` expands to: scanlines + chromatic_aberration + vignette + film_grain + color_grading

When no `--effects` flag is given, the template's `default_effects` from `manifest.json` are used.

## Audio Analysis Pipeline

### Pass 1 — Global Analysis
- Peak RMS, peak amplitude
- Beat detection via spectral flux with adaptive threshold
- Tempo estimation via autocorrelation of beat intervals

### Pass 2 — Per-Frame FFT (parallelized with rayon)
- 2048-point FFT, Hann window, 1024 hop size
- 7 frequency bands (sub_bass through brilliance)
- RMS, spectral centroid, waveform samples (downsampled to 512)

### Pass 3 — Smoothing & Normalization
- Bidirectional EMA (forward + backward, zero phase delay)
- All values normalized to 0.0–1.0 using global peaks
- Beat intensity: 1.0 on onset → exponential decay
- Beat phase: 0.0–1.0 within current beat interval

## Build & Run

```bash
cargo build --release
./target/release/sonica audio.wav -o output.mp4
./target/release/sonica audio.wav -t circular_spectrum --effects crt --width 1920 --height 1080
```

Requires `ffmpeg` in PATH.

## Performance Notes

On Apple M2 Max:
- 1080p30 with CRT effects: ~2.3x realtime
- 720p30 no effects: ~12x realtime
- Audio analysis of 100s file: ~70ms

The bottleneck is the per-frame GPU readback (`map_async` + `poll(Wait)`). A double-buffered readback strategy could improve throughput.

## Not Yet Implemented

- Compute shader dispatch: `ComputePipelineWrapper` and bind group layout exist, but the render loop has a TODO stub for creating compute bind groups and dispatching workgroups. Templates with `compute` shader in manifest will load the pipeline but not execute it.

## Conventions

- Edition 2021 (not 2024, for dependency compatibility)
- wgpu v24 (stable, Metal backend confirmed working)
- All GPU structs use `#[repr(C)]` + `bytemuck::Pod` for safe buffer writes
- Template shaders are self-contained (duplicate the common VS and struct definitions)
- Post-processing effect shaders are embedded as string literals in `postprocess.rs`

