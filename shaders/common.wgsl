// Common shader definitions for all sonica templates

struct FrameUniforms {
    resolution: vec2<f32>,
    time: f32,
    frame: u32,
    fps: f32,
    duration: f32,
    rms: f32,
    spectral_centroid: f32,
    spectral_flux: f32,
    beat_intensity: f32,
    beat_phase: f32,
    is_beat: f32,
    bass: f32,
    mid: f32,
    high: f32,
    _padding: f32,
};

@group(0) @binding(0) var<uniform> u: FrameUniforms;
@group(0) @binding(1) var<storage, read> fft_bins: array<f32>;
@group(0) @binding(2) var<storage, read> waveform: array<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle (3 vertices, no vertex buffer needed)
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    // Generate fullscreen triangle
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// Utility: map 0..1 to logarithmic frequency bin index
fn log_freq_index(t: f32, num_bins: u32) -> f32 {
    let min_freq = 20.0;
    let max_freq = 20000.0;
    let freq = min_freq * pow(max_freq / min_freq, t);
    let bin = freq / (u.resolution.x); // approximate, actual mapping uses sample_rate
    return clamp(freq / max_freq * f32(num_bins), 0.0, f32(num_bins) - 1.0);
}

// Utility: sample FFT with logarithmic interpolation
fn sample_fft_log(t: f32, num_bins: u32) -> f32 {
    let min_freq = 20.0;
    let max_freq = 20000.0;
    let freq = min_freq * pow(max_freq / min_freq, t);
    let bin_f = freq / max_freq * f32(num_bins);
    let bin_lo = u32(floor(bin_f));
    let bin_hi = min(bin_lo + 1u, num_bins - 1u);
    let frac = bin_f - floor(bin_f);
    let lo = fft_bins[bin_lo];
    let hi = fft_bins[bin_hi];
    return mix(lo, hi, frac);
}
