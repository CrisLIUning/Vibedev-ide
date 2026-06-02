use anyhow::Result;
use serde_json::Value;

use crate::migrations::migrate_settings;

const AGENT_SERVERS_KEY: &str = "agent_servers";

// VIBEDEV: heal the "Custom agent server `VibeDev` is not registered" trap.
//
// VibeDev is a *baked* custom agent — it only ever exists as
// `agent_servers.VibeDev = { type: "custom", command: ... }` in the fork's
// default.json (see assets/settings/default.json + PATCHES.md row #8).
//
// Zed's agent-settings UI, however, serializes any interaction with an agent
// (e.g. changing its session mode) as a *registry* reference written into the
// user's settings.json: `agent_servers.VibeDev = { type: "registry", ... }`.
// Because user settings override defaults and enum MergeFrom replaces wholesale
// (see settings_content CustomAgentServerSettings — enum merge is
// `*self = other.clone()`), that registry entry shadows the baked custom one.
// At registration time the agent then takes the Registry branch in
// `project::agent_server_store::reregister_agents`, fails to find "VibeDev" in
// the ACP registry catalog (it isn't a registry agent), is skipped, and the
// agent panel reports "Custom agent server `VibeDev` is not registered".
//
// The fix: strip a user-settings `VibeDev` entry that is registry-typed (or
// otherwise has no `command`), so the baked custom definition takes over again.
// We deliberately do NOT touch a user entry that has its own `command` — that
// is a legitimate dev-checkout override (PATCHES.md trap #4 says devs may point
// VibeDev at a local dist), and removing it would surprise them.
pub fn heal_vibedev_registry_override(value: &mut Value) -> Result<()> {
    migrate_settings(value, &mut migrate_one)
}

fn migrate_one(obj: &mut serde_json::Map<String, Value>) -> Result<()> {
    let Some(agent_servers) = obj.get_mut(AGENT_SERVERS_KEY) else {
        return Ok(());
    };
    let Some(servers_map) = agent_servers.as_object_mut() else {
        return Ok(());
    };

    let should_strip = servers_map
        .get("VibeDev")
        .and_then(|v| v.as_object())
        .is_some_and(|vibedev| {
            let is_registry = vibedev.get("type").and_then(|t| t.as_str()) == Some("registry");
            let has_command = vibedev.contains_key("command");
            // Strip when the entry is registry-typed, OR has neither a type nor
            // a command (an empty/half-written entry that still shadows the
            // baked custom def). Keep any entry that carries its own command.
            (is_registry || !vibedev.contains_key("type")) && !has_command
        });

    if should_strip {
        servers_map.remove("VibeDev");
        // If VibeDev was the only configured agent server, drop the now-empty
        // map so the user's settings.json doesn't keep a dangling `{}`.
        if servers_map.is_empty() {
            obj.remove(AGENT_SERVERS_KEY);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str) -> Value {
        let mut v: Value = serde_json::from_str(input).unwrap();
        heal_vibedev_registry_override(&mut v).unwrap();
        v
    }

    #[test]
    fn strips_registry_typed_vibedev() {
        let out = run(
            r#"{
                "agent_servers": {
                    "VibeDev": {
                        "type": "registry",
                        "default_config_options": { "mode": "auto" }
                    }
                },
                "vim_mode": false
            }"#,
        );
        // VibeDev stripped; since it was the only agent server, the whole key
        // is removed → baked custom default.json def takes over.
        assert!(out.get("agent_servers").is_none());
        assert_eq!(out.get("vim_mode"), Some(&Value::Bool(false)));
    }

    #[test]
    fn keeps_other_agent_servers_when_stripping_vibedev() {
        let out = run(
            r#"{
                "agent_servers": {
                    "VibeDev": { "type": "registry" },
                    "my-agent": { "type": "custom", "command": "/bin/foo" }
                }
            }"#,
        );
        let servers = out.get("agent_servers").unwrap().as_object().unwrap();
        assert!(servers.get("VibeDev").is_none());
        assert!(servers.get("my-agent").is_some());
    }

    #[test]
    fn preserves_a_real_custom_vibedev_override() {
        // A dev who points VibeDev at a local dist (has its own command) keeps it.
        let out = run(
            r#"{
                "agent_servers": {
                    "VibeDev": {
                        "type": "custom",
                        "command": "C:/dev/bun.exe",
                        "args": ["cli.js", "--acp"]
                    }
                }
            }"#,
        );
        let vibedev = out
            .get("agent_servers")
            .unwrap()
            .get("VibeDev")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            vibedev.get("command").and_then(|c| c.as_str()),
            Some("C:/dev/bun.exe")
        );
    }

    #[test]
    fn noop_when_no_agent_servers() {
        let out = run(r#"{ "vim_mode": true }"#);
        assert_eq!(out.get("vim_mode"), Some(&Value::Bool(true)));
        assert!(out.get("agent_servers").is_none());
    }
}
