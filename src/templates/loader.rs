use anyhow::{Context, Result};
use std::path::PathBuf;

use super::manifest::TemplateManifest;

#[allow(dead_code)]
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
