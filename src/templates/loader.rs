use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

use super::embedded;
use super::manifest::{ParamDef, ParamType, TemplateManifest};

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
    warn_unknown_manifest_keys(
        &manifest,
        &template_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );

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
    warn_unknown_manifest_keys(&manifest, name);

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

/// Inject template parameters as WGSL const declarations prepended to the
/// shader source.
///
/// Invalid `--param` values used to fall back to manifest defaults silently
/// and unknown override keys were dropped without any feedback, so a typo
/// produced a video that quietly ignored the request. Parsing is now strict:
/// an unknown key, an unparsable value, or an out-of-i32-range integer throws
/// an error naming the offending key. Manifest-declared `min`/`max` clamps,
/// which were deserialized but dead, are now enforced to keep overrides inside
/// what the shader was designed for.
pub fn inject_params(
    shader_src: &str,
    manifest: &TemplateManifest,
    overrides: &HashMap<String, String>,
) -> Result<String> {
    for key in overrides.keys() {
        validate_wgsl_identifier(key)?;
        if !manifest.parameters.contains_key(key.as_str()) {
            let mut known: Vec<&str> =
                manifest.parameters.keys().map(|k| k.as_str()).collect();
            known.sort_unstable();
            anyhow::bail!(
                "Unknown template parameter '{key}'. Declared parameters: {}",
                if known.is_empty() {
                    "(this template declares none)".to_string()
                } else {
                    known.join(", ")
                }
            );
        }
        validate_wgsl_identifier(key)?;
    }

    // Manifest keys must be valid WGSL identifiers even without overrides, so
    // a declaration like "bar count" fails at load time, not at shader compile.
    for name in manifest.parameters.keys() {
        validate_wgsl_identifier(name).with_context(|| {
            format!("Template declares an invalid parameter name '{name}'")
        })?;
    }

    let mut consts = String::from("// Template parameters\n");

    for (name, param_def) in &manifest.parameters {
        let upper_name = name.to_uppercase();
        let value = overrides.get(name.as_str());

        match param_def.param_type {
            ParamType::Int => {
                let v: i64 = match value {
                    Some(v) => v.parse().with_context(|| {
                        format!("Invalid --param {name}={v}: expected an integer")
                    })?,
                    None => param_def.default.as_i64().unwrap_or(0),
                };
                let clamped = manifest_clamp(param_def, v as f64, name);
                if !(i32::MIN as f64..=i32::MAX as f64).contains(&clamped) {
                    anyhow::bail!(
                        "--param {name}={} is outside the i32 range after manifest clamping",
                        clamped as i64
                    );
                }
                consts.push_str(&format!("const PARAM_{}: i32 = {};\n", upper_name, clamped as i64));
            }
            ParamType::Float => {
                let v: f64 = match value {
                    Some(v) => v.parse().with_context(|| {
                        format!("Invalid --param {name}={v}: expected a float")
                    })?,
                    None => param_def.default.as_f64().unwrap_or(0.0),
                };
                let clamped = manifest_clamp(param_def, v, name);
                if !clamped.is_finite() {
                    anyhow::bail!("--param {name}={v} is not a finite float");
                }
                consts.push_str(&format!("const PARAM_{}: f32 = {:.6};\n", upper_name, clamped));
            }
            ParamType::Bool => {
                let v = match value.map(String::as_str) {
                    Some("true" | "1") => true,
                    Some("false" | "0") => false,
                    Some(other) => anyhow::bail!(
                        "Invalid --param {name}={other}: expected true/false (or 1/0)"
                    ),
                    None => param_def.default.as_bool().unwrap_or(false),
                };
                consts.push_str(&format!(
                    "const PARAM_{}: i32 = {};\n",
                    upper_name,
                    if v { 1 } else { 0 }
                ));
            }
            ParamType::Color => {
                let (r, g, b) = if let Some(v) = value {
                    let parts: Vec<f64> = v
                        .split(':')
                        .map(|part| {
                            part.trim().parse().with_context(|| {
                                format!("Invalid --param {name}={v}: expected r:g:b floats")
                            })
                        })
                        .collect::<Result<Vec<f64>>>()?;
                    if parts.len() != 3 {
                        anyhow::bail!(
                            "Invalid --param {name}={v}: expected 3 colon-separated values (r:g:b), got {}",
                            parts.len()
                        );
                    }
                    (parts[0], parts[1], parts[2])
                } else if let Some(arr) = param_def.default.as_array() {
                    (
                        arr.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                        arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                        arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
                    )
                } else {
                    log::warn!(
                        "Color parameter '{name}' has no usable default; emitting black"
                    );
                    (0.0, 0.0, 0.0)
                };
                consts.push_str(&format!("const PARAM_{}_R: f32 = {:.6};\n", upper_name, r));
                consts.push_str(&format!("const PARAM_{}_G: f32 = {:.6};\n", upper_name, g));
                consts.push_str(&format!("const PARAM_{}_B: f32 = {:.6};\n", upper_name, b));
            }
        }
    }

    consts.push('\n');
    Ok(format!("{}{}", consts, shader_src))
}

