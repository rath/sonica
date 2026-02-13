use anyhow::Result;
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};

use super::decode::AudioData;
use super::features::{FrameFeatures, GlobalAnalysis, SmoothedFrame};

const FFT_SIZE: usize = 2048;
const HOP_SIZE: usize = 1024;

pub fn analyze(audio: &AudioData, fps: u32, smoothing: f32) -> Result<(GlobalAnalysis, Vec<SmoothedFrame>)> {
    let samples = &audio.samples;
    let sr = audio.sample_rate;
    let duration = samples.len() as f32 / sr as f32;
    let total_frames = (duration * fps as f32).ceil() as usize;

    log::info!("Pass 1: Global analysis...");
    let global = pass1_global(samples, sr, duration);

    log::info!("Pass 2: Per-frame FFT ({} frames)...", total_frames);
    let raw_frames = pass2_per_frame(samples, sr, fps, total_frames);

    log::info!("Pass 3: Smoothing & normalization (smoothing={:.2})...", smoothing);
    let smoothed = pass3_smooth(&raw_frames, &global, fps, duration, smoothing);

    Ok((global, smoothed))
}

fn pass1_global(samples: &[f32], sample_rate: u32, duration: f32) -> GlobalAnalysis {
    let peak_amplitude = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    // RMS in windows
    let window_size = sample_rate as usize / 10; // 100ms windows
    let mut peak_rms = 0.0f32;
    for chunk in samples.chunks(window_size) {
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        peak_rms = peak_rms.max(rms);
    }

    // Onset detection via spectral flux
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let hann = hann_window(FFT_SIZE);

    let mut prev_magnitudes = vec![0.0f32; FFT_SIZE / 2];
    let mut flux_values: Vec<(f32, f32)> = Vec::new(); // (time, flux)

    let mut pos = 0;
    while pos + FFT_SIZE <= samples.len() {
        let mut buffer: Vec<Complex<f32>> = samples[pos..pos + FFT_SIZE]
            .iter()
            .enumerate()
            .map(|(i, &s)| Complex::new(s * hann[i], 0.0))
            .collect();
        fft.process(&mut buffer);

        let magnitudes: Vec<f32> = buffer[..FFT_SIZE / 2].iter().map(|c| c.norm()).collect();

        let flux: f32 = magnitudes
            .iter()
            .zip(prev_magnitudes.iter())
            .map(|(cur, prev)| (cur - prev).max(0.0))
            .sum();

        let time = pos as f32 / sample_rate as f32;
        flux_values.push((time, flux));
        prev_magnitudes = magnitudes;
        pos += HOP_SIZE;
    }

    // Adaptive threshold for beat detection
    let beat_times = detect_beats(&flux_values);

    // Tempo estimation
    let tempo_bpm = estimate_tempo(&beat_times);

    log::info!(
        "Global: peak_rms={:.4}, peak_amp={:.4}, beats={}, tempo={:.1} BPM",
        peak_rms, peak_amplitude, beat_times.len(), tempo_bpm
    );

    GlobalAnalysis {
        sample_rate,
        total_samples: samples.len(),
        duration,
        peak_rms,
        peak_amplitude,
        beat_times,
        tempo_bpm,
    }
}

fn detect_beats(flux_values: &[(f32, f32)]) -> Vec<f32> {
    if flux_values.is_empty() {
        return Vec::new();
    }

    let window = 20; // ~200ms at typical hop rate
    let mut beat_times = Vec::new();

    for i in 0..flux_values.len() {
        let start = i.saturating_sub(window);
        let end = (i + window + 1).min(flux_values.len());
        let local_mean: f32 = flux_values[start..end].iter().map(|(_, f)| f).sum::<f32>()
            / (end - start) as f32;

        let threshold = local_mean * 1.5 + 0.01;

        if flux_values[i].1 > threshold {
            // Check if it's a local peak
            let is_peak = (i == 0 || flux_values[i].1 >= flux_values[i - 1].1)
                && (i == flux_values.len() - 1 || flux_values[i].1 >= flux_values[i + 1].1);

            // Minimum gap between beats (100ms)
            let far_enough = beat_times
                .last()
                .map_or(true, |&last: &f32| flux_values[i].0 - last > 0.1);

            if is_peak && far_enough {
                beat_times.push(flux_values[i].0);
            }
        }
    }

    beat_times
}

