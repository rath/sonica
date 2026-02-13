#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct FrameFeatures {
    /// FFT magnitude bins (N/2 elements, linear scale)
    pub fft_bins: Vec<f32>,
    /// Band energies
    pub sub_bass: f32,   // 20-60 Hz
    pub bass: f32,       // 60-250 Hz
    pub low_mid: f32,    // 250-500 Hz
    pub mid: f32,        // 500-2000 Hz
    pub upper_mid: f32,  // 2-4 kHz
    pub presence: f32,   // 4-6 kHz
    pub brilliance: f32, // 6-20 kHz
    /// RMS energy (linear)
    pub rms: f32,
    /// Spectral centroid (Hz)
    pub spectral_centroid: f32,
    /// Spectral flux (change from previous frame)
    pub spectral_flux: f32,
    /// Raw waveform samples for this frame
    pub waveform: Vec<f32>,
}

/// Smoothed and normalized per-frame data (Pass 3 output), ready for GPU
#[derive(Clone, Debug)]
pub struct SmoothedFrame {
    /// FFT magnitude bins, smoothed and normalized (0.0-1.0)
    pub fft_bins: Vec<f32>,
    /// Simplified 3-band energies for uniforms (0.0-1.0)
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    /// RMS energy, normalized (0.0-1.0)
    pub rms: f32,
    /// Spectral centroid, normalized (0.0-1.0)
    pub spectral_centroid: f32,
    /// Spectral flux, normalized (0.0-1.0)
    pub spectral_flux: f32,
    /// Beat intensity (1.0 at onset, exponential decay)
    pub beat_intensity: f32,
    /// Beat phase (0.0-1.0 within current beat interval)
    pub beat_phase: f32,
    /// Is this frame on a beat onset?
    pub is_beat: bool,
    /// Waveform samples for this frame
    pub waveform: Vec<f32>,
    /// Time in seconds
    pub time: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct GlobalAnalysis {
    pub sample_rate: u32,
    pub total_samples: usize,
    pub duration: f32,
    pub peak_rms: f32,
    pub peak_amplitude: f32,
    pub beat_times: Vec<f32>,
    pub tempo_bpm: f32,
}
