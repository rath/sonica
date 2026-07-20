use clap::Parser;
use std::path::PathBuf;

/// Shown at the bottom of `--help`. Kept task-shaped: each line is a job
/// someone actually comes to sonica to do, not a tour of the flags.
const EXAMPLES: &str = "\
EXAMPLES:
  # Defaults: frequency_bars, 1080p30, template's own effects
  sonica track.wav -o out.mp4

  # Pick a look, add effects (see --list-templates / --list-effects)
  sonica track.wav -t circular_spectrum --effects crt

  # Strip the template's default effects
  sonica track.wav -t kaleidoscope --effects none

  # Smaller, faster render for previewing
  sonica track.wav --width 1024 --height 576 --crf 23

  # Burn in speech subtitles (build with --features subtitles)
  sonica talk.mp3 --subtitles --whisper-model small --subtitle-lang en

  # Transcribe first, hand-correct the SRT, then render it
  sonica talk.mp3 --transcribe-only --write-subtitles talk.srt
  sonica talk.mp3 --subtitle-file talk.srt

  # Non-Latin subtitles need a font with those glyphs
  sonica talk.mp3 --subtitle-file zh.srt \\
    --subtitle-font \"/System/Library/Fonts/Hiragino Sans GB.ttc\"

NOTES:
  * --subtitle-font-size is in pixels, so lower it when you lower --height.
  * film_grain is high-entropy noise and can inflate the file size ~10x at a
    low --crf; raise --crf or set --bitrate if you use it.
  * Requires ffmpeg on PATH.
";

#[derive(Parser, Debug)]
#[command(
    name = "sonica",
    about = "GPU-accelerated audio visualizer video generator",
    long_about = "Renders an audio file into an MP4 visualization on the GPU.\n\
                  \n\
                  A run picks one TEMPLATE (the visual itself) and any number of\n\
                  EFFECTS (post-processing applied on top of it). These are separate\n\
                  things: circular_spectrum is a template, bloom is an effect.",
    after_help = "Run `sonica --help` for the full list of templates, effects, and examples.",
    after_long_help = EXAMPLES,
    max_term_width = 100
)]
pub struct Cli {
    /// Input audio file (WAV, MP3, FLAC, OGG, AAC, WebM/Opus, or any FFmpeg-supported format)
    pub input: Option<PathBuf>,

    // ---------------------------------------------------------------- Visuals
    /// Visual template; see --list-templates
    #[arg(short, long, default_value = "frequency_bars", help_heading = "Visuals")]
    pub template: String,

    /// Post-processing effects, comma-separated; see --list-effects
    #[arg(long, value_delimiter = ',', help_heading = "Visuals")]
    pub effects: Vec<String>,

