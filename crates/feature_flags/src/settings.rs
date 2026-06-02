use collections::HashMap;
use schemars::{Schema, json_schema};
use serde_json::{Map, Value};
use settings::{RegisterSetting, Settings, SettingsContent};

use crate::FeatureFlagStore;

#[derive(Clone, Debug, Default, RegisterSetting)]
pub struct FeatureFlagsSettings {
    pub overrides: HashMap<String, String>,
}

impl Settings for FeatureFlagsSettings {
    fn from_settings(content: &SettingsContent) -> Self {
        Self {
            overrides: content
                .feature_flags
                .as_ref()
                .map(|map| map.0.clone())
                .unwrap_or_default(),
        }
    }
}

/// Produces a JSON schema for the `feature_flags` object that lists each known
/// flag as a property with its variant keys as an `enum`.
///
/// Unknown flags are permitted via `additionalProperties: { "type": "string" }`,
/// so removing a flag from the binary never turns existing entries in
/// `settings.json` into validation errors.
pub fn generate_feature_flags_schema() -> Schema {
    let mut properties = Map::new();

    for descriptor in FeatureFlagStore::known_flags() {
        let variants = (descriptor.variants)();
        let enum_values: Vec<Value> = variants
            .iter()
            .map(|v| Value::String(v.override_key.to_string()))
            .collect();
        let enum_descriptions: Vec<Value> = variants
            .iter()
            .map(|v| Value::String(v.label.to_string()))
            .collect();

        let mut property = Map::new();
        property.insert("type".to_string(), Value::String("string".to_string()));
        property.insert("enum".to_string(), Value::Array(enum_values));
        // VS Code / json-language-server use `enumDescriptions` for hover docs
        // on each enum value; schemars passes them through untouched.
        property.insert(
            "enumDescriptions".to_string(),
            Value::Array(enum_descriptions),
        );
        property.insert(
            "description".to_string(),
            Value::String(format!(
                "`{}` 功能标志的覆盖值。默认值: `{}` ({} 变体)。",
                descriptor.name,
                (descriptor.default_variant_key)(),
                (descriptor.default_variant_key)(),
            )),
        );

        properties.insert(descriptor.name.to_string(), Value::Object(property));
    }

    json_schema!({
        "type": "object",
        "description": "功能标志的本地覆盖值, 以标志名称为键。",
        "properties": properties,
        "additionalProperties": {
            "type": "string",
            "description": "未知功能标志; 保留此项以免移除的标志导致设置验证失败。"
        }
    })
}
