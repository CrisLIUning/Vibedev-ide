use anyhow::Result;
use serde_json::Value;

use crate::migrations::migrate_settings;

pub fn make_file_finder_include_ignored_an_enum(value: &mut Value) -> Result<()> {
    migrate_settings(value, &mut migrate_one)
}

fn migrate_one(obj: &mut serde_json::Map<String, Value>) -> Result<()> {
    let Some(file_finder) = obj.get_mut("file_finder") else {
        return Ok(());
    };

    let Some(file_finder_obj) = file_finder.as_object_mut() else {
        anyhow::bail!("file_finder 应为一个对象");
    };

    let Some(include_ignored) = file_finder_obj.get_mut("include_ignored") else {
        return Ok(());
    };
    *include_ignored = match include_ignored {
        Value::Bool(true) => Value::String("all".to_string()),
        Value::Bool(false) => Value::String("indexed".to_string()),
        Value::Null => Value::String("smart".to_string()),
        Value::String(s) if s == "all" || s == "indexed" || s == "smart" => return Ok(()),
        _ => anyhow::bail!("include_ignored 应为布尔值或 null"),
    };
    Ok(())
}