    /// Template parameter overrides (key=value, comma-separated)
    #[arg(
        long = "param",
        value_delimiter = ',',
        value_name = "KEY=VALUE",
        help_heading = "Visuals"
    )]
    pub params: Vec<String>,

    /// Video width in pixels
    #[arg(long, default_value_t = 1920, help_heading = "Visuals")]
    pub width: u32,

    /// Video height in pixels
    #[arg(long, default_value_t = 1080, help_heading = "Visuals")]
    pub height: u32,

    /// Frames per second
    #[arg(long, default_value_t = 30, help_heading = "Visuals")]
    pub fps: u32,

    // ------------------------------------------------------ Output & encoding
    /// Output video file
    #[arg(
        short,
        long,
        default_value = "output.mp4",
        help_heading = "Output & Encoding"
    )]
    pub output: PathBuf,

    /// H.264 quality (0-51, lower = better). Ignored when --bitrate is set
    #[arg(long, default_value_t = 18, help_heading = "Output & Encoding")]
    pub crf: u32,

    /// Target bitrate (e.g. 2400k, 5M). Overrides --crf
    #[arg(short, long, help_heading = "Output & Encoding")]
    pub bitrate: Option<String>,

    /// FFmpeg video codec
    #[arg(
        long,
        default_value = "libx264",
        help_heading = "Output & Encoding"
    )]
    pub codec: String,

    /// FFmpeg pixel format
    #[arg(long, default_value = "yuv420p", help_heading = "Output & Encoding")]
    pub pix_fmt: String,

    // ----------------------------------------------------------- Text overlay
    /// Title text drawn in the corner
    #[arg(long, help_heading = "Text Overlay")]
    pub title: Option<String>,

    /// Show elapsed time overlay
    #[arg(long, help_heading = "Text Overlay")]
    pub show_time: bool,

    /// Font file for title/time overlay (TTF/OTF path)
    #[arg(long, value_name = "PATH", help_heading = "Text Overlay")]
    pub font: Option<PathBuf>,

    /// Font URL for title/time overlay (direct TTF/OTF URL or Google Fonts URL)
    #[arg(long, value_name = "URL", help_heading = "Text Overlay")]
    pub font_url: Option<String>,

    /// Installed font family for title/time overlay
    #[arg(long, value_name = "NAME", help_heading = "Text Overlay")]
    pub font_family: Option<String>,

    // -------------------------------------------------------------- Subtitles
    /// Transcribe speech and burn in subtitles (build with --features subtitles)
    #[arg(long, help_heading = "Subtitles")]
    pub subtitles: bool,

    /// Burn in subtitles from an existing SRT instead of transcribing
    #[arg(long, value_name = "PATH", help_heading = "Subtitles")]
    pub subtitle_file: Option<PathBuf>,

    /// Save the generated subtitles as an editable SRT
    #[arg(long, value_name = "PATH", help_heading = "Subtitles")]
    pub write_subtitles: Option<PathBuf>,

    /// Transcribe only, skipping the video render
    #[arg(long, requires = "write_subtitles", help_heading = "Subtitles")]
    pub transcribe_only: bool,

    /// Whisper model: tiny/base/small/medium/large (+ .en variants), or a ggml path
    #[arg(long, default_value = "base", help_heading = "Subtitles")]
    pub whisper_model: String,

    /// Language as ISO 639-1 (e.g. "en", "ko", "zh"). Auto-detects if unset
    #[arg(long, help_heading = "Subtitles")]
    pub subtitle_lang: Option<String>,

    /// Subtitle font size in pixels (scale this with --height)
    #[arg(long, default_value_t = 48.0, help_heading = "Subtitles")]
    pub subtitle_font_size: f32,

    /// Subtitle font file; needed for non-Latin scripts (TTF/OTF/TTC path)
    #[arg(long, value_name = "PATH", help_heading = "Subtitles")]
    pub subtitle_font: Option<PathBuf>,

    /// Subtitle font URL
    #[arg(long, value_name = "URL", help_heading = "Subtitles")]
    pub subtitle_font_url: Option<String>,

    /// Installed font family for subtitles
    #[arg(long, value_name = "NAME", help_heading = "Subtitles")]
    pub subtitle_font_family: Option<String>,

    /// Maximum characters per subtitle line (lower for CJK)
    #[arg(long, default_value_t = 42, help_heading = "Subtitles")]
    pub subtitle_max_chars: usize,

    /// Subtitle background opacity (0.0-1.0)
    #[arg(long, default_value_t = 0.55, help_heading = "Subtitles")]
    pub subtitle_background_opacity: f32,

    /// Opacity of not-yet-spoken karaoke text (0.0-1.0)
    #[arg(long, default_value_t = 0.75, help_heading = "Subtitles")]
    pub subtitle_dim_opacity: f32,

    /// Subtitle text color as #RRGGBB
    #[arg(long, default_value = "#FFFFFF", help_heading = "Subtitles")]
    pub subtitle_text_color: String,

    /// Karaoke highlight color as #RRGGBB
    #[arg(long, default_value = "#FFFFFF", help_heading = "Subtitles")]
    pub subtitle_highlight_color: String,

    /// Subtitle outline color as #RRGGBB
    #[arg(long, default_value = "#000000", help_heading = "Subtitles")]
    pub subtitle_outline_color: String,

    /// Subtitle outline width in pixels
    #[arg(long, default_value_t = 2, help_heading = "Subtitles")]
    pub subtitle_outline_width: u32,

    /// Bottom margin as a fraction of video height (0.0-0.5)
    #[arg(long, default_value_t = 0.08, help_heading = "Subtitles")]
    pub subtitle_margin_bottom: f32,

    /// Disable word-by-word karaoke highlighting
    #[arg(long, help_heading = "Subtitles")]
    pub no_subtitle_karaoke: bool,

    // --------------------------------------------------------- Audio analysis
    /// Smoothing factor for audio analysis (0.0-1.0; higher = calmer motion)
    #[arg(long, default_value_t = 0.85, help_heading = "Audio Analysis")]
    pub smoothing: f32,

    // --------------------------------------------------- Discovery and config
    /// List available templates and exit
    #[arg(long, help_heading = "Discovery & Config")]
    pub list_templates: bool,

    /// List available post-processing effects and exit
    #[arg(long, help_heading = "Discovery & Config")]
    pub list_effects: bool,

    /// Config file path (defaults to ./sonica.toml if present)
    #[arg(long, value_name = "PATH", help_heading = "Discovery & Config")]
    pub config: Option<PathBuf>,
}
