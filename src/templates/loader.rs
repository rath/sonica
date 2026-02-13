use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

use super::embedded;
use super::manifest::TemplateManifest;

pub struct LoadedTemplate {
    pub manifest: TemplateManifest,
    pub fragment_shader: String,
    pub compute_shader: Option<String>,
}

/// Discover templates from built-in templates directory
fn find_templates_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    // Check next to executable
    if let Some(ref dir) = exe_dir {
        let templates_dir = dir.join("templates");
        if templates_dir.exists() {
            return Some(templates_dir);
        }
        // Check parent (for target/debug layout)
        if let Some(parent) = dir.parent() {
            let templates_dir = parent.join("templates");
            if templates_dir.exists() {
                return Some(templates_dir);
            }
            if let Some(grandparent) = parent.parent() {
                let templates_dir = grandparent.join("templates");
                if templates_dir.exists() {
                    return Some(templates_dir);
                }
            }
        }
    }

    // Fall back to CARGO_MANIFEST_DIR (works in dev)
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let dir = PathBuf::from(manifest_dir).join("templates");
    if dir.exists() {
        return Some(dir);
    }

    None
}

fn find_shaders_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    if let Some(ref dir) = exe_dir {
        let shaders_dir = dir.join("shaders");
        if shaders_dir.exists() {
            return Some(shaders_dir);
        }
        if let Some(parent) = dir.parent() {
            let shaders_dir = parent.join("shaders");
            if shaders_dir.exists() {
                return Some(shaders_dir);
            }
            if let Some(grandparent) = parent.parent() {
                let shaders_dir = grandparent.join("shaders");
                if shaders_dir.exists() {
                    return Some(shaders_dir);
                }
            }
        }
    }

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let dir = PathBuf::from(manifest_dir).join("shaders");
    if dir.exists() {
        return Some(dir);
    }

    None
}

pub fn list_templates() -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();

    // Filesystem templates
    if let Some(dir) = find_templates_dir() {
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

    // Embedded templates (add any not already found on filesystem)
    for (name, _) in embedded::embedded_templates() {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }

    names.sort();
    Ok(names)
}

pub fn load_template(name: &str) -> Result<LoadedTemplate> {
    // Try filesystem first
    if let Some(loaded) = try_load_template_fs(name)? {
        return Ok(loaded);
    }

    // Fall back to embedded
    load_template_embedded(name)
}

fn try_load_template_fs(name: &str) -> Result<Option<LoadedTemplate>> {
    let dir = match find_templates_dir() {
        Some(d) => d,
        None => return Ok(None),
    };

    let template_dir = dir.join(name);
    if !template_dir.exists() {
        return Ok(None);
    }

    let manifest_path = template_dir.join("manifest.json");
    let manifest_str = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read manifest: {}", manifest_path.display()))?;
    let manifest: TemplateManifest = serde_json::from_str(&manifest_str)
        .with_context(|| format!("Failed to parse manifest: {}", manifest_path.display()))?;

    let fragment_path = template_dir.join(&manifest.shaders.fragment);
    let fragment_raw = std::fs::read_to_string(&fragment_path)
        .with_context(|| format!("Failed to read shader: {}", fragment_path.display()))?;
    let fragment_shader = preprocess_imports(&fragment_raw)?;

    let compute_shader = if let Some(ref compute_name) = manifest.shaders.compute {
        let compute_path = template_dir.join(compute_name);
        let raw = std::fs::read_to_string(&compute_path)
            .with_context(|| format!("Failed to read compute shader: {}", compute_path.display()))?;
        Some(preprocess_imports(&raw)?)
    } else {
        None
    };

    Ok(Some(LoadedTemplate {
        manifest,
        fragment_shader,
        compute_shader,
    }))
}

fn load_template_embedded(name: &str) -> Result<LoadedTemplate> {
    let tmpl = embedded::embedded_templates()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t);

    let tmpl = match tmpl {
        Some(t) => t,
        None => {
            anyhow::bail!(
                "Template '{}' not found. Available templates: {:?}",
                name,
                list_templates().unwrap_or_default()
            );
        }
    };

    let manifest: TemplateManifest = serde_json::from_str(tmpl.manifest_json)
        .with_context(|| format!("Failed to parse embedded manifest for '{}'", name))?;

    let fragment_shader = preprocess_imports(tmpl.fragment_wgsl)?;

    Ok(LoadedTemplate {
        manifest,
        fragment_shader,
        compute_shader: None,
    })
}

pub fn load_shared_shader(relative_path: &str) -> Result<String> {
    // Try filesystem first
    if let Some(dir) = find_shaders_dir() {
        let path = dir.join(relative_path);
        if path.exists() {
            return std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read shared shader: {}", path.display()));
        }
    }

    // Fall back to embedded
    embedded::embedded_shared_shader(relative_path)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Shared shader '{}' not found", relative_path))
}

/// Process `// #import "filename.wgsl"` directives by replacing them with shared shader contents.
pub fn preprocess_imports(shader_src: &str) -> Result<String> {
    let mut result = String::with_capacity(shader_src.len());
    for line in shader_src.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("// #import \"") {
            if let Some(filename) = rest.strip_suffix('"') {
                let shared = load_shared_shader(filename)?;
                result.push_str(&shared);
                result.push('\n');
                continue;
            }
        }
        result.push_str(line);
        result.push('\n');
    }
    Ok(result)
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