fn estimate_tempo(beat_times: &[f32]) -> f32 {
    if beat_times.len() < 2 {
        return 120.0; // default
    }

    let intervals: Vec<f32> = beat_times.windows(2).map(|w| w[1] - w[0]).collect();

    // Filter reasonable intervals (60-200 BPM → 0.3-1.0s)
    let reasonable: Vec<f32> = intervals
        .iter()
        .copied()
        .filter(|&i| i >= 0.3 && i <= 1.0)
        .collect();

    if reasonable.is_empty() {
        return 120.0;
    }

    let median_interval = {
        let mut sorted = reasonable.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted[sorted.len() / 2]
    };

    60.0 / median_interval
}

fn pass2_per_frame(
    samples: &[f32],
    sample_rate: u32,
    fps: u32,
    total_frames: usize,
) -> Vec<FrameFeatures> {
    let samples_per_frame = sample_rate as f32 / fps as f32;
    let freq_resolution = sample_rate as f32 / FFT_SIZE as f32;
    let hann = hann_window(FFT_SIZE);

    (0..total_frames)
        .into_par_iter()
        .map(|frame_idx| {
            let center = (frame_idx as f32 * samples_per_frame) as usize;
            let start = center.saturating_sub(FFT_SIZE / 2);
            let end = (start + FFT_SIZE).min(samples.len());

            // Extract windowed samples
            let mut fft_input: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];
            for i in 0..(end - start) {
                fft_input[i] = Complex::new(samples[start + i] * hann[i], 0.0);
            }

            // Per-thread FFT planner (rayon-safe)
            let mut planner = FftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(FFT_SIZE);
            fft.process(&mut fft_input);

            let half = FFT_SIZE / 2;
            let fft_bins: Vec<f32> = fft_input[..half].iter().map(|c| c.norm()).collect();

            // Band energies
            let band_energy = |low_hz: f32, high_hz: f32| -> f32 {
                let low_bin = (low_hz / freq_resolution) as usize;
                let high_bin = ((high_hz / freq_resolution) as usize).min(half);
                if low_bin >= high_bin {
                    return 0.0;
                }
                let sum: f32 = fft_bins[low_bin..high_bin].iter().map(|&x| x * x).sum();
                (sum / (high_bin - low_bin) as f32).sqrt()
            };

            let sub_bass = band_energy(20.0, 60.0);
            let bass = band_energy(60.0, 250.0);
            let low_mid = band_energy(250.0, 500.0);
            let mid = band_energy(500.0, 2000.0);
            let upper_mid = band_energy(2000.0, 4000.0);
            let presence = band_energy(4000.0, 6000.0);
            let brilliance = band_energy(6000.0, 20000.0);

            // RMS
            let frame_start = (center).saturating_sub(samples_per_frame as usize / 2);
            let frame_end = (frame_start + samples_per_frame as usize).min(samples.len());
            let frame_samples = &samples[frame_start..frame_end];
            let rms = if frame_samples.is_empty() {
                0.0
            } else {
                (frame_samples.iter().map(|s| s * s).sum::<f32>() / frame_samples.len() as f32)
                    .sqrt()
            };

            // Spectral centroid
            let total_energy: f32 = fft_bins.iter().sum();
            let spectral_centroid = if total_energy > 1e-10 {
                fft_bins
                    .iter()
                    .enumerate()
                    .map(|(i, &mag)| i as f32 * freq_resolution * mag)
                    .sum::<f32>()
                    / total_energy
            } else {
                0.0
            };

            // Waveform samples for this frame (downsample to ~512 points)
            let waveform_len = 512.min(frame_samples.len());
            let waveform: Vec<f32> = if frame_samples.is_empty() {
                vec![0.0; 512]
            } else {
                (0..waveform_len)
                    .map(|i| {
                        let idx = i * frame_samples.len() / waveform_len;
                        frame_samples[idx]
                    })
                    .collect()
            };

            FrameFeatures {
                fft_bins,
                sub_bass,
                bass,
                low_mid,
                mid,
                upper_mid,
                presence,
                brilliance,
                rms,
                spectral_centroid,
                spectral_flux: 0.0, // computed in sequential post-pass
                waveform,
            }
        })
        .collect()
}

