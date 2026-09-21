use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    pub font_family: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
            font_family: None,
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
#[serde(deny_unknown_fields)]
pub struct SubtitleConfig {
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    pub language: Option<String>,
    #[serde(default = "default_subtitle_font_size")]
    pub font_size: f32,
    #[serde(default = "default_subtitle_max_chars")]
    pub max_chars_per_line: usize,
    pub font: Option<PathBuf>,
    pub font_url: Option<String>,
    pub font_family: Option<String>,
    #[serde(default = "default_subtitle_background_opacity")]
    pub background_opacity: f32,
    #[serde(default = "default_subtitle_dim_opacity")]
    pub dim_opacity: f32,
    #[serde(default = "default_subtitle_text_color")]
    pub text_color: String,
    #[serde(default = "default_subtitle_highlight_color")]
    pub highlight_color: String,
    #[serde(default = "default_subtitle_outline_color")]
    pub outline_color: String,
    #[serde(default = "default_subtitle_outline_width")]
    pub outline_width: u32,
    #[serde(default = "default_subtitle_margin_bottom")]
    pub margin_bottom: f32,
    #[serde(default = "default_subtitle_karaoke")]
    pub karaoke: bool,
}

impl Default for SubtitleConfig {
    fn default() -> Self {
        Self {
            whisper_model: default_whisper_model(),
            language: None,
            font_size: default_subtitle_font_size(),
            max_chars_per_line: default_subtitle_max_chars(),
            font: None,
            font_url: None,
            font_family: None,
            background_opacity: default_subtitle_background_opacity(),
            dim_opacity: default_subtitle_dim_opacity(),
            text_color: default_subtitle_text_color(),
            highlight_color: default_subtitle_highlight_color(),
            outline_color: default_subtitle_outline_color(),
            outline_width: default_subtitle_outline_width(),
            margin_bottom: default_subtitle_margin_bottom(),
            karaoke: default_subtitle_karaoke(),
        }
    }
}

fn default_whisper_model() -> String { "base".into() }
fn default_subtitle_font_size() -> f32 { 48.0 }
fn default_subtitle_max_chars() -> usize { 42 }
fn default_subtitle_background_opacity() -> f32 { 0.55 }
fn default_subtitle_dim_opacity() -> f32 { 0.75 }
fn default_subtitle_text_color() -> String { "#FFFFFF".into() }
fn default_subtitle_highlight_color() -> String { "#FFFFFF".into() }
fn default_subtitle_outline_color() -> String { "#000000".into() }
fn default_subtitle_outline_width() -> u32 { 2 }
fn default_subtitle_margin_bottom() -> f32 { 0.08 }
fn default_subtitle_karaoke() -> bool { true }

/// Load a TOML config, surfacing read and parse errors with the offending path.
///
/// A malformed config used to collapse into `None`, silently reverting every
/// setting to defaults while main logged success — so typos went unnoticed.
pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("Invalid TOML in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sonica-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_nested_output_values() {
        let dir = temp_dir("valid");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[output]\nwidth = 1280\nfps = 60\n").unwrap();

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.output.width, 1280);
        assert_eq!(cfg.output.fps, 60);
        assert_eq!(cfg.output.height, 1080); // untouched fields keep their default

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn surfaces_unknown_config_keys() {
        let dir = temp_dir("typo");
        let path = dir.join("config.toml");
        std::fs::write(&path, "widht = 1280\n").unwrap();

        let err = load_config(&path).unwrap_err();
        let message = format!("{err:#}"); // full chain: context + serde cause
        assert!(message.contains("widht"), "error should name the bad key: {message}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
