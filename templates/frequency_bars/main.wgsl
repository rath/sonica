// Frequency Bars - Classic equalizer visualization
// Logarithmic frequency mapping with smooth color gradients

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

// HSV to RGB conversion
fn hsv2rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let c = v * s;
    let hp = h * 6.0;
    let x = c * (1.0 - abs(hp % 2.0 - 1.0));
    var rgb: vec3<f32>;
    if hp < 1.0 { rgb = vec3<f32>(c, x, 0.0); }
    else if hp < 2.0 { rgb = vec3<f32>(x, c, 0.0); }
    else if hp < 3.0 { rgb = vec3<f32>(0.0, c, x); }
    else if hp < 4.0 { rgb = vec3<f32>(0.0, x, c); }
    else if hp < 5.0 { rgb = vec3<f32>(x, 0.0, c); }
    else { rgb = vec3<f32>(c, 0.0, x); }
    let m = v - c;
    return rgb + vec3<f32>(m, m, m);
}

// Sample FFT with log frequency mapping
fn sample_fft_log(t: f32, num_bins: u32) -> f32 {
    let min_freq = 20.0;
    let max_freq = 20000.0;
    let freq = min_freq * pow(max_freq / min_freq, t);
    let bin_f = freq / max_freq * f32(num_bins);
    let bin_lo = u32(floor(bin_f));
    let bin_hi = min(bin_lo + 1u, num_bins - 1u);
    let frac = bin_f - floor(bin_f);

    if bin_lo >= num_bins {
        return 0.0;
    }

    let lo = fft_bins[bin_lo];
    let hi = fft_bins[min(bin_hi, num_bins - 1u)];
    return mix(lo, hi, frac);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let num_bins = arrayLength(&fft_bins);
    let bar_count = 64u;
    let gap_ratio = 0.2;
    let mirror = true;

    // Which bar are we in?
    var bar_uv_x = uv.x;
    if mirror {
        bar_uv_x = abs(uv.x - 0.5) * 2.0;
    }

    let bar_width = 1.0 / f32(bar_count);
    let bar_index = u32(floor(bar_uv_x / bar_width));
    let within_bar = (bar_uv_x % bar_width) / bar_width;

    // Gap between bars
    if within_bar > (1.0 - gap_ratio) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // Sample FFT for this bar (log mapping)
    let t = (f32(bar_index) + 0.5) / f32(bar_count);
    var height = sample_fft_log(t, num_bins);

    // Apply some power curve for visual impact
    height = pow(height, 0.7);

    // Beat reactive boost
    height = height * (1.0 + u.beat_intensity * 0.3);

    // Bar fill
    let y = 1.0 - uv.y; // flip Y so bars grow upward
    if y > height {
        // Background - subtle gradient
        let bg = vec3<f32>(0.02, 0.02, 0.05);
        return vec4<f32>(bg, 1.0);
    }

    // Color: hue based on frequency position, brightness based on height
    let hue = t * 0.6 + 0.55; // cyan → magenta range
    let sat = 0.8 + u.beat_intensity * 0.2;
    let val = 0.6 + (y / max(height, 0.001)) * 0.4;
    var color = hsv2rgb(hue % 1.0, sat, val);

    // Glow at the top of each bar
    let top_dist = abs(y - height);
    if top_dist < 0.015 {
        let glow = 1.0 - top_dist / 0.015;
        color = color + vec3<f32>(glow * 0.5);
    }

    // Subtle horizontal segments (retro look)
    let segment_y = y * 40.0;
    let segment_line = smoothstep(0.0, 0.1, abs(segment_y - round(segment_y)));
    color = color * (0.85 + 0.15 * segment_line);

    return vec4<f32>(color, 1.0);
}