fn pass3_smooth(
    raw: &[FrameFeatures],
    global: &GlobalAnalysis,
    fps: u32,
    _duration: f32,
    smoothing: f32,
) -> Vec<SmoothedFrame> {
    if raw.is_empty() {
        return Vec::new();
    }

    let n = raw.len();
    let num_bins = raw[0].fft_bins.len();

    // Compute spectral flux sequentially
    let mut flux_values: Vec<f32> = vec![0.0; n];
    for i in 1..n {
        let flux: f32 = raw[i]
            .fft_bins
            .iter()
            .zip(raw[i - 1].fft_bins.iter())
            .map(|(cur, prev)| (cur - prev).max(0.0))
            .sum();
        flux_values[i] = flux;
    }

    // Find peaks for normalization
    let peak_rms = global.peak_rms.max(1e-10);
    let peak_flux = flux_values.iter().copied().fold(0.0f32, f32::max).max(1e-10);
    let max_centroid = raw
        .iter()
        .map(|f| f.spectral_centroid)
        .fold(0.0f32, f32::max)
        .max(1e-10);

    // Find peak per FFT bin for normalization
    let mut peak_bins = vec![1e-10f32; num_bins];
    for frame in raw {
        for (i, &val) in frame.fft_bins.iter().enumerate() {
            peak_bins[i] = peak_bins[i].max(val);
        }
    }

    // Bidirectional EMA smoothing
    let alpha = 1.0 - smoothing; // smoothing=0.85 → alpha=0.15 (default behavior)

    // Forward pass
    let mut forward_bins: Vec<Vec<f32>> = vec![vec![0.0; num_bins]; n];
    let mut forward_rms = vec![0.0f32; n];
    let mut forward_bass = vec![0.0f32; n];
    let mut forward_mid = vec![0.0f32; n];
    let mut forward_high = vec![0.0f32; n];

    forward_bins[0] = raw[0].fft_bins.clone();
    forward_rms[0] = raw[0].rms;
    forward_bass[0] = raw[0].sub_bass + raw[0].bass;
    forward_mid[0] = raw[0].low_mid + raw[0].mid;
    forward_high[0] = raw[0].upper_mid + raw[0].presence + raw[0].brilliance;

    for i in 1..n {
        for j in 0..num_bins {
            forward_bins[i][j] =
                alpha * raw[i].fft_bins[j] + (1.0 - alpha) * forward_bins[i - 1][j];
        }
        forward_rms[i] = alpha * raw[i].rms + (1.0 - alpha) * forward_rms[i - 1];
        let bass_val = raw[i].sub_bass + raw[i].bass;
        let mid_val = raw[i].low_mid + raw[i].mid;
        let high_val = raw[i].upper_mid + raw[i].presence + raw[i].brilliance;
        forward_bass[i] = alpha * bass_val + (1.0 - alpha) * forward_bass[i - 1];
        forward_mid[i] = alpha * mid_val + (1.0 - alpha) * forward_mid[i - 1];
        forward_high[i] = alpha * high_val + (1.0 - alpha) * forward_high[i - 1];
    }

    // Backward pass
    let mut backward_bins: Vec<Vec<f32>> = vec![vec![0.0; num_bins]; n];
    let mut backward_rms = vec![0.0f32; n];
    let mut backward_bass = vec![0.0f32; n];
    let mut backward_mid = vec![0.0f32; n];
    let mut backward_high = vec![0.0f32; n];

    backward_bins[n - 1] = raw[n - 1].fft_bins.clone();
    backward_rms[n - 1] = raw[n - 1].rms;
    backward_bass[n - 1] = raw[n - 1].sub_bass + raw[n - 1].bass;
    backward_mid[n - 1] = raw[n - 1].low_mid + raw[n - 1].mid;
    backward_high[n - 1] = raw[n - 1].upper_mid + raw[n - 1].presence + raw[n - 1].brilliance;

    for i in (0..n - 1).rev() {
        for j in 0..num_bins {
            backward_bins[i][j] =
                alpha * raw[i].fft_bins[j] + (1.0 - alpha) * backward_bins[i + 1][j];
        }
        backward_rms[i] = alpha * raw[i].rms + (1.0 - alpha) * backward_rms[i + 1];
        let bass_val = raw[i].sub_bass + raw[i].bass;
        let mid_val = raw[i].low_mid + raw[i].mid;
        let high_val = raw[i].upper_mid + raw[i].presence + raw[i].brilliance;
        backward_bass[i] = alpha * bass_val + (1.0 - alpha) * backward_bass[i + 1];
        backward_mid[i] = alpha * mid_val + (1.0 - alpha) * backward_mid[i + 1];
        backward_high[i] = alpha * high_val + (1.0 - alpha) * backward_high[i + 1];
    }

    // Peak values for band normalization
    let peak_bass = forward_bass.iter().copied().fold(0.0f32, f32::max).max(1e-10);
    let peak_mid = forward_mid.iter().copied().fold(0.0f32, f32::max).max(1e-10);
    let peak_high = forward_high.iter().copied().fold(0.0f32, f32::max).max(1e-10);

    // Beat tracking
    let beat_decay = 0.9f32.powf(1.0 / fps as f32 * 10.0); // ~100ms decay

    let mut beat_intensity = 0.0f32;
    let mut frames: Vec<SmoothedFrame> = Vec::with_capacity(n);

    for i in 0..n {
        let time = i as f32 / fps as f32;

        // Check if this frame is on a beat
        let is_beat = global.beat_times.iter().any(|&bt| {
            let frame_time = time;
            (frame_time - bt).abs() < 0.5 / fps as f32
        });

        if is_beat {
            beat_intensity = 1.0;
        } else {
            beat_intensity *= beat_decay;
        }

        // Beat phase
        let beat_phase = compute_beat_phase(time, &global.beat_times);

        // Average forward + backward, then normalize
        let smoothed_bins: Vec<f32> = (0..num_bins)
            .map(|j| {
                let avg = (forward_bins[i][j] + backward_bins[i][j]) * 0.5;
                (avg / peak_bins[j]).min(1.0)
            })
            .collect();

        let rms = ((forward_rms[i] + backward_rms[i]) * 0.5 / peak_rms).min(1.0);
        let bass = ((forward_bass[i] + backward_bass[i]) * 0.5 / peak_bass).min(1.0);
        let mid = ((forward_mid[i] + backward_mid[i]) * 0.5 / peak_mid).min(1.0);
        let high = ((forward_high[i] + backward_high[i]) * 0.5 / peak_high).min(1.0);

        let spectral_centroid = (raw[i].spectral_centroid / max_centroid).min(1.0);
        let spectral_flux = (flux_values[i] / peak_flux).min(1.0);

        frames.push(SmoothedFrame {
            fft_bins: smoothed_bins,
            bass,
            mid,
            high,
            rms,
            spectral_centroid,
            spectral_flux,
            beat_intensity,
            beat_phase,
            is_beat,
            waveform: raw[i].waveform.clone(),
            time,
        });
    }

    frames
}

fn compute_beat_phase(time: f32, beat_times: &[f32]) -> f32 {
    if beat_times.is_empty() {
        return 0.0;
    }

    // Find surrounding beats
    let idx = beat_times.partition_point(|&bt| bt <= time);

    if idx == 0 {
        // Before first beat
        if beat_times[0] > 0.0 {
            return (time / beat_times[0]).min(1.0);
        }
        return 0.0;
    }

    if idx >= beat_times.len() {
        // After last beat
        return 1.0;
    }

    let prev = beat_times[idx - 1];
    let next = beat_times[idx];
    let interval = next - prev;

    if interval > 0.0 {
        (time - prev) / interval
    } else {
        0.0
    }
}

fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (size - 1) as f32).cos())
        })
        .collect()
}
