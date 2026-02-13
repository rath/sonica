// Spectrogram - scrolling time-frequency heatmap
// Since we can't do feedback textures without compute, we display the current
// frame's FFT as a column and use time-based positioning

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

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// Inferno-like colormap
fn inferno(t: f32) -> vec3<f32> {
    let t_c = clamp(t, 0.0, 1.0);
    // Simplified inferno
    let r = clamp(1.5 * t_c - 0.1, 0.0, 1.0);
    let g = clamp(sin(t_c * 3.14159) * 0.9, 0.0, 1.0);
    let b = clamp(sin(t_c * 1.5708) * 0.8, 0.0, 1.0);

    if t_c < 0.33 {
        return vec3<f32>(t_c * 1.2, 0.0, t_c * 2.5);
    } else if t_c < 0.66 {
        let s = (t_c - 0.33) * 3.0;
        return vec3<f32>(0.4 + s * 0.6, s * 0.5, 0.83 - s * 0.83);
    } else {
        let s = (t_c - 0.66) * 3.0;
        return vec3<f32>(1.0, 0.5 + s * 0.5, s * 0.3);
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let num_bins = arrayLength(&fft_bins);

    // Vertical axis = frequency (log scale), bottom = low freq
    let freq_t = 1.0 - uv.y;

    // Log frequency mapping
    let min_freq = 20.0;
    let max_freq = 20000.0;
    let freq = min_freq * pow(max_freq / min_freq, freq_t);
    let bin_f = freq / max_freq * f32(num_bins);
    let bin_lo = u32(floor(bin_f));
    let bin_hi = min(bin_lo + 1u, num_bins - 1u);

    var fft_val = 0.0;
    if bin_lo < num_bins {
        let frac = bin_f - floor(bin_f);
        fft_val = mix(fft_bins[bin_lo], fft_bins[min(bin_hi, num_bins - 1u)], frac);
    }

    // Current column position (rightmost = current time)
    let progress = u.time / max(u.duration, 0.001);
    let col_pos = progress;

    // Horizontal: current frame is drawn as a thin band at the current position
    // Everything else fades to black
    let dist_from_cursor = abs(uv.x - col_pos);

    // Show a narrow column of the current FFT
    let column_width = 1.5 / u.resolution.x;

    if dist_from_cursor < column_width {
        // Current column
        let intensity = pow(fft_val, 0.6);
        let color = inferno(intensity);
        return vec4<f32>(color, 1.0);
    }

    // Trail: fade based on distance from cursor
    let trail_length = 0.3 / PARAM_SCROLL_SPEED;
    let behind = col_pos - uv.x;
    if behind > 0.0 && behind < trail_length {
        let fade = 1.0 - behind / trail_length;
        let trail_intensity = pow(fft_val, 0.6) * fade * fade;
        let color = inferno(trail_intensity) * fade;
        return vec4<f32>(color, 1.0);
    }

    // Background with frequency scale hint
    let grid_lines = smoothstep(0.005, 0.0, abs(fract(freq_t * 10.0) - 0.5) - 0.48);
    let bg = vec3<f32>(0.01, 0.01, 0.02) + vec3<f32>(0.02) * grid_lines;

    return vec4<f32>(bg, 1.0);
}
