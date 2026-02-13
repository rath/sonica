# Sonica

GPU-accelerated audio visualizer video generator.

Takes an audio file, runs FFT analysis, renders visualizations with GPU shaders, and outputs an MP4 video with the original audio.

## Requirements

- Rust 1.70+
- ffmpeg (must be in PATH)
- macOS with Metal, or Linux/Windows with Vulkan/DX12

## Install

```bash
git clone <repo-url>
cd sonica
cargo build --release
```

The binary will be at `target/release/sonica`.

## Usage

```bash
# Basic — generates output.mp4 with frequency bars + default effects
sonica audio.wav

# Specify output and template
sonica audio.wav -o visualizer.mp4 -t circular_spectrum

# Cycle through all templates (equal duration each)
sonica audio.wav -t all --effects crt

# CRT retro style
sonica audio.wav --effects crt

# 4K 60fps high quality
sonica track.flac -t kaleidoscope --width 3840 --height 2160 --fps 60 --crf 12

# Hardware encoding on macOS
sonica audio.wav --codec h264_videotoolbox --pix-fmt nv12

# List available templates
sonica --list-templates
```

## Templates

| Template | Description |
|----------|-------------|
| `frequency_bars` | Classic equalizer bars with log frequency mapping |
| `waveform_scope` | PCM waveform oscilloscope with glow |
| `circular_spectrum` | Radial spectrum analyzer with beat-reactive radius |
| `spectrogram` | Scrolling time-frequency heatmap |
| `particle_burst` | Beat-driven particle system |
| `kaleidoscope` | Audio-reactive fractal kaleidoscope |
| `all` | Cycle through all templates, equal duration each |

## Effects

Post-processing effects can be combined with `--effects`:

```bash
# Single effect
sonica audio.wav --effects bloom

# Multiple effects
sonica audio.wav --effects bloom,vignette,chromatic_aberration

# CRT preset (scanlines + chromatic aberration + vignette + film grain + color grading)
sonica audio.wav --effects crt
```

Available effects: `bloom`, `chromatic_aberration`, `vignette`, `film_grain`, `crt_scanlines`, `color_grading`

When `--effects` is not specified, each template uses its own default effects.

## CLI Reference

```
sonica [OPTIONS] [INPUT]

Arguments:
  [INPUT]  Input audio file (WAV, MP3, FLAC, OGG)

Options:
  -o, --output <PATH>        Output video file [default: output.mp4]
  -t, --template <NAME>      Template name, or "all" to cycle [default: frequency_bars]
  -b, --bitrate <RATE>       Video bitrate (e.g. 2400k, 5M), overrides --crf
      --width <PX>           Video width [default: 1920]
      --height <PX>          Video height [default: 1080]
      --fps <N>              Frames per second [default: 30]
      --crf <N>              H.264 quality, 0-51, lower=better [default: 18]
      --effects <LIST>       Post-processing effects, comma-separated (use "none" to disable)
      --smoothing <F>        Audio smoothing factor, 0.0-1.0 [default: 0.85]
      --title <TEXT>         Title text overlay (bottom center)
      --show-time            Show elapsed time overlay (bottom right)
      --config <PATH>        Config file path [default: ./sonica.toml]
      --codec <NAME>         FFmpeg video codec [default: libx264]
      --pix-fmt <FMT>        FFmpeg pixel format [default: yuv420p]
      --list-templates       List available templates and exit
  -h, --help                 Print help
```

## Configuration File

Sonica can read settings from a TOML config file. By default it looks for `sonica.toml` in the current directory. Use `--config <path>` to specify a custom path.

CLI flags always take priority over config values.

```toml
[output]
width = 1280
height = 720
fps = 60
crf = 15
codec = "libx264"

[audio]
smoothing = 0.9

effects = ["bloom", "vignette"]
```

## Supported Audio Formats

WAV, MP3, FLAC, OGG/Vorbis, AAC — via [symphonia](https://github.com/pdeljanov/Symphonia).

## How It Works

1. **Decode** audio to mono PCM samples
2. **Analyze** in 3 passes:
   - Global stats (peak levels, beat detection, tempo)
   - Per-frame FFT with frequency band extraction (parallelized)
   - Bidirectional smoothing and normalization
3. **Render** each frame on GPU via wgpu (Metal/Vulkan) with WGSL shaders
4. **Post-process** through a chain of effect shaders
5. **Encode** by piping raw RGBA frames to ffmpeg

## Performance

On Apple M2 Max, 100 seconds of audio:

| Resolution | Effects | Time | Speed |
|-----------|---------|------|-------|
| 1280x720 | none | ~8s | 12x realtime |
| 1920x1080 | CRT (5 passes) | ~43s | 2.3x realtime |

## License

MIT
