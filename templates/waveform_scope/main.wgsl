// Waveform Scope - PCM oscilloscope with glow

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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let num_samples = arrayLength(&waveform);

    // Background: dark with subtle grid
    var color = vec3<f32>(0.02, 0.02, 0.04);

    // Grid lines
    let grid_x = abs(fract(uv.x * 10.0) - 0.5);
    let grid_y = abs(fract(uv.y * 8.0) - 0.5);
    let grid = smoothstep(0.0, 0.02, min(grid_x, grid_y));
    color = mix(vec3<f32>(0.06, 0.06, 0.1), color, grid);

    // Center line
    let center_dist = abs(uv.y - 0.5);
    let center_line = smoothstep(0.002, 0.0, center_dist);
    color = mix(color, vec3<f32>(0.1, 0.1, 0.15), center_line * 0.5);

    // Sample the waveform
    let sample_idx_f = uv.x * f32(num_samples - 1u);
    let idx0 = u32(floor(sample_idx_f));
    let idx1 = min(idx0 + 1u, num_samples - 1u);
    let frac = sample_idx_f - floor(sample_idx_f);

    let sample_val = mix(waveform[idx0], waveform[idx1], frac);
    let wave_y = 0.5 - sample_val * 0.4;

    let dist = abs(uv.y - wave_y);

    // Line thickness based on RMS
    let base_thickness = 2.0 / u.resolution.y;
    let thickness = base_thickness * (1.0 + u.rms * 2.0);

    // Main line
    let line = smoothstep(thickness, 0.0, dist);

    // Glow
    let glow_size = thickness * 8.0;
    let glow = exp(-dist * dist / (glow_size * glow_size)) * 0.6;

    // Color based on spectral centroid (hue shift)
    let hue = 0.45 + u.spectral_centroid * 0.3;
    let wave_color = hsv2rgb(hue, 0.8, 1.0);
    let glow_color = hsv2rgb(hue + 0.1, 0.6, 0.8);

    color += wave_color * line;
    color += glow_color * glow * (0.5 + u.beat_intensity * 0.5);

    return vec4<f32>(color, 1.0);
}
