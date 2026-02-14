use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// A single word-level segment from Whisper transcription.
#[derive(Clone, Debug)]
pub struct WordSegment {
    pub text: String,
    pub start_time: f32,
    pub end_time: f32,
}

pub struct WhisperTranscriber {
    ctx: WhisperContext,
    language: Option<String>,
}

impl WhisperTranscriber {
    pub fn new(model_path: &Path, language: Option<&str>) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .context("Model path contains invalid UTF-8")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| anyhow::anyhow!("Failed to initialize Whisper context: {}", e))?;

        Ok(Self {
            ctx,
            language: language.map(String::from),
        })
    }

    /// Transcribe PCM audio samples and return word-level segments.
    ///
    /// Input samples are mono f32. If `sample_rate` is not 16000, the audio
    /// is resampled to 16kHz using rubato before transcription.
    pub fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<WordSegment>> {
        let samples_16k = if sample_rate != 16000 {
            resample_to_16k(samples, sample_rate)?
        } else {
            samples.to_vec()
        };

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_token_timestamps(true);
        params.set_max_len(1); // word-level segmentation
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        if let Some(ref lang) = self.language {
            params.set_language(Some(lang));
        }

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| anyhow::anyhow!("Failed to create Whisper state: {}", e))?;

        state
            .full(params, &samples_16k)
            .map_err(|e| anyhow::anyhow!("Whisper transcription failed: {}", e))?;

        let num_segments = state.full_n_segments();
        let mut words = Vec::new();

        for i in 0..num_segments {
            let segment = state
                .get_segment(i)
                .ok_or_else(|| anyhow::anyhow!("Segment {} out of bounds", i))?;

            let text = segment
                .to_str_lossy()
                .map_err(|e| anyhow::anyhow!("Failed to get segment text: {}", e))?;

            // Timestamps are in centiseconds (10ms units)
            let start = segment.start_timestamp() as f32 / 100.0;
            let end = segment.end_timestamp() as f32 / 100.0;

            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }

            words.push(WordSegment {
                text: trimmed.to_string(),
                start_time: start,
                end_time: end,
            });
        }

        Ok(words)
    }
}

/// Resample mono f32 audio from `from_rate` to 16000 Hz using rubato.
fn resample_to_16k(samples: &[f32], from_rate: u32) -> Result<Vec<f32>> {
    use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let ratio = 16000.0 / from_rate as f64;
    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        2.0,    // max relative ratio
        params,
        samples.len(),
        1, // mono
    )
    .context("Failed to create resampler")?;

    let input = vec![samples.to_vec()];
    let output = resampler
        .process(&input, None)
        .context("Resampling failed")?;

    Ok(output.into_iter().next().unwrap_or_default())
}
