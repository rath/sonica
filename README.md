# Sonica

GPU-accelerated audio visualizer video generator.

Takes an audio file, runs FFT analysis, renders visualizations with GPU shaders, and outputs an MP4 video with the original audio.

## Requirements

- Rust 1.70+
- ffmpeg (must be in PATH)
- macOS (Metal) — tested
- Linux (Vulkan) / Windows (DX12) — should work but untested

## Install

```bash
cargo install --git <repo-url>
```

All templates and shaders are embedded in the binary, so no additional files are needed.

### Build from source

```bash
git clone <repo-url>
cd sonica
cargo build --release
# binary at target/release/sonica
```

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

# Korean title with Google Noto Sans KR
sonica audio.wav --title "안녕하세요, SONICA" --font-url "https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/SubsetOTF/KR/NotoSansKR-Regular.otf"

# 실제 동작되는 Noto Sans KR TTF/OTF URL 예시
sonica audio.wav --title "안녕하세요" --font-url "https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/SubsetOTF/KR/NotoSansKR-Regular.otf"

# 로컬 폰트 파일 경로 예시 (macOS)
sonica audio.wav --title "안녕하세요" --font "/System/Library/Fonts/Supplemental/NotoSansCJK-Regular.ttc"

# List available templates
sonica --list-templates
```

## Templates

### circular_spectrum
Radial spectrum analyzer with beat-reactive radius

https://github.com/user-attachments/assets/f4806740-7ca4-4bef-be9d-2cb1ddf34fef

### particle_burst
Beat-driven particle system

https://github.com/user-attachments/assets/89f9a6b8-d322-4a01-b180-4b04c06015d7

### waveform_scope
PCM waveform oscilloscope with glow

https://github.com/user-attachments/assets/04d19960-2676-401c-a844-8f12c2986a7f

### frequency_bars
Classic equalizer bars with log frequency mapping

https://github.com/user-attachments/assets/2e3a6ac7-0014-4a59-9ecd-8c968018ce48

### kaleidoscope
Audio-reactive fractal kaleidoscope

https://github.com/user-attachments/assets/502ccbc0-7242-4736-a837-72588b1f16e7

### spectrogram
Scrolling time-frequency heatmap

https://github.com/user-attachments/assets/e01722fc-a38a-473f-a321-81781299f108

### all
Cycle through all templates, equal duration each.

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
      --font <PATH>          Font file for title/time overlay (TTF/OTF)
      --font-url <URL>       Font URL for title/time overlay (TTF/OTF or Google Fonts URL)
      --show-time            Show elapsed time overlay, MM:SS.CC (bottom right)
      --param <KEY=VALUE>    Template parameter overrides, comma-separated
      --config <PATH>        Config file path [default: ./sonica.toml]
      --codec <NAME>         FFmpeg video codec [default: libx264]
      --pix-fmt <FMT>        FFmpeg pixel format [default: yuv420p]
      --list-templates       List available templates and exit
  -h, --help                 Print help
```

## Template Parameters

Each template defines configurable parameters (bar count, colors, etc.) with `--param`:

```bash
# Change bar count and disable mirroring
sonica audio.wav -t frequency_bars --param bar_count=128,mirror=false,gap_ratio=0.1

# Kaleidoscope with 8-fold symmetry
sonica audio.wav -t kaleidoscope --param symmetry=8,zoom=2.0

# More particles
sonica audio.wav -t particle_burst --param particle_count=500
```

Use `--list-templates` to see available templates. Check each template's `manifest.json` for parameter definitions.

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
pix_fmt = "yuv420p"
font = "/path/to/NotoSansKR-Regular.otf"
font_url = "https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/SubsetOTF/KR/NotoSansKR-Regular.otf"

[audio]
smoothing = 0.9

effects = ["bloom", "vignette"]
```

For Korean text, use a font that includes CJK glyphs (for example `NotoSansKR-Regular.otf` from Google Fonts) via `--font`, `--font-url`, or `font` / `font_url` in the config.

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
