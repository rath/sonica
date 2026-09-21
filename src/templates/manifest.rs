use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct TemplateManifest {
    #[allow(dead_code)]
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub shaders: ShaderPaths,
    #[serde(default)]
    pub default_effects: Vec<String>,
    #[serde(default)]
    pub parameters: HashMap<String, ParamDef>,
    /// Catch-all for keys the schema does not know about. Handwritten
    /// manifests add authoring notes/scratch keys; silently dropping them
    /// hid typos (e.g. `parameter` instead of `parameters`) that rendered
    /// fine but used defaults forever. Loaded templates warn on these.
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ShaderPaths {
    pub fragment: String,
    #[serde(default)]
    pub compute: Option<String>,
}

/// Declared parameter data types (manifest `"type"` field).
///
/// An enum (with serde's unknown-variant errors) instead of a bare string, so
/// `{ "type": "floot" }` fails at manifest parse time naming the offending
/// value, rather than skipping through a deep match later.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Int,
    Float,
    Bool,
    Color,
}

#[derive(Debug, Deserialize)]
pub struct ParamDef {
    #[serde(rename = "type")]
    pub param_type: ParamType,
    pub default: serde_json::Value,
    #[serde(default)]
    pub min: Option<serde_json::Value>,
    #[serde(default)]
    pub max: Option<serde_json::Value>,
}
