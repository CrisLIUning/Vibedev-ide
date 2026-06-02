use anyhow::Result;
use serde_json::Value;

use crate::migrations::migrate_language_setting;

pub fn restore_code_actions_on_format(value: &mut Value) -> Result<()> {
    migrate_language_setting(value, restore_code_actions_on_format_inner)
}

fn restore_code_actions_on_format_inner(value: &mut Value, path: &[&str]) -> Result<()> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    let code_actions_on_format = obj
        .get("code_actions_on_format")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));

    fn fmt_path(path: &[&str], key: &str) -> String {
        let mut path = path.to_vec();
        path.push(key);
        path.join(".")
    }

    let Some(mut code_actions_map) = code_actions_on_format.as_object().cloned() else {
        anyhow::bail!(
            r#"`code_actions_on_format` 处于无效状态,无法在 {} 处迁移。请确保 code_actions_on_format 设置为 Map<String, bool>"#,
            fmt_path(path, "code_actions_on_format"),
        );
    };

    let Some(formatter) = obj.get("formatter") else {
        return Ok(());
    };
    let formatter_array = if let Some(array) = formatter.as_array() {
        array.clone()
    } else {
        vec![formatter.clone()]
    };
    if formatter_array.is_empty() {
        return Ok(());
    }
    let mut code_action_formatters = Vec::new();
    for formatter in formatter_array {
        let Some(code_action) = formatter.get("code_action") else {
            return Ok(());
        };
        let Some(code_action_name) = code_action.as_str() else {
            anyhow::bail!(
                r#"`code_action` 处于无效状态,无法在 {} 处迁移。请确保 code_action 设置为 String"#,
                fmt_path(path, "formatter"),
            );
        };
        code_action_formatters.push(code_action_name.to_string());
    }

    code_actions_map.extend(
        code_action_formatters
            .into_iter()
            .rev()
            .map(|code_action| (code_action, Value::Bool(true))),
    );

    obj.insert("formatter".to_string(), Value::Array(vec![]));
    obj.insert(
        "code_actions_on_format".into(),
        Value::Object(code_actions_map),
    );

    Ok(())
}
