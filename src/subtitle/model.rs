use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Known Whisper model sizes and their HuggingFace filenames.
const KNOWN_MODELS: &[(&str, &str)] = &[
    ("tiny", "ggml-tiny.bin"),
    ("tiny.en", "ggml-tiny.en.bin"),
    ("base", "ggml-base.bin"),
    ("base.en", "ggml-base.en.bin"),
    ("small", "ggml-small.bin"),
    ("small.en", "ggml-small.en.bin"),
    ("medium", "ggml-medium.bin"),
    ("medium.en", "ggml-medium.en.bin"),
    ("large", "ggml-large-v3-turbo.bin"),
];

/// Resolve a model input string to an actual file path.
///
/// - If `input` is an existing file path, return it directly.
/// - If `input` is a known model name (tiny/base/small/medium/large),
///   check the cache directory and download from HuggingFace if missing.
pub fn resolve_model_path(input: &str) -> Result<PathBuf> {
    let as_path = Path::new(input);
    if as_path.exists() {
        log::info!("Using Whisper model from path: {}", as_path.display());
        return Ok(as_path.to_path_buf());
    }

    let (model_name, filename) = KNOWN_MODELS
        .iter()
        .find(|(name, _)| *name == input)
        .copied()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown Whisper model '{}'. Valid names: {}. Or provide a file path.",
                input,
                KNOWN_MODELS
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    let cache_dir = model_cache_dir()?;
    let cached_path = cache_dir.join(filename);

    if cached_path.exists() {
        log::info!(
            "Using cached Whisper model '{}': {}",
            model_name,
            cached_path.display()
        );
        return Ok(cached_path);
    }

    log::info!(
        "Downloading Whisper model '{}' from HuggingFace...",
        model_name
    );
    download_model(filename, &cached_path)?;
    log::info!("Model saved to {}", cached_path.display());

    Ok(cached_path)
}

fn model_cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .or_else(dirs::home_dir)
        .context("Cannot determine cache directory")?;
    let dir = base.join("sonica").join("models");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create model cache dir: {}", dir.display()))?;
    Ok(dir)
}

fn download_model(filename: &str, dest: &Path) -> Result<()> {
    use hf_hub::api::sync::Api;

    let api = Api::new().context("Failed to initialize HuggingFace Hub API")?;
    let repo = api.model("ggerganov/whisper.cpp".to_string());
    let downloaded = repo
        .get(filename)
        .with_context(|| format!("Failed to download model file '{}' from HuggingFace", filename))?;

    // hf-hub downloads to its own cache; copy to our cache location
    if downloaded != dest {
        std::fs::copy(&downloaded, dest).with_context(|| {
            format!(
                "Failed to copy model from {} to {}",
                downloaded.display(),
                dest.display()
            )
        })?;
    }

    Ok(())
}
