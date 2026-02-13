// Kaleidoscope - audio-reactive fractal/kaleidoscope patterns

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

const PI: f32 = 3.14159265;
const TWO_PI: f32 = 6.2831853;

// Kaleidoscope fold: mirror around N axes
fn kaleidoscope_fold(p: vec2<f32>, n: f32) -> vec2<f32> {
    let angle = atan2(p.y, p.x);
    let sector = TWO_PI / n;
    let folded_angle = abs(((angle % sector) + sector) % sector - sector * 0.5);
    let r = length(p);
    return vec2<f32>(cos(folded_angle), sin(folded_angle)) * r;
}

// Simple 2D noise-like pattern
fn pattern(p: vec2<f32>, t: f32) -> f32 {
    let v1 = sin(p.x * 3.0 + t) * cos(p.y * 2.7 - t * 0.7);
    let v2 = sin(length(p) * 5.0 - t * 1.3);
    let v3 = cos(p.x * p.y * 2.0 + t * 0.5);
    return (v1 + v2 + v3) / 3.0;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / u.resolution.y;
    var p = (in.uv - vec2<f32>(0.5)) * 2.0;
    p.x *= aspect;

    // Audio-reactive zoom
    let zoom = PARAM_ZOOM + 0.5 + u.bass * 0.5 + u.beat_intensity * 0.3;
    p *= zoom;

    // Rotation driven by time and spectral centroid
    let rot_speed = 0.2 + u.spectral_centroid * 0.3;
    let rot_angle = u.time * rot_speed;
    let cs = cos(rot_angle);
    let sn = sin(rot_angle);
    p = vec2<f32>(p.x * cs - p.y * sn, p.x * sn + p.y * cs);

    // Kaleidoscope symmetry
    let symmetry = f32(PARAM_SYMMETRY);
    let kp = kaleidoscope_fold(p, symmetry);

    // Layer 1: base pattern
    let t1 = u.time * 0.3;
    let p1 = pattern(kp * (1.0 + u.mid * 0.5), t1);

    // Layer 2: detail
    let t2 = u.time * 0.5;
    let p2 = pattern(kp * 2.5 + vec2<f32>(t2, -t2), t2);

    // Layer 3: fine detail
    let p3 = pattern(kp * 5.0, u.time * 0.7) * u.high;

    let combined = p1 * 0.5 + p2 * 0.3 + p3 * 0.2;

    // Color mapping
    let hue = fract(combined * 0.5 + u.time * 0.05 + u.spectral_centroid * 0.2);
    let sat = 0.7 + u.rms * 0.3;
    let val = 0.3 + abs(combined) * 0.5 + u.beat_intensity * 0.2;

    var color = hsv2rgb(hue, sat, clamp(val, 0.0, 1.0));

    // Beat flash
    color += vec3<f32>(u.beat_intensity * 0.1);

    // Radial fade
    let center_dist = length(in.uv - vec2<f32>(0.5)) * 2.0;
    let fade = 1.0 - smoothstep(0.8, 1.5, center_dist);
    color *= fade;

    return vec4<f32>(color, 1.0);
}
