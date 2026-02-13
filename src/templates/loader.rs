use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

use super::manifest::TemplateManifest;

pub struct LoadedTemplate {
    pub manifest: TemplateManifest,
    pub fragment_shader: String,
    pub compute_shader: Option<String>,
}

/// Discover templates from built-in templates directory
pub fn find_templates_dir() -> PathBuf {
    // Look relative to the executable, then fall back to manifest dir
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    // Check next to executable
    if let Some(ref dir) = exe_dir {
        let templates_dir = dir.join("templates");
        if templates_dir.exists() {
            return templates_dir;
        }
        // Check parent (for target/debug layout)
        if let Some(parent) = dir.parent() {
            let templates_dir = parent.join("templates");
            if templates_dir.exists() {
                return templates_dir;
            }
            if let Some(grandparent) = parent.parent() {
                let templates_dir = grandparent.join("templates");
                if templates_dir.exists() {
                    return templates_dir;
                }
            }
        }
    }

    // Fall back to CARGO_MANIFEST_DIR (works in dev)
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("templates")
}

#[allow(dead_code)]
pub fn find_shaders_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    if let Some(ref dir) = exe_dir {
        let shaders_dir = dir.join("shaders");
        if shaders_dir.exists() {
            return shaders_dir;
        }
        if let Some(parent) = dir.parent() {
            let shaders_dir = parent.join("shaders");
            if shaders_dir.exists() {
                return shaders_dir;
            }
            if let Some(grandparent) = parent.parent() {
                let shaders_dir = grandparent.join("shaders");
                if shaders_dir.exists() {
                    return shaders_dir;
                }
            }
        }
    }

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("shaders")
}

pub fn list_templates() -> Result<Vec<String>> {
    let dir = find_templates_dir();
    let mut names = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let manifest_path = entry.path().join("manifest.json");
                if manifest_path.exists() {
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

pub fn load_template(name: &str) -> Result<LoadedTemplate> {
    let dir = find_templates_dir();
    let template_dir = dir.join(name);

    if !template_dir.exists() {
        anyhow::bail!(
            "Template '{}' not found. Available templates: {:?}",
            name,
            list_templates().unwrap_or_default()
        );
    }

    let manifest_path = template_dir.join("manifest.json");
    let manifest_str = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read manifest: {}", manifest_path.display()))?;
    let manifest: TemplateManifest = serde_json::from_str(&manifest_str)
        .with_context(|| format!("Failed to parse manifest: {}", manifest_path.display()))?;

    let fragment_path = template_dir.join(&manifest.shaders.fragment);
    let fragment_shader = std::fs::read_to_string(&fragment_path)
        .with_context(|| format!("Failed to read shader: {}", fragment_path.display()))?;

    let compute_shader = if let Some(ref compute_name) = manifest.shaders.compute {
        let compute_path = template_dir.join(compute_name);
        Some(std::fs::read_to_string(&compute_path)
            .with_context(|| format!("Failed to read compute shader: {}", compute_path.display()))?)
    } else {
        None
    };

    Ok(LoadedTemplate {
        manifest,
        fragment_shader,
        compute_shader,
    })
}

#[allow(dead_code)]
pub fn load_shared_shader(relative_path: &str) -> Result<String> {
    let dir = find_shaders_dir();
    let path = dir.join(relative_path);
    std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read shared shader: {}", path.display()))
}

/// Inject template parameters as WGSL const declarations prepended to the shader source.
pub fn inject_params(
    shader_src: &str,
    manifest: &TemplateManifest,
    overrides: &HashMap<String, String>,
) -> String {
    if manifest.parameters.is_empty() {
        return shader_src.to_string();
    }

    let mut consts = String::from("// Template parameters\n");

    for (name, param_def) in &manifest.parameters {
        let upper_name = name.to_uppercase();
        let value = overrides.get(name.as_str());

        match param_def.param_type.as_str() {
            "int" => {
                let v: i64 = value
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| param_def.default.as_i64().unwrap_or(0));
                consts.push_str(&format!("const PARAM_{}: i32 = {};\n", upper_name, v));
            }
            "float" => {
                let v: f64 = value
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| param_def.default.as_f64().unwrap_or(0.0));
                consts.push_str(&format!("const PARAM_{}: f32 = {:.6};\n", upper_name, v));
            }
            "bool" => {
                let v: bool = value
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or_else(|| param_def.default.as_bool().unwrap_or(false));
                consts.push_str(&format!(
                    "const PARAM_{}: i32 = {};\n",
                    upper_name,
                    if v { 1 } else { 0 }
                ));
            }
            "color" => {
                let (r, g, b) = if let Some(v) = value {
                    // Parse "r,g,b" or "#rrggbb"
                    let parts: Vec<f64> = v.split(':').filter_map(|s| s.parse().ok()).collect();
                    if parts.len() >= 3 {
                        (parts[0], parts[1], parts[2])
                    } else {
                        (0.0, 0.0, 0.0)
                    }
                } else if let Some(arr) = param_def.default.as_array() {
                    (
                        arr.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                        arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                        arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
                    )
                } else {
                    (0.0, 0.0, 0.0)
                };
                consts.push_str(&format!("const PARAM_{}_R: f32 = {:.6};\n", upper_name, r));
                consts.push_str(&format!("const PARAM_{}_G: f32 = {:.6};\n", upper_name, g));
                consts.push_str(&format!("const PARAM_{}_B: f32 = {:.6};\n", upper_name, b));
            }
            _ => {
                log::warn!("Unknown parameter type '{}' for '{}'", param_def.param_type, name);
            }
        }
    }

    consts.push('\n');
    format!("{}{}", consts, shader_src)
}
