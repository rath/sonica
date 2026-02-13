// Particle Burst - procedural particles driven by audio
// Uses deterministic pseudo-random to simulate particles without compute shader

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

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(hash21(p), hash21(p + vec2<f32>(127.1, 311.7)));
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / u.resolution.y;
    var p = in.uv - vec2<f32>(0.5);
    p.x *= aspect;

    var color = vec3<f32>(0.01, 0.01, 0.02);
    let particle_count = 200;

    for (var i = 0; i < particle_count; i++) {
        let seed = vec2<f32>(f32(i) * 0.37, f32(i) * 0.73);
        let rnd = hash22(seed);

        // Particle lifetime based on time and index
        let speed = 0.3 + rnd.x * 0.7;
        let life_cycle = 3.0 + rnd.y * 2.0; // seconds per cycle
        let phase = fract(u.time * speed / life_cycle + rnd.x);

        // Angle from center
        let base_angle = rnd.x * TWO_PI;
        let angle = base_angle + sin(u.time * 0.5 + f32(i)) * 0.5;

        // Distance from center: grows with phase, influenced by bass
        let energy = u.bass * 0.5 + u.rms * 0.5;
        let max_dist = 0.3 + energy * 0.3;
        let dist = phase * max_dist;

        // Beat burst: particles jump outward on beats
        let burst = u.beat_intensity * 0.15 * rnd.y;

        let particle_pos = vec2<f32>(
            cos(angle) * (dist + burst),
            sin(angle) * (dist + burst)
        );

        let d = length(p - particle_pos);

        // Particle size: small, fades with phase
        let fade = 1.0 - phase;
        let size = (0.003 + rnd.y * 0.004) * (0.5 + fade * 0.5);
        let brightness = smoothstep(size, 0.0, d) * fade * fade;

        // Color: varies per particle
        let hue = fract(rnd.x * 0.6 + u.spectral_centroid * 0.3 + u.time * 0.05);
        let particle_color = hsv2rgb(hue, 0.7 + rnd.y * 0.3, 1.0);

        color += particle_color * brightness * 0.8;

        // Glow
        let glow = exp(-d * d / (size * size * 16.0)) * fade * 0.15;
        color += particle_color * glow;
    }

    // Central glow
    let center_dist = length(p);
    let center_glow = exp(-center_dist * center_dist * 20.0) * u.rms * 0.3;
    let center_color = hsv2rgb(0.6 + u.spectral_centroid * 0.3, 0.4, 1.0);
    color += center_color * center_glow;

    return vec4<f32>(color, 1.0);
}
