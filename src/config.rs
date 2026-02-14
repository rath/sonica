use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub effects: Vec<String>,
    #[serde(default)]
    pub subtitle: SubtitleConfig,
}

#[derive(Debug, Deserialize)]
pub struct OutputConfig {
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_crf")]
    pub crf: u32,
    #[serde(default = "default_codec")]
    pub codec: String,
    pub font: Option<PathBuf>,
    pub font_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_smoothing")]
    pub smoothing: f32,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            width: default_width(),
            height: default_height(),
            fps: default_fps(),
            crf: default_crf(),
            codec: default_codec(),
            font: None,
            font_url: None,
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            smoothing: default_smoothing(),
        }
    }
}

fn default_width() -> u32 { 1920 }
fn default_height() -> u32 { 1080 }
fn default_fps() -> u32 { 30 }
fn default_crf() -> u32 { 18 }
fn default_codec() -> String { "libx264".into() }
fn default_smoothing() -> f32 { 0.85 }

#[derive(Debug, Deserialize)]
pub struct SubtitleConfig {
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    pub language: Option<String>,
    #[serde(default = "default_subtitle_font_size")]
    pub font_size: f32,
    #[serde(default = "default_subtitle_max_chars")]
    pub max_chars_per_line: usize,
}

impl Default for SubtitleConfig {
    fn default() -> Self {
        Self {
            whisper_model: default_whisper_model(),
            language: None,
            font_size: default_subtitle_font_size(),
            max_chars_per_line: default_subtitle_max_chars(),
        }
    }
}

fn default_whisper_model() -> String { "base".into() }
fn default_subtitle_font_size() -> f32 { 48.0 }
fn default_subtitle_max_chars() -> usize { 42 }

pub fn load_config(path: &PathBuf) -> Option<Config> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}