/// Clamp against the manifest's (previously dead) min/max declarations.
fn manifest_clamp(param_def: &ParamDef, value: f64, name: &str) -> f64 {
    let min = param_def.min.as_ref().and_then(|m| m.as_f64());
    let max = param_def.max.as_ref().and_then(|m| m.as_f64());
    let clamped = if value < min.unwrap_or(f64::MIN) {
        min.unwrap_or(value)
    } else if value > max.unwrap_or(f64::MAX) {
        max.unwrap_or(value)
    } else {
        value
    };
    if clamped != value {
        log::info!(
            "--param {}={} clamped to {} by the manifest min/max",
            name,
            value,
            clamped
        );
    }
    clamped
}

/// Surface manifest keys the schema does not recognize — a typo (`parameter`,
/// `deafults`) used to fall through silently and was invisible for months.
fn warn_unknown_manifest_keys(manifest: &TemplateManifest, template: &str) {
    if !manifest.unknown.is_empty() {
        let keys: Vec<String> = manifest.unknown.keys().cloned().collect();
        log::warn!(
            "Template '{}' manifest has unknown keys (ignored): {}",
            template,
            keys.join(", ")
        );
    }
}

fn validate_wgsl_identifier(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.chars().enumerate().all(|(index, c)| {
            c == '_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_alphanumeric())
        });
    if !valid {
        anyhow::bail!("--param key '{name}' is not a valid WGSL identifier");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{ParamDef, ShaderPaths};
    use super::*;

    fn manifest_with(parameters: Vec<(&str, &str, serde_json::Value)>) -> TemplateManifest {
        TemplateManifest {
            name: "test".into(),
            display_name: "Test".into(),
            description: String::new(),
            shaders: ShaderPaths {
                fragment: "main.wgsl".into(),
                compute: None,
            },
            default_effects: vec![],
            parameters: parameters
                .into_iter()
                .map(|(name, kind, default)| {
                    let param_type = match kind {
                        "int" => ParamType::Int,
                        "float" => ParamType::Float,
                        "bool" => ParamType::Bool,
                        "color" => ParamType::Color,
                        other => panic!("test helper: unknown param kind '{other}'"),
                    };
                    (
                        name.to_string(),
                        ParamDef {
                            param_type,
                            default,
                            min: None,
                            max: None,
                        },
                    )
                })
                .collect(),
            unknown: HashMap::new(),
        }
    }

    fn test_shader() -> &'static str {
        "@fragment fn fs_main() {}"
    }

    #[test]
    fn emits_consts_for_overrides() {
        let manifest = manifest_with(vec![("bar_count", "int", serde_json::json!(64))]);
        let overrides: HashMap<String, String> =
            [("bar_count".to_string(), "128".to_string())].into_iter().collect();

        let src = inject_params(test_shader(), &manifest, &overrides).unwrap();

        assert!(src.contains("const PARAM_BAR_COUNT: i32 = 128;"));
        assert!(src.ends_with(test_shader()));
    }

    #[test]
    fn rejects_unknown_override_keys() {
        let manifest = manifest_with(vec![]);
        let overrides: HashMap<String, String> =
            [("bark_count".to_string(), "12".to_string())].into_iter().collect();

        let err = inject_params(test_shader(), &manifest, &overrides).unwrap_err();

        assert!(format!("{err:#}").contains("Unknown template parameter 'bark_count'"));
    }

    #[test]
    fn rejects_unparsable_values_instead_of_falling_back() {
        let manifest = manifest_with(vec![
            ("density", "float", serde_json::json!(0.5)),
            ("mirror", "bool", serde_json::json!(true)),
            ("bars", "int", serde_json::json!(8)),
        ]);

        let bad_float: HashMap<String, String> =
            [("density".to_string(), "thick".to_string())].into_iter().collect();
        assert!(format!("{err:#}", err = inject_params(test_shader(), &manifest, &bad_float).unwrap_err())
            .contains("Invalid --param density=thick"));

        let bad_int: HashMap<String, String> =
            [("bars".to_string(), "12.5".to_string())].into_iter().collect();
        assert!(format!("{err:#}", err = inject_params(test_shader(), &manifest, &bad_int).unwrap_err())
            .contains("expected an integer"));

        let bad_bool: HashMap<String, String> =
            [("mirror".to_string(), "maybe".to_string())].into_iter().collect();
        assert!(format!("{err:#}", err = inject_params(test_shader(), &manifest, &bad_bool).unwrap_err())
            .contains("expected true/false"));
    }

    #[test]
    fn clamps_overrides_to_manifest_min_max() {
        let mut manifest = manifest_with(vec![("brightness", "float", serde_json::json!(1.0))]);
        manifest.parameters.get_mut("brightness").unwrap().max = Some(serde_json::json!(2.0));
        let overrides: HashMap<String, String> =
            [("brightness".to_string(), "9.0".to_string())].into_iter().collect();

        let src = inject_params(test_shader(), &manifest, &overrides).unwrap();

        assert!(src.contains("const PARAM_BRIGHTNESS: f32 = 2.000000"));
    }

    #[test]
    fn rejects_int_overflowing_i32() {
        let manifest = manifest_with(vec![("bars", "int", serde_json::json!(8))]);
        let overrides: HashMap<String, String> =
            [("bars".to_string(), "99999999999".to_string())].into_iter().collect();

        let err = inject_params(test_shader(), &manifest, &overrides).unwrap_err();

        assert!(format!("{err:#}").contains("outside the i32 range"));
    }

    #[test]
    fn rejects_non_identifier_keys() {
        let manifest = manifest_with(vec![]);
        let overrides: HashMap<String, String> =
            [("bar count".to_string(), "1".to_string())].into_iter().collect();

        let err = inject_params(test_shader(), &manifest, &overrides).unwrap_err();

        assert!(format!("{err:#}").contains("not a valid WGSL identifier"));
    }
}

