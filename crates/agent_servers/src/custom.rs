use crate::{AgentServer, AgentServerDelegate, load_proxy_env};
use acp_thread::AgentConnection;
use agent_client_protocol::schema as acp;
use anyhow::{Context as _, Result};
use collections::HashSet;
use fs::Fs;
use gpui::{App, AppContext as _, Entity, Task};
use language_model::{ApiKey, EnvVar};
use project::{
    Project,
    agent_server_store::{AgentId, AllAgentServersSettings},
};
use settings::{SettingsStore, update_settings_file};
use std::{rc::Rc, sync::Arc};
use ui::IconName;
use vibedev_account;

pub const GEMINI_ID: &str = "gemini";
pub const CLAUDE_AGENT_ID: &str = "claude-acp";
pub const CODEX_ID: &str = "codex-acp";
/// VIBEDEV (V2-PLAN-8): the custom agent id for the bundled VibeDev ACP agent
/// (matches the `agent_servers.VibeDev` key in default/user settings). Only
/// this agent gets the remote self-provisioning command rewrite.
pub const VIBEDEV_AGENT_ID: &str = "VibeDev";

/// A generic agent server implementation for custom user-defined agents
pub struct CustomAgentServer {
    agent_id: AgentId,
}

impl CustomAgentServer {
    pub fn new(agent_id: AgentId) -> Self {
        Self { agent_id }
    }
}

impl AgentServer for CustomAgentServer {
    fn agent_id(&self) -> AgentId {
        self.agent_id.clone()
    }

    fn logo(&self) -> IconName {
        IconName::Terminal
    }

    fn default_mode(&self, cx: &App) -> Option<acp::SessionModeId> {
        let settings = cx.read_global(|settings: &SettingsStore, _| {
            settings
                .get::<AllAgentServersSettings>(None)
                .get(self.agent_id().0.as_ref())
                .cloned()
        });

        settings
            .as_ref()
            .and_then(|s| s.default_mode().map(acp::SessionModeId::new))
    }

