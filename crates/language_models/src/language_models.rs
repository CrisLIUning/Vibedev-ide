use std::sync::Arc;

use ::settings::{Settings, SettingsStore};
use client::{Client, UserStore};
use collections::HashSet;
use credentials_provider::CredentialsProvider;
use gpui::{App, Context, Entity};
use language_model::{
    ConfiguredModel, LanguageModelProviderId, LanguageModelRegistry, ZED_CLOUD_PROVIDER_ID,
};
use provider::deepseek::DeepSeekLanguageModelProvider;

pub mod extension;
pub mod provider;
mod settings;
// VIBEDEV: registers an `openai_compatible` provider pointed at the local
// sidecar's loopback `/v1` and selects it as the inline-assistant + default
// model, so the inline assistant (内联助手) has a configured provider instead of
// erroring `NoProvider`. Called from `vibedev_account` after the sidecar
// handshake, mirroring how FIM is wired.
pub mod vibedev_inline_assistant;

pub use crate::extension::init_proxy as init_extension_proxy;
pub use crate::vibedev_inline_assistant::vibedev_configure_inline_assistant;

use crate::provider::anthropic::AnthropicLanguageModelProvider;
use crate::provider::bedrock::BedrockLanguageModelProvider;
use crate::provider::google::GoogleLanguageModelProvider;
use crate::provider::lmstudio::LmStudioLanguageModelProvider;
pub use crate::provider::mistral::MistralLanguageModelProvider;
use crate::provider::ollama::OllamaLanguageModelProvider;
use crate::provider::open_ai::OpenAiLanguageModelProvider;
use crate::provider::open_ai_compatible::OpenAiCompatibleLanguageModelProvider;
use crate::provider::open_router::OpenRouterLanguageModelProvider;
use crate::provider::opencode::OpenCodeLanguageModelProvider;
use crate::provider::vercel_ai_gateway::VercelAiGatewayLanguageModelProvider;
use crate::provider::x_ai::XAiLanguageModelProvider;
pub use crate::settings::*;

/// VIBEDEV: stash of the app's credentials provider — the same store the
/// language-model providers read their API keys from — so the inline-assistant
/// configurator (`vibedev_inline_assistant`) can write the VibeDev sidecar
/// handshake token into the SAME keychain store the provider authenticates
/// against (the env-var path is unreliable; see vibedev_inline_assistant).
pub(crate) struct GlobalVibedevCredentials(pub Arc<dyn CredentialsProvider>);
impl gpui::Global for GlobalVibedevCredentials {}

pub fn init(user_store: Entity<UserStore>, client: Arc<Client>, cx: &mut App) {
    let credentials_provider = client.credentials_provider();
    cx.set_global(GlobalVibedevCredentials(credentials_provider.clone()));
    let registry = LanguageModelRegistry::global(cx);
    registry.update(cx, |registry, cx| {
        register_language_model_providers(
            registry,
            user_store,
            client.clone(),
            credentials_provider.clone(),
            cx,
        );
    });

    // Subscribe to extension store events to track LLM extension installations
    if let Some(extension_store) = extension_host::ExtensionStore::try_global(cx) {
        cx.subscribe(&extension_store, {
            let registry = registry.downgrade();
            move |extension_store, event, cx| {
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                match event {
                    extension_host::Event::ExtensionInstalled(extension_id) => {
                        if let Some(manifest) = extension_store
                            .read(cx)
                            .extension_manifest_for_id(extension_id)
                        {
                            if !manifest.language_model_providers.is_empty() {
                                registry.update(cx, |registry, cx| {
                                    registry.extension_installed(extension_id.clone(), cx);
                                });
                            }
                        }
                    }
                    extension_host::Event::ExtensionUninstalled(extension_id) => {
                        registry.update(cx, |registry, cx| {
                            registry.extension_uninstalled(extension_id, cx);
                        });
                    }
                    extension_host::Event::ExtensionsUpdated => {
                        let mut new_ids = HashSet::default();
                        for (extension_id, entry) in extension_store.read(cx).installed_extensions()
                        {
                            if !entry.manifest.language_model_providers.is_empty() {
                                new_ids.insert(extension_id.clone());
                            }
                        }
                        registry.update(cx, |registry, cx| {
                            registry.sync_installed_llm_extensions(new_ids, cx);
                        });
                    }
                    _ => {}
                }
            }
        })
        .detach();

        // Initialize with currently installed extensions
        registry.update(cx, |registry, cx| {
            let mut initial_ids = HashSet::default();
            for (extension_id, entry) in extension_store.read(cx).installed_extensions() {
                if !entry.manifest.language_model_providers.is_empty() {
                    initial_ids.insert(extension_id.clone());
                }
            }
            registry.sync_installed_llm_extensions(initial_ids, cx);
        });
    }

    let mut openai_compatible_providers = AllLanguageModelSettings::get_global(cx)
        .openai_compatible
        .keys()
        .cloned()
        .collect::<HashSet<_>>();

    registry.update(cx, |registry, cx| {
        register_openai_compatible_providers(
            registry,
            &HashSet::default(),
            &openai_compatible_providers,
            client.clone(),
            credentials_provider.clone(),
            cx,
        );
    });

    let registry = registry.downgrade();
    cx.observe_global::<SettingsStore>(move |cx| {
        let Some(registry) = registry.upgrade() else {
            return;
        };
        let openai_compatible_providers_new = AllLanguageModelSettings::get_global(cx)
            .openai_compatible
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        if openai_compatible_providers_new != openai_compatible_providers {
            registry.update(cx, |registry, cx| {
                register_openai_compatible_providers(
                    registry,
                    &openai_compatible_providers,
                    &openai_compatible_providers_new,
                    client.clone(),
                    credentials_provider.clone(),
                    cx,
                );
            });
            openai_compatible_providers = openai_compatible_providers_new;
        }
    })
    .detach();
}

