use anyhow::{Context, Result};
use log::debug;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// A single word with timing information, reassembled from BPE tokens.
#[derive(Clone, Debug)]
pub struct TimedWord {
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

    /// Transcribe PCM audio samples and return word-level segments with per-word timing.
    ///
    /// Input samples are mono f32. If `sample_rate` is not 16000, the audio
    /// is resampled to 16kHz using rubato before transcription.
    ///
    /// Tokens from Whisper's BPE are reassembled into whole words, preserving
    /// individual token timestamps for karaoke-style rendering.
    pub fn transcribe(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<TimedWord>> {
        let samples_16k = if sample_rate != 16000 {
            resample_to_16k(samples, sample_rate)?
        } else {
            samples.to_vec()
        };

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_token_timestamps(true);
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
        let mut all_words = Vec::new();

        for i in 0..num_segments {
            let segment = state
                .get_segment(i)
                .ok_or_else(|| anyhow::anyhow!("Segment {} out of bounds", i))?;

            let n_tokens = segment.n_tokens();
            let mut token_bytes: Vec<u8> = Vec::new();
            let mut token_t0: i64 = 0;
            let mut token_t1: i64 = 0;
            let mut has_tokens = false;

            for j in 0..n_tokens {
                let Some(token) = segment.get_token(j) else {
                    continue;
                };

                let data = token.token_data();

                // Skip special tokens (negative IDs or IDs >= special token range)
                if data.id < 0 {
                    continue;
                }

                let bytes = match token.to_bytes() {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                // Empty token bytes — skip
                if bytes.is_empty() {
                    continue;
                }

                // Skip special tokens: [_BEG_], [_TT_123], [_SOT_], etc.
                // These are ASCII-safe so checking raw bytes is fine.
                if bytes.starts_with(b"[_") && bytes.ends_with(b"]") {
                    continue;
                }

                // Whisper BPE tokens: a leading space (or Ġ = 0xC4 0xA0) signals a new word boundary
                let starts_new_word = bytes.starts_with(b" ") || bytes.starts_with(&[0xC4, 0xA0]);

                if starts_new_word && has_tokens {
                    // Flush accumulated bytes as a word
                    if let Some(word) = flush_word(&token_bytes, token_t0, token_t1) {
                        all_words.push(word);
                    }
                    token_bytes.clear();
                    has_tokens = false;
                }

                if !has_tokens {
                    token_t0 = data.t0;
                    has_tokens = true;
                }
                token_t1 = data.t1;
                token_bytes.extend_from_slice(bytes);
            }

            // Flush last word in segment
            if has_tokens {
                if let Some(word) = flush_word(&token_bytes, token_t0, token_t1) {
                    all_words.push(word);
                }
            }
        }

        debug!("Token reassembly produced {} words", all_words.len());
        Ok(all_words)
    }
}

/// Flush accumulated token bytes into a TimedWord, trimming whitespace.
fn flush_word(bytes: &[u8], t0: i64, t1: i64) -> Option<TimedWord> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(TimedWord {
        text: trimmed.to_string(),
        start_time: t0 as f32 / 100.0,
        end_time: t1 as f32 / 100.0,
    })
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