    fn favorite_config_option_value_ids(
        &self,
        config_id: &acp::SessionConfigId,
        cx: &mut App,
    ) -> HashSet<acp::SessionConfigValueId> {
        let settings = cx.read_global(|settings: &SettingsStore, _| {
            settings
                .get::<AllAgentServersSettings>(None)
                .get(self.agent_id().0.as_ref())
                .cloned()
        });

        settings
            .as_ref()
            .and_then(|s| s.favorite_config_option_values(config_id.0.as_ref()))
            .map(|values| {
                values
                    .iter()
                    .cloned()
                    .map(acp::SessionConfigValueId::new)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn toggle_favorite_config_option_value(
        &self,
        config_id: acp::SessionConfigId,
        value_id: acp::SessionConfigValueId,
        should_be_favorite: bool,
        fs: Arc<dyn Fs>,
        cx: &App,
    ) {
        let agent_id = self.agent_id();
        let config_id = config_id.to_string();
        let value_id = value_id.to_string();

        update_settings_file(fs, cx, move |settings, _cx| {
            let settings = settings
                .agent_servers
                .get_or_insert_default()
                .entry(agent_id.0.to_string())
                .or_insert_with(|| default_settings_for_agent(&agent_id, _cx));

            match settings {
                settings::CustomAgentServerSettings::Custom {
                    favorite_config_option_values,
                    ..
                }
                | settings::CustomAgentServerSettings::Registry {
                    favorite_config_option_values,
                    ..
                } => {
                    let entry = favorite_config_option_values
                        .entry(config_id.clone())
                        .or_insert_with(Vec::new);

                    if should_be_favorite {
                        if !entry.iter().any(|v| v == &value_id) {
                            entry.push(value_id.clone());
                        }
                    } else {
                        entry.retain(|v| v != &value_id);
                        if entry.is_empty() {
                            favorite_config_option_values.remove(&config_id);
                        }
                    }
                }
            }
        });
    }

    fn set_default_mode(&self, mode_id: Option<acp::SessionModeId>, fs: Arc<dyn Fs>, cx: &mut App) {
        let agent_id = self.agent_id();
        update_settings_file(fs, cx, move |settings, _cx| {
            let settings = settings
                .agent_servers
                .get_or_insert_default()
                .entry(agent_id.0.to_string())
                .or_insert_with(|| default_settings_for_agent(&agent_id, _cx));

            match settings {
                settings::CustomAgentServerSettings::Custom { default_mode, .. }
                | settings::CustomAgentServerSettings::Registry { default_mode, .. } => {
                    *default_mode = mode_id.map(|m| m.to_string());
                }
            }
        });
    }

    fn default_model(&self, cx: &App) -> Option<acp::ModelId> {
        let settings = cx.read_global(|settings: &SettingsStore, _| {
            settings
                .get::<AllAgentServersSettings>(None)
                .get(self.agent_id().as_ref())
                .cloned()
        });

        settings
            .as_ref()
            .and_then(|s| s.default_model().map(acp::ModelId::new))
    }

    fn set_default_model(&self, model_id: Option<acp::ModelId>, fs: Arc<dyn Fs>, cx: &mut App) {
        let agent_id = self.agent_id();
        update_settings_file(fs, cx, move |settings, _cx| {
            let settings = settings
                .agent_servers
                .get_or_insert_default()
                .entry(agent_id.0.to_string())
                .or_insert_with(|| default_settings_for_agent(&agent_id, _cx));

            match settings {
                settings::CustomAgentServerSettings::Custom { default_model, .. }
                | settings::CustomAgentServerSettings::Registry { default_model, .. } => {
                    *default_model = model_id.map(|m| m.to_string());
                }
            }
        });
    }

    fn favorite_model_ids(&self, cx: &mut App) -> HashSet<acp::ModelId> {
        let settings = cx.read_global(|settings: &SettingsStore, _| {
            settings
                .get::<AllAgentServersSettings>(None)
                .get(self.agent_id().as_ref())
                .cloned()
        });

        settings
            .as_ref()
            .map(|s| {
                s.favorite_models()
                    .iter()
                    .map(|id| acp::ModelId::new(id.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn toggle_favorite_model(
        &self,
        model_id: acp::ModelId,
        should_be_favorite: bool,
        fs: Arc<dyn Fs>,
        cx: &App,
    ) {
        let agent_id = self.agent_id();
        update_settings_file(fs, cx, move |settings, _cx| {
            let settings = settings
                .agent_servers
                .get_or_insert_default()
                .entry(agent_id.0.to_string())
                .or_insert_with(|| default_settings_for_agent(&agent_id, _cx));

            let favorite_models = match settings {
                settings::CustomAgentServerSettings::Custom {
                    favorite_models, ..
                }
                | settings::CustomAgentServerSettings::Registry {
                    favorite_models, ..
                } => favorite_models,
            };

            let model_id_str = model_id.to_string();
            if should_be_favorite {
                if !favorite_models.contains(&model_id_str) {
                    favorite_models.push(model_id_str);
                }
            } else {
                favorite_models.retain(|id| id != &model_id_str);
            }
        });
    }

    fn default_config_option(&self, config_id: &str, cx: &App) -> Option<String> {
        let settings = cx.read_global(|settings: &SettingsStore, _| {
            settings
                .get::<AllAgentServersSettings>(None)
                .get(self.agent_id().as_ref())
                .cloned()
        });

        settings
            .as_ref()
            .and_then(|s| s.default_config_option(config_id).map(|s| s.to_string()))
    }

    fn set_default_config_option(
        &self,
        config_id: &str,
        value_id: Option<&str>,
        fs: Arc<dyn Fs>,
        cx: &mut App,
    ) {
        let agent_id = self.agent_id();
        let config_id = config_id.to_string();
        let value_id = value_id.map(|s| s.to_string());
        update_settings_file(fs, cx, move |settings, _cx| {
            let settings = settings
                .agent_servers
                .get_or_insert_default()
                .entry(agent_id.0.to_string())
                .or_insert_with(|| default_settings_for_agent(&agent_id, _cx));

            match settings {
                settings::CustomAgentServerSettings::Custom {
                    default_config_options,
                    ..
                }
                | settings::CustomAgentServerSettings::Registry {
                    default_config_options,
                    ..
                } => {
                    if let Some(value) = value_id.clone() {
                        default_config_options.insert(config_id.clone(), value);
                    } else {
                        default_config_options.remove(&config_id);
                    }
                }
            }
        });
    }

    fn connect(
        &self,
        delegate: AgentServerDelegate,
        project: Entity<Project>,
        cx: &mut App,
    ) -> Task<Result<Rc<dyn AgentConnection>>> {
        let agent_id = self.agent_id();
        let default_mode = self.default_mode(cx);
        let default_model = self.default_model(cx);
        let is_registry_agent = is_registry_agent(agent_id.clone(), cx);
        let default_config_options = cx.read_global(|settings: &SettingsStore, _| {
            settings
                .get::<AllAgentServersSettings>(None)
                .get(self.agent_id().as_ref())
                .map(|s| match s {
                    project::agent_server_store::CustomAgentServerSettings::Custom {
                        default_config_options,
                        ..
                    }
                    | project::agent_server_store::CustomAgentServerSettings::Registry {
                        default_config_options,
                        ..
                    } => default_config_options.clone(),
                })
                .unwrap_or_default()
        });

        if is_registry_agent {
            if let Some(registry_store) = project::AgentRegistryStore::try_global(cx) {
                registry_store.update(cx, |store, cx| store.refresh_if_stale(cx));
            }
        }

        let mut extra_env = load_proxy_env(cx);
        if delegate.store.read(cx).no_browser() {
            extra_env.insert("NO_BROWSER".to_owned(), "1".to_owned());
        }
        if is_registry_agent {
            match agent_id.as_ref() {
                CLAUDE_AGENT_ID => {
                    extra_env.insert("ANTHROPIC_API_KEY".into(), "".into());
                }
                CODEX_ID => {
                    if let Ok(api_key) = std::env::var("CODEX_API_KEY") {
                        extra_env.insert("CODEX_API_KEY".into(), api_key);
                    }
                    if let Ok(api_key) = std::env::var("OPEN_AI_API_KEY") {
                        extra_env.insert("OPEN_AI_API_KEY".into(), api_key);
                    }
                }
                GEMINI_ID => {
                    extra_env.insert("SURFACE".to_owned(), "zed".to_owned());
                }
                _ => {}
            }
        }
        let store = delegate.store.downgrade();
        cx.spawn(async move |cx| {
            if is_registry_agent && agent_id.as_ref() == GEMINI_ID {
                if let Some(api_key) = cx.update(api_key_for_gemini_cli).await.ok() {
                    extra_env.insert("GEMINI_API_KEY".into(), api_key);
                }
            }
            // VIBEDEV BYOK: inject the user's authenticated providers (key/endpoint/models)
            // so the agent backend can use the user's own keys per selected model. Empty map
            // ({"providers":[]}) when none — harmless. Failure is non-fatal (gateway fallback).
            let byok_task = cx.update(|cx| vibedev_account::byok::collect_byok_map(cx));
            if let Ok(json) = byok_task.await {
                extra_env.insert("VIBEDEV_BYOK".to_owned(), json);
            }
            let command = store
                .update(cx, |store, cx| {
                    let agent = store.get_external_agent(&agent_id).with_context(|| {
                        format!("Custom agent server `{}` is not registered", agent_id)
                    })?;
                    if let Some(new_version_available_tx) = delegate.new_version_available {
                        agent.set_new_version_available_tx(new_version_available_tx);
                    }
                    anyhow::Ok(agent.get_command(vec![], extra_env, &mut cx.to_async()))
                })??
                .await?;
            // VIBEDEV (V2-PLAN-8): on a REMOTE ssh project the client-resolved
            // command is a local (Windows) path that cannot execute on the remote
            // host. For the VibeDev agent we (1) replace it with a self-provisioning
            // launcher that deploys + execs the agent bundle on the remote, (2) sign
            // it in via env, and (3) route its gateway traffic back through the
            // client over a reverse tunnel (the remote often lacks egress). Other
            // custom/registry agents keep their command.
            let ssh_opts = project.read_with(cx, |project, cx| {
                project.remote_client().and_then(|rc| {
                    match rc.read(cx).connection_options() {
                        remote::remote_client::RemoteConnectionOptions::Ssh(opts) => Some(opts),
                        _ => None,
                    }
                })
            });
            let command = match ssh_opts {
                Some(opts) if agent_id.as_ref() == VIBEDEV_AGENT_ID => {
                    // VIBEDEV (agent-OTA): pass the local app's bundled agent
                    // version so the provisioning script re-provisions a remote
                    // whose dist/VERSION is absent or stale, keeping it in
                    // lockstep with the local app. `None` (dev / no bundled
                    // agent) keeps the original absent-only check. Bound first so
                    // `as_deref()` borrows the local.
                    let expected = vibedev_account::remote_agent::bundled_agent_version();
                    let (program, args) =
                        vibedev_account::remote_agent::remote_launch_command(
                            &command.args,
                            expected.as_deref(),
                        );
                    let mut env = command.env.unwrap_or_default();
                    // root-on-remote: bypassPermissions is disabled when geteuid()==0
                    // unless IS_SANDBOX is set; the agent runs in a controlled remote
                    // dev box, so opt in (don't override an explicit value).
                    env.entry("IS_SANDBOX".to_owned())
                        .or_insert_with(|| "1".to_owned());
                    // sign the remote agent in via env (no auth.json copied to the
                    // remote): OPENAI_API_KEY / OPENAI_BASE_URL from the local
                    // ~/.vibedev/auth.json. The base URL stays on the aitoken host so
                    // the request signature (over method+url+body) stays valid.
                    if let Some((api_key, base_url)) = vibedev_account::local_gateway_credentials()
                    {
                        env.entry("OPENAI_API_KEY".to_owned()).or_insert(api_key);
                        env.entry("OPENAI_BASE_URL".to_owned()).or_insert(base_url);
                    }
                    // provision the FULL account auth.json on the remote (api_key +
                    // proxy_url + available_models): the self-provision script writes
                    // it from this env var, so the remote agent is signed in AND its
                    // model picker shows the real gateway models (getVibedevChatModels
                    // reads available_models) instead of the default claude tiers.
                    if let Some(auth_json) = vibedev_account::local_auth_json_raw() {
                        env.insert("VIBEDEV_AUTH_JSON".to_owned(), auth_json);
                    }
                    // reverse tunnel: route the agent's gateway HTTPS back through the
                    // client (which has egress). Overrides any dead proxy inherited
                    // from the remote env. Best-effort — on failure the agent falls
                    // back to the remote's own egress.
                    // ensure_tunnel blocks (spawns `ssh -R`, then waits to verify
                    // the forward is live) — run it on a background thread so the
                    // UI executor isn't frozen during agent connect.
                    let host = opts.host.to_string();
                    let t_host = host.clone();
                    let t_port = opts.port;
                    let t_user = opts.username.clone();
                    let t_args = opts.args.clone().unwrap_or_default();
                    let tunnel = cx
                        .background_spawn(async move {
                            vibedev_account::remote_tunnel::ensure_tunnel(
                                &t_host,
                                t_port,
                                t_user.as_deref(),
                                &t_args,
                            )
                        })
                        .await;
                    match tunnel {
                        Ok(remote_port) => {
                            let proxy = format!("http://127.0.0.1:{remote_port}");
                            env.insert("HTTPS_PROXY".to_owned(), proxy.clone());
                            env.insert("https_proxy".to_owned(), proxy.clone());
                            env.insert("HTTP_PROXY".to_owned(), proxy.clone());
                            env.insert("http_proxy".to_owned(), proxy);
                        }
                        Err(error) => log::warn!(
                            "vibedev remote tunnel not established for {host} ({error:#}); \
                             agent will use the remote's own egress"
                        ),
                    }
                    project::agent_server_store::AgentServerCommand {
                        path: program.into(),
                        args,
                        env: Some(env),
                    }
                }
                _ => command,
            };
            let connection = crate::acp::connect(
                agent_id,
                project,
                command,
                store.clone(),
                default_mode,
                default_model,
                default_config_options,
                cx,
            )
            .await?;
            Ok(connection)
        })
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn std::any::Any> {
        self
    }
}

fn api_key_for_gemini_cli(cx: &mut App) -> Task<Result<String>> {
    let env_var = EnvVar::new("GEMINI_API_KEY".into()).or(EnvVar::new("GOOGLE_AI_API_KEY".into()));
    if let Some(key) = env_var.value {
        return Task::ready(Ok(key));
    }
    let credentials_provider = zed_credentials_provider::global(cx);
    let api_url = google_ai::API_URL.to_string();
    cx.spawn(async move |cx| {
        Ok(
            ApiKey::load_from_system_keychain(&api_url, credentials_provider.as_ref(), cx)
                .await?
                .key()
                .to_string(),
        )
    })
}

fn is_registry_agent(agent_id: impl Into<AgentId>, cx: &App) -> bool {
    let agent_id = agent_id.into();
    let is_in_registry = project::AgentRegistryStore::try_global(cx)
        .map(|store| store.read(cx).agent(&agent_id).is_some())
        .unwrap_or(false);
    let is_settings_registry = cx.read_global(|settings: &SettingsStore, _| {
        settings
            .get::<AllAgentServersSettings>(None)
            .get(agent_id.as_ref())
            .is_some_and(|s| {
                matches!(
                    s,
                    project::agent_server_store::CustomAgentServerSettings::Registry { .. }
                )
            })
    });
    is_in_registry || is_settings_registry
}

fn default_settings_for_agent(
    agent_id: &AgentId,
    cx: &App,
) -> settings::CustomAgentServerSettings {
    // VIBEDEV: when first creating the user-settings entry for an agent whose
    // CURRENT (baked/effective) definition is `Custom` — i.e. the baked VibeDev
    // agent from default.json — produce a `Custom` entry that PRESERVES that
    // command. The settings merge replaces the whole enum wholesale, so the old
    // code (always `Registry`) silently dropped the baked command, turning the
    // agent into a command-less registry entry that fails to register (the
    // "VibeDev not registered" / model-"未知" trap, with defaults never applying).
    // Genuinely registry-based agents still default to `Registry`.
    let resolved = cx.read_global(|store: &SettingsStore, _| {
        store
            .get::<AllAgentServersSettings>(None)
            .get(agent_id.as_ref())
            .cloned()
    });
    if let Some(project::agent_server_store::CustomAgentServerSettings::Custom { command, .. }) =
        resolved
    {
        settings::CustomAgentServerSettings::Custom {
            path: command.path,
            args: command.args,
            env: command.env.unwrap_or_default().into_iter().collect(),
            default_mode: None,
            default_model: None,
            favorite_models: Vec::new(),
            default_config_options: Default::default(),
            favorite_config_option_values: Default::default(),
        }
    } else {
        settings::CustomAgentServerSettings::Registry {
            default_model: None,
            default_mode: None,
            env: Default::default(),
            favorite_models: Vec::new(),
            default_config_options: Default::default(),
            favorite_config_option_values: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::HashMap;
    use gpui::TestAppContext;
    use project::agent_registry_store::{
        AgentRegistryStore, RegistryAgent, RegistryAgentMetadata, RegistryNpxAgent,
    };
    use settings::Settings as _;
    use ui::SharedString;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
    }

    fn init_registry_with_agents(cx: &mut TestAppContext, agent_ids: &[&str]) {
        let agents: Vec<RegistryAgent> = agent_ids
            .iter()
            .map(|id| {
                let id = SharedString::from(id.to_string());
                RegistryAgent::Npx(RegistryNpxAgent {
                    metadata: RegistryAgentMetadata {
                        id: AgentId::new(id.clone()),
                        name: id.clone(),
                        description: SharedString::from(""),
                        version: SharedString::from("1.0.0"),
                        repository: None,
                        website: None,
                        icon_path: None,
                    },
                    package: id,
                    args: Vec::new(),
                    env: HashMap::default(),
                })
            })
            .collect();
        cx.update(|cx| {
            AgentRegistryStore::init_test_global(cx, agents);
        });
    }

    fn set_agent_server_settings(
        cx: &mut TestAppContext,
        entries: Vec<(&str, settings::CustomAgentServerSettings)>,
    ) {
        cx.update(|cx| {
            AllAgentServersSettings::override_global(
                project::agent_server_store::AllAgentServersSettings(
                    entries
                        .into_iter()
                        .map(|(name, settings)| (name.to_string(), settings.into()))
                        .collect(),
                ),
                cx,
            );
        });
    }

    #[gpui::test]
    fn test_unknown_agent_is_not_registry(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            assert!(!is_registry_agent("my-custom-agent", cx));
        });
    }

    #[gpui::test]
    fn test_agent_in_registry_store_is_registry(cx: &mut TestAppContext) {
        init_test(cx);
        init_registry_with_agents(cx, &["some-new-registry-agent"]);
        cx.update(|cx| {
            assert!(is_registry_agent("some-new-registry-agent", cx));
            assert!(!is_registry_agent("not-in-registry", cx));
        });
    }

    #[gpui::test]
    fn test_agent_with_registry_settings_type_is_registry(cx: &mut TestAppContext) {
        init_test(cx);
        set_agent_server_settings(
            cx,
            vec![(
                "agent-from-settings",
                settings::CustomAgentServerSettings::Registry {
                    env: HashMap::default(),
                    default_mode: None,
                    default_model: None,
                    favorite_models: Vec::new(),
                    default_config_options: HashMap::default(),
                    favorite_config_option_values: HashMap::default(),
                },
            )],
        );
        cx.update(|cx| {
            assert!(is_registry_agent("agent-from-settings", cx));
        });
    }
}