/// Compiled-time validated: every embedded template's WGSL (and its
/// `#import` composition with the shared shader) must pass full naga
/// validation, matching what runtime backends enforce.
#[test]
fn embedded_template_shaders_pass_strict_wgsl_validation() {
    let validate = |shader: &str, name: &str| {
        let module = naga::front::wgsl::parse_str(shader)
            .unwrap_or_else(|err| panic!("'{name}' WGSL parse error: {err}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("'{name}' WGSL validation error: {err}"));
    };

    validate(
        embedded::embedded_shared_shader("common.wgsl").unwrap(),
        "shaders/common.wgsl",
    );

    for (name, template) in embedded::embedded_templates() {
        let manifest: super::manifest::TemplateManifest = parse_manifest_json(
            template.manifest_json,
        );
        let shader = inject_params(
            template.fragment_wgsl,
            &manifest,
            &std::collections::HashMap::new(),
        )
        .unwrap_or_else(|err| panic!("'{name}' parameter injection failed: {err:#}"));
        let composed = preprocess_imports(&shader)
            .unwrap_or_else(|err| panic!("'{name}' import processing failed: {err}"));
        validate(&composed, name);
    }
}

/// Parse a template manifest from its embedded JSON.
#[cfg(test)]
fn parse_manifest_json(manifest_json: &str) -> super::manifest::TemplateManifest {
    serde_json::from_str(manifest_json).expect("embedded template manifest JSON parses")
}
