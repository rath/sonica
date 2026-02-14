use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "sonica", about = "GPU-accelerated audio visualizer video generator")]
pub struct Cli {
    /// Input audio file (WAV, MP3, FLAC, OGG)
    pub input: Option<PathBuf>,

    /// Output video file
    #[arg(short, long, default_value = "output.mp4")]
    pub output: PathBuf,

    /// Template name
    #[arg(short, long, default_value = "frequency_bars")]
    pub template: String,

    /// Video width in pixels
    #[arg(long, default_value_t = 1920)]
    pub width: u32,

    /// Video height in pixels
    #[arg(long, default_value_t = 1080)]
    pub height: u32,

    /// Frames per second
    #[arg(long, default_value_t = 30)]
    pub fps: u32,

    /// H.264 CRF quality (0-51, lower = better). Ignored when --bitrate is set.
    #[arg(long, default_value_t = 18)]
    pub crf: u32,

    /// Video bitrate (e.g. 2400k, 5M). When set, uses -b:v instead of -crf.
    #[arg(short, long)]
    pub bitrate: Option<String>,

    /// Post-processing effects (comma-separated or preset name)
    #[arg(long, value_delimiter = ',')]
    pub effects: Vec<String>,

    /// Title text overlay
    #[arg(long)]
    pub title: Option<String>,

    /// Font file for title/time overlay (TTF/OTF path)
    #[arg(long)]
    pub font: Option<PathBuf>,

    /// Font URL for title/time overlay (direct TTF/OTF URL or Google Fonts URL)
    #[arg(long)]
    pub font_url: Option<String>,

    /// Show elapsed time overlay
    #[arg(long)]
    pub show_time: bool,

    /// Smoothing factor for audio analysis (0.0-1.0)
    #[arg(long, default_value_t = 0.85)]
    pub smoothing: f32,

    /// Template parameter overrides (key=value, comma-separated)
    #[arg(long = "param", value_delimiter = ',')]
    pub params: Vec<String>,

    /// Config file path (defaults to ./sonica.toml if present)
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// List available templates and exit
    #[arg(long)]
    pub list_templates: bool,

    /// FFmpeg video codec
    #[arg(long, default_value = "libx264")]
    pub codec: String,

    /// FFmpeg pixel format
    #[arg(long, default_value = "yuv420p")]
    pub pix_fmt: String,

    /// Enable subtitle generation via speech recognition (requires --features subtitles)
    #[arg(long)]
    pub subtitles: bool,

    /// Whisper model: file path or model name (tiny/base/small/medium/large)
    #[arg(long, default_value = "base")]
    pub whisper_model: String,

    /// Subtitle language (ISO 639-1, e.g. "en", "ko"). Auto-detect if not set.
    #[arg(long)]
    pub subtitle_lang: Option<String>,

    /// Subtitle font size in pixels
    #[arg(long, default_value_t = 48.0)]
    pub subtitle_font_size: f32,

    /// Maximum characters per subtitle line
    #[arg(long, default_value_t = 42)]
    pub subtitle_max_chars: usize,
}
