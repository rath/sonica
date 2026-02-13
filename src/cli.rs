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

    /// Show elapsed time overlay
    #[arg(long)]
    pub show_time: bool,

    /// Smoothing factor for audio analysis (0.0-1.0)
    #[arg(long, default_value_t = 0.85)]
    pub smoothing: f32,

    /// List available templates and exit
    #[arg(long)]
    pub list_templates: bool,

    /// FFmpeg video codec
    #[arg(long, default_value = "libx264")]
    pub codec: String,

    /// FFmpeg pixel format
    #[arg(long, default_value = "yuv420p")]
    pub pix_fmt: String,
}
