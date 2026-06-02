use anyhow::Result;
use serde_json::Value;

use crate::migrations::migrate_language_setting;

pub fn remove_formatters_on_save(value: &mut Value) -> Result<()> {
    migrate_language_setting(value, remove_formatters_on_save_inner)
}

fn remove_formatters_on_save_inner(value: &mut Value, path: &[&str]) -> Result<()> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(format_on_save) = obj.get("format_on_save").cloned() else {
        return Ok(());
    };
    let is_format_on_save_set_to_formatter = format_on_save
        .as_str()
        .map_or(true, |s| s != "on" && s != "off");
    if !is_format_on_save_set_to_formatter {
        return Ok(());
    }

    fn fmt_path(path: &[&str], key: &str) -> String {
        let mut path = path.to_vec();
        path.push(key);
        path.join(".")
    }

    anyhow::ensure!(
        obj.get("formatter").is_none(),
        r#"在 "format_on_save" 和 "formatter" 中同时设置格式化工具已弃用。请将格式化工具从 {} 迁移到 {}"#,
        fmt_path(path, "format_on_save"),
        fmt_path(path, "formatter")
    );

    obj.insert("format_on_save".to_string(), serde_json::json!("on"));
    obj.insert("formatter".to_string(), format_on_save);

    Ok(())
}
