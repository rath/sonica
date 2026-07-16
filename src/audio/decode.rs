use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

pub struct AudioData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub fn decode_audio(path: &Path) -> Result<AudioData> {
    let audio = match decode_with_symphonia(path) {
        Ok(audio) => audio,
        Err(symphonia_error) => {
            log::warn!(
                "Symphonia could not decode {}: {:#}. Falling back to FFmpeg.",
                path.display(),
                symphonia_error
            );
            decode_with_ffmpeg(path).map_err(|ffmpeg_error| {
                anyhow!(
                    "Failed to decode audio with both Symphonia and FFmpeg.\n\
                     Symphonia: {symphonia_error:#}\n\
                     FFmpeg: {ffmpeg_error:#}"
                )
            })?
        }
    };

    log::info!(
        "Decoded audio: {} samples, {}Hz, {:.1}s",
        audio.samples.len(),
        audio.sample_rate,
        audio.samples.len() as f32 / audio.sample_rate as f32
    );

    Ok(audio)
}

fn decode_with_symphonia(path: &Path) -> Result<AudioData> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open audio file: {}", path.display()))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .context("Failed to probe audio format")?;

    let track = format
        .default_track(TrackType::Audio)
        .context("No audio tracks found")?;

    let track_id = track.id;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .context("Audio track has no codec parameters")?;

    let channels = codec_params.channels.as_ref().map_or(1, |c| c.count());
    let sample_rate = codec_params.sample_rate.context("Unknown sample rate")?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .context("Failed to create audio decoder")?;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut packet_samples: Vec<f32> = Vec::new();

    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };

        decoded.copy_to_vec_interleaved(&mut packet_samples);

        // Downmix to mono
        if channels == 1 {
            all_samples.extend_from_slice(&packet_samples);
        } else {
            for frame_samples in packet_samples.chunks(channels) {
                let mono: f32 = frame_samples.iter().sum::<f32>() / channels as f32;
                all_samples.push(mono);
            }
        }
    }

    Ok(AudioData {
        samples: all_samples,
        sample_rate,
    })
}

const FFMPEG_FALLBACK_SAMPLE_RATE: u32 = 48_000;

fn decode_with_ffmpeg(path: &Path) -> Result<AudioData> {
    let sample_rate = FFMPEG_FALLBACK_SAMPLE_RATE.to_string();
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
        ])
        .arg(path)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &sample_rate,
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ])
        .output()
        .context("Failed to run FFmpeg audio decoder. Is ffmpeg installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("FFmpeg audio decoder exited with an error:\n{stderr}");
    }

    let samples = parse_f32le(&output.stdout)?;
    if samples.is_empty() {
        anyhow::bail!("FFmpeg audio decoder returned no samples");
    }

    Ok(AudioData {
        samples,
        sample_rate: FFMPEG_FALLBACK_SAMPLE_RATE,
    })
}

fn parse_f32le(bytes: &[u8]) -> Result<Vec<f32>> {
    let chunks = bytes.chunks_exact(std::mem::size_of::<f32>());
    if !chunks.remainder().is_empty() {
        anyhow::bail!("FFmpeg returned a truncated f32le audio stream");
    }

    Ok(chunks
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("f32 chunk has four bytes")))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_little_endian_float_samples() {
        let expected = [-1.0f32, -0.25, 0.0, 0.5, 1.0];
        let bytes = expected
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();

        let samples = parse_f32le(&bytes).unwrap();

        assert_eq!(samples, expected);
    }

    #[test]
    fn rejects_truncated_float_stream() {
        assert!(parse_f32le(&[0, 1, 2]).is_err());
    }
}
