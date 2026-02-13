pub struct EmbeddedTemplate {
    pub manifest_json: &'static str,
    pub fragment_wgsl: &'static str,
}

pub fn embedded_templates() -> &'static [(&'static str, EmbeddedTemplate)] {
    &[
        (
            "circular_spectrum",
            EmbeddedTemplate {
                manifest_json: include_str!("../../templates/circular_spectrum/manifest.json"),
                fragment_wgsl: include_str!("../../templates/circular_spectrum/main.wgsl"),
            },
        ),
        (
            "frequency_bars",
            EmbeddedTemplate {
                manifest_json: include_str!("../../templates/frequency_bars/manifest.json"),
                fragment_wgsl: include_str!("../../templates/frequency_bars/main.wgsl"),
            },
        ),
        (
            "kaleidoscope",
            EmbeddedTemplate {
                manifest_json: include_str!("../../templates/kaleidoscope/manifest.json"),
                fragment_wgsl: include_str!("../../templates/kaleidoscope/main.wgsl"),
            },
        ),
        (
            "particle_burst",
            EmbeddedTemplate {
                manifest_json: include_str!("../../templates/particle_burst/manifest.json"),
                fragment_wgsl: include_str!("../../templates/particle_burst/main.wgsl"),
            },
        ),
        (
            "spectrogram",
            EmbeddedTemplate {
                manifest_json: include_str!("../../templates/spectrogram/manifest.json"),
                fragment_wgsl: include_str!("../../templates/spectrogram/main.wgsl"),
            },
        ),
        (
            "waveform_scope",
            EmbeddedTemplate {
                manifest_json: include_str!("../../templates/waveform_scope/manifest.json"),
                fragment_wgsl: include_str!("../../templates/waveform_scope/main.wgsl"),
            },
        ),
    ]
}

pub fn embedded_shared_shader(name: &str) -> Option<&'static str> {
    match name {
        "common.wgsl" => Some(include_str!("../../shaders/common.wgsl")),
        _ => None,
    }
}