/// Recomputes and sets the [`LanguageModelRegistry`]'s environment fallback
/// model based on currently authenticated providers.
///
/// Prefers the Zed cloud provider so that, once the user is signed in, we
/// always pick a Zed-hosted model over models from other authenticated
/// providers in the environment. If the Zed cloud provider is authenticated
/// but hasn't finished loading its models yet, we don't fall back to another
/// provider to avoid flickering between providers during sign in.
pub fn update_environment_fallback_model(cx: &mut App) {
    let registry = LanguageModelRegistry::global(cx);
    let fallback_model = {
        let registry = registry.read(cx);
        let cloud_provider = registry.provider(&ZED_CLOUD_PROVIDER_ID);
        if cloud_provider
            .as_ref()
            .is_some_and(|provider| provider.is_authenticated(cx))
        {
            cloud_provider.and_then(|provider| {
                let model = provider
                    .default_model(cx)
                    .or_else(|| provider.recommended_models(cx).first().cloned())?;
                Some(ConfiguredModel { provider, model })
            })
        } else {
            registry
                .providers()
                .iter()
                .filter(|provider| provider.is_authenticated(cx))
                .find_map(|provider| {
                    let model = provider
                        .default_model(cx)
                        .or_else(|| provider.recommended_models(cx).first().cloned())?;
                    Some(ConfiguredModel {
                        provider: provider.clone(),
                        model,
                    })
                })
        }
    };
    registry.update(cx, |registry, cx| {
        registry.set_environment_fallback_model(fallback_model, cx);
    });
}

fn register_openai_compatible_providers(
    registry: &mut LanguageModelRegistry,
    old: &HashSet<Arc<str>>,
    new: &HashSet<Arc<str>>,
    client: Arc<Client>,
    credentials_provider: Arc<dyn CredentialsProvider>,
    cx: &mut Context<LanguageModelRegistry>,
) {
    for provider_id in old {
        if !new.contains(provider_id) {
            registry.unregister_provider(LanguageModelProviderId::from(provider_id.clone()), cx);
        }
    }

    for provider_id in new {
        if !old.contains(provider_id) {
            registry.register_provider(
                Arc::new(OpenAiCompatibleLanguageModelProvider::new(
                    provider_id.clone(),
                    client.http_client(),
                    credentials_provider.clone(),
                    cx,
                )),
                cx,
            );
        }
    }
}

fn register_language_model_providers(
    registry: &mut LanguageModelRegistry,
    user_store: Entity<UserStore>,
    client: Arc<Client>,
    credentials_provider: Arc<dyn CredentialsProvider>,
    cx: &mut Context<LanguageModelRegistry>,
) {
    // VIBEDEV: Zed Cloud provider registration disabled — its configuration UI
    // (provider/cloud.rs) renders Sign-In / Trial / Upgrade buttons that link
    // to zed.dev/account, wrong backend for VibeDev. Users reach Anthropic
    // models through the sub2 gateway via the Anthropic provider's
    // OPENAI_BASE_URL override, configured by claude-code-best's ACP agent.
    //
    // registry.register_provider(
    //     Arc::new(CloudLanguageModelProvider::new(
    //         user_store,
    //         client.clone(),
    //         cx,
    //     )),
    //     cx,
    // );
    let _ = user_store; // silence unused-binding warning now that Cloud is gone
    registry.register_provider(
        Arc::new(AnthropicLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(OpenAiLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(OllamaLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(LmStudioLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(DeepSeekLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(GoogleLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        MistralLanguageModelProvider::global(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        ),
        cx,
    );
    registry.register_provider(
        Arc::new(BedrockLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(OpenRouterLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(VercelAiGatewayLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(XAiLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(OpenCodeLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    // VIBEDEV: CopilotChat provider disabled — we removed Copilot from the
    // edit-prediction provider menu, and we don't want a separate
    // Copilot-as-chat-model entry either. Users who actually have Copilot can
    // re-enable in user settings if they configure it manually.
    // registry.register_provider(Arc::new(CopilotChatLanguageModelProvider::new(cx)), cx);

    // VIBEDEV: OpenAiSubscribed provider disabled — this is Zed's own OpenAI
    // subscription resold through Zed Cloud. VibeDev uses sub2 for OpenAI
    // access via the existing OpenAiLanguageModelProvider with a custom base URL.
    // registry.register_provider(
    //     Arc::new(OpenAiSubscribedProvider::new(
    //         client.http_client(),
    //         credentials_provider,
    //         cx,
    //     )),
    //     cx,
    // );
    let _ = credentials_provider; // silence unused after the last consumer was disabled
}
