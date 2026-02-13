use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct TemplateManifest {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub shaders: ShaderPaths,
    #[serde(default)]
    pub default_effects: Vec<String>,
    #[serde(default)]
    pub parameters: HashMap<String, ParamDef>,
}

#[derive(Debug, Deserialize)]
pub struct ShaderPaths {
    pub fragment: String,
    #[serde(default)]
    pub compute: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ParamDef {
    #[serde(rename = "type")]
    pub param_type: String,
    pub default: serde_json::Value,
    #[serde(default)]
    pub min: Option<serde_json::Value>,
    #[serde(default)]
    pub max: Option<serde_json::Value>,
}
