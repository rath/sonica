// Circular Spectrum - radial frequency display with beat-reactive radius

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
    return rgb + vec3<f32>(m);
}

fn sample_fft_log(t: f32, num_bins: u32) -> f32 {
    let min_freq = 20.0;
    let max_freq = 20000.0;
    let freq = min_freq * pow(max_freq / min_freq, t);
    let bin_f = freq / max_freq * f32(num_bins);
    let bin_lo = u32(floor(bin_f));
    let bin_hi = min(bin_lo + 1u, num_bins - 1u);
    if bin_lo >= num_bins { return 0.0; }
    let frac = bin_f - floor(bin_f);
    return mix(fft_bins[bin_lo], fft_bins[min(bin_hi, num_bins - 1u)], frac);
}

const PI: f32 = 3.14159265;
const TWO_PI: f32 = 6.2831853;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let num_bins = arrayLength(&fft_bins);
    let aspect = u.resolution.x / u.resolution.y;

    // Center and correct aspect ratio
    var p = in.uv - vec2<f32>(0.5);
    p.x *= aspect;

    let dist = length(p);
    let angle = atan2(p.y, p.x);
    let norm_angle = (angle + PI) / TWO_PI; // 0..1

    // Inner radius with beat pulse
    let inner_r = PARAM_INNER_RADIUS + u.beat_intensity * 0.06;

    // Sample FFT at this angle
    let fft_val = sample_fft_log(norm_angle, num_bins);
    let bar_height = pow(fft_val, 0.6) * 0.35 * (1.0 + u.rms * 1.0);
    let outer_r = inner_r + bar_height;

    // Background
    var color = vec3<f32>(0.01, 0.01, 0.03);

    // Ring glow
    let ring_dist = abs(dist - inner_r);
    let ring_glow = exp(-ring_dist * ring_dist * 800.0) * 0.3;
    let ring_color = hsv2rgb(0.6 + u.spectral_centroid * 0.3, 0.5, 0.5);
    color += ring_color * ring_glow;

    // Bars
    if dist >= inner_r && dist <= outer_r {
        let fill = (dist - inner_r) / max(outer_r - inner_r, 0.001);

        // Color: hue varies with angle, brightness with fill
        let hue = norm_angle * 0.8 + 0.5 + u.time * 0.02;
        let val = 0.5 + fill * 0.5;
        let bar_color = hsv2rgb(hue % 1.0, 0.85, val);
        color = bar_color;
    }

    // Outer glow for bars
    if dist > outer_r && dist < outer_r + 0.04 {
        let glow_t = 1.0 - (dist - outer_r) / 0.04;
        let hue = norm_angle * 0.8 + 0.5 + u.time * 0.02;
        let glow_c = hsv2rgb(hue % 1.0, 0.6, 0.8);
        color += glow_c * glow_t * glow_t * fft_val;
    }

    // Center circle
    if dist < inner_r * 0.85 {
        let center_brightness = 0.03 + u.rms * 0.05;
        color = vec3<f32>(center_brightness);

        // Subtle inner pattern
        let inner_pattern = sin(angle * 8.0 + u.time * 2.0) * 0.5 + 0.5;
        color += hsv2rgb(0.6, 0.3, 0.02) * inner_pattern * u.bass;
    }

    return vec4<f32>(color, 1.0);
}
