//! VIBEDEV: wire an `openai_compatible` LanguageModelRegistry provider that
//! points at the local VibeDev sidecar's loopback chat endpoint, and select it
//! as the inline-assistant (内联助手) + default model.
//!
//! WHY THIS EXISTS
//! ---------------
//! Zed's inline assistant (`crates/agent_ui/src/inline_assistant.rs`) reads its
//! model from `LanguageModelRegistry::inline_assistant_model()`. With no
//! configured/authenticated provider it errors `ConfigurationError::NoProvider`
//! ("未配置 llm 供应商"). VibeDev's models live behind the sub2 gateway, which
//! REQUIRES a vibedev HMAC signature that Zed's native `openai_compatible`
//! provider cannot produce. So — exactly like FIM/edit_predictions
//! (`edit_prediction::open_ai_compatible::vibedev_configure_fim`) — we point the
//! provider at the signing SIDECAR's loopback `/v1` (the sidecar adds the
//! signature and forwards to the gateway). The provider's per-request bearer is
//! the sidecar's per-launch handshake TOKEN, which passes the sidecar's `/v1/*`
//! bearer gate.
//!
//! Endpoint contract: `api_url = http://127.0.0.1:<port>/v1`. The OpenAI client
//! appends `/chat/completions` (open_ai.rs `stream_completion`), so requests hit
//! the sidecar's new `POST /v1/chat/completions` (chatCompletions.ts).
//!
//! HOW THE PROVIDER GETS REGISTERED + AUTHENTICATED
//! ------------------------------------------------
//! `language_models::init` watches `SettingsStore` and (de)registers an
//! `OpenAiCompatibleLanguageModelProvider` for every key under
//! `language_models.openai_compatible`. So writing our settings key is what
//! actually registers the provider — we don't construct it by hand.
//!
//! That provider builds its own `ApiKeyState` seeded from the env var
//! `<ID>_API_KEY` (upper-snake of the settings key). For settings key `vibedev`
//! that is `VIBEDEV_API_KEY`. We therefore `set_var("VIBEDEV_API_KEY", token)`
//! BEFORE the provider is created, so when we call `authenticate()` the key
//! loads straight from the env (no keychain round-trip) and `is_authenticated()`
//! flips true — clearing the `NoProvider` error.
//!
//! ORDER MATTERS, and registration is async (it happens on the SettingsStore
//! observer's next effect cycle). So we: (1) set the env var synchronously, (2)
//! await the settings-file write, then (3) spawn a short retry loop that waits
//! for the provider to appear, authenticates it, and selects it as the
//! inline-assistant + default model.

use std::time::Duration;

use collections::HashMap;
use fs::Fs;
use gpui::App;
use language_model::{
    LanguageModelId, LanguageModelProviderId, LanguageModelRegistry, SelectedModel,
};
use settings::{
    OpenAiCompatibleAvailableModel, OpenAiCompatibleModelCapabilities,
    OpenAiCompatibleSettingsContent, update_settings_file,
};

/// Settings key + registry provider id for the VibeDev chat provider. Lower-case
/// because the OpenAI-compatible provider derives its display name and its
/// `<ID>_API_KEY` env var from this exact string.
pub const VIBEDEV_CHAT_PROVIDER_ID: &str = "vibedev";

/// Env var the auto-created provider reads its key from (`<ID>_API_KEY`,
/// upper-snake of [`VIBEDEV_CHAT_PROVIDER_ID`]). Must match what the provider's
/// `OpenAiCompatibleLanguageModelProvider::new` computes, or the seeded token
/// won't be picked up.
const VIBEDEV_CHAT_API_KEY_ENV_VAR: &str = "VIBEDEV_API_KEY";

/// Conservative default context window when the gateway gives us only a model
/// id (the sidecar's `available_models` are bare ids). 200k matches the Claude /
/// GPT-5 tier the gateway commonly serves; the gateway is the real authority on
/// limits, this only bounds Zed's local token estimate.
const DEFAULT_MAX_TOKENS: u64 = 200_000;

/// How long to keep retrying for the settings-driven provider registration to
/// land before giving up (logged). Registration normally appears within a frame
/// or two; this is a generous ceiling for a slow settings write.
const REGISTER_WAIT: Duration = Duration::from_secs(5);
const REGISTER_POLL: Duration = Duration::from_millis(100);

/// Point the inline assistant (and the default model) at the VibeDev sidecar's
/// loopback chat endpoint.
///
/// - `port` / `token`: the sidecar handshake (`~/.vibedev/endpoint.json`). The
///   token is the bearer that the sidecar's `/v1/*` gate checks; it is NOT the
///   sub2 api_key (the sidecar holds that and signs server-side).
/// - `models`: chat-capable model ids from `~/.vibedev/auth.json`
///   `available_models` (sourced by the caller, e.g. via the sidecar's
///   `/vibedev/providers`). When empty, the provider is still registered (so the
///   endpoint + token are wired) but no model can be selected yet — the caller
///   should re-invoke once the user has signed in and models are known.
pub fn vibedev_configure_inline_assistant(
    cx: &mut App,
    port: u16,
    token: String,
    models: Vec<String>,
) {
    // The chat provider's `api_url` is the `/v1` BASE — the OpenAI client appends
    // `/chat/completions`. (Contrast FIM, whose `api_url` is the full completions
    // path because its request builder uses it verbatim.)
    let api_url = format!("http://127.0.0.1:{port}/v1");

    // (1) Seed the env var BEFORE the provider is created from settings, so its
    // `ApiKeyState` loads the handshake token straight from the env on
    // `authenticate()`. Safe: single-threaded GPUI main thread, done before the
    // settings write that triggers provider construction.
    unsafe {
        std::env::set_var(VIBEDEV_CHAT_API_KEY_ENV_VAR, &token);
    }

    let available_models: Vec<OpenAiCompatibleAvailableModel> =
        models.iter().map(|id| chat_model(id.clone())).collect();

    // (2) Force a FRESH provider construction. The OpenAiCompatible provider
    // captures its API-key env var (VIBEDEV_API_KEY) exactly ONCE, at
    // construction (`env_var::EnvVar::new` stores `std::env::var(name)` into a
    // field and never re-reads it; `ApiKeyState::load_if_needed` only ever reads
    // that captured field). On any launch where this key is already persisted in
    // settings.json, `language_models::init` constructs the provider at STARTUP —
    // before the sidecar handshake set the env var — so it captures an empty key
    // and `is_authenticated()` is permanently false. The settings observer only
    // calls `handle_url_change` (which re-reads the same captured field), and the
    // `openai_compatible` registration only re-runs when the KEY SET changes, not
    // when a key's `api_url` changes — so a plain re-write never re-authenticates.
    // The inline assistant then silently no-ops on
    // `ConfigurationError::ProviderNotAuthenticated` (inline_assistant.rs spawns
    // an async authenticate then synchronously re-checks before it can finish, so
    // the prompt never opens). Fix: REMOVE the key, wait for it to de-register,
    // THEN re-add it — `register_openai_compatible_providers` then constructs a
    // brand-new provider whose `EnvVar` finally sees the token set in step (1).
    let fs = <dyn Fs>::global(cx);
    let provider_id = LanguageModelProviderId::from(VIBEDEV_CHAT_PROVIDER_ID.to_string());
    let first_model = models.first().cloned();
    let keychain_url = api_url.clone();
    cx.spawn(async move |cx| {
        // (2a) Remove the key so any stale, unauthenticated provider de-registers.
        cx.update(|cx| {
            update_settings_file(fs.clone(), cx, |settings, _| {
                if let Some(openai_compatible) = settings
                    .language_models
                    .as_mut()
                    .and_then(|language_models| language_models.openai_compatible.as_mut())
                {
                    openai_compatible.remove(VIBEDEV_CHAT_PROVIDER_ID);
                }
            });
        });

        // (2b) Wait for the de-registration to land. The key-set-change observer
        // must observe the key GONE before we re-add it, or a coalesced settings
        // reload nets to "no change" and skips the reconstruction. Bounded; if it
        // doesn't clear in time we proceed anyway (re-add is still attempted).
        let deadline = std::time::Instant::now() + REGISTER_WAIT;
        loop {
            let absent = cx.update(|cx| {
                LanguageModelRegistry::read_global(cx)
                    .provider(&provider_id)
                    .is_none()
            });
            if absent {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            cx.background_executor().timer(REGISTER_POLL).await;
        }

        // (2c) Re-add the key → a brand-new provider is constructed now, AFTER the
        // env var was set, so its `EnvVar` captures the token.
        cx.update(|cx| {
            update_settings_file(fs.clone(), cx, move |settings, _| {
                let language_models = settings.language_models.get_or_insert_default();
                let openai_compatible = language_models
                    .openai_compatible
                    .get_or_insert_with(HashMap::default);
                openai_compatible.insert(
                    VIBEDEV_CHAT_PROVIDER_ID.into(),
                    OpenAiCompatibleSettingsContent {
                        api_url,
                        available_models,
                        custom_headers: None,
                    },
                );
            });
        });

        // (3) Wait for the fresh registration to land, then authenticate it
        // (loads the env token) and select it as the inline-assistant + default
        // model. If `models` is empty we still register/authenticate but skip the
        // selection (nothing to point at yet).
        // (`AsyncApp::update` in this GPUI fork returns the closure value
        // directly, not a Result.)
        let deadline = std::time::Instant::now() + REGISTER_WAIT;
        loop {
            let present = cx.update(|cx| {
                LanguageModelRegistry::read_global(cx)
                    .provider(&provider_id)
                    .is_some()
            });
            if present {
                break;
            }
            if std::time::Instant::now() >= deadline {
                log::warn!(
                    "vibedev inline-assistant provider did not register within {REGISTER_WAIT:?}; \
                     inline assistant may still report no provider"
                );
                return;
            }
            cx.background_executor().timer(REGISTER_POLL).await;
        }

        // Store the handshake token in the OS keychain for this provider's
        // api_url, via the SAME credentials provider the language-model providers
        // authenticate against (stashed by language_models::init). The env-var
        // path can't authenticate here: the provider's `EnvVar` captured
        // `std::env` ONCE at construction (app startup, before this token existed)
        // and never re-reads it, so `is_authenticated()` stays false and the
        // inline assistant silently won't open. Writing the keychain is exactly
        // what a manually-entered key does (`set_api_key` -> `write_credentials`);
        // `ApiKeyState::load_if_needed`'s keychain branch then loads it -> `Loaded`.
        let credentials = cx.update(|cx| {
            cx.try_global::<crate::GlobalVibedevCredentials>()
                .map(|global| global.0.clone())
        });
        if let Some(credentials) = credentials {
            if let Err(error) = credentials
                .write_credentials(&keychain_url, "Bearer", token.as_bytes(), cx)
                .await
            {
                log::warn!("vibedev inline-assistant keychain write failed: {error:#}");
            }
        }

        // Authenticate the provider so `is_authenticated()` is true (loads the
        // keychain token). Then select it as the inline-assistant + default.
        let authenticate = cx.update(|cx| {
            LanguageModelRegistry::read_global(cx)
                .provider(&provider_id)
                .map(|provider| provider.authenticate(cx))
        });
        if let Some(task) = authenticate {
            // A failure here is non-fatal: the env var is set, so the next read
            // still resolves the key; we just log and continue to selection.
            if let Err(error) = task.await {
                log::warn!("vibedev inline-assistant provider authenticate failed: {error:#}");
            }
        }

        let Some(model_id) = first_model else {
            log::info!(
                "vibedev inline-assistant provider registered (no models yet; \
                 sign in then re-configure to select a model)"
            );
            return;
        };

        let registry = cx.update(|cx| LanguageModelRegistry::global(cx));
        let selected = SelectedModel {
            provider: provider_id.clone(),
            model: LanguageModelId::from(model_id.clone()),
        };
        registry.update(cx, |registry, cx| {
            // Inline assistant (the actual bug) + default so the agent panel and
            // other consumers (commit message / thread summary fall back to it)
            // also have a model.
            registry.select_inline_assistant_model(Some(&selected), cx);
            registry.select_default_model(Some(&selected), cx);
        });
        log::info!("vibedev inline-assistant pointed at loopback sidecar /v1 (model {model_id})");
    })
    .detach();
}

/// Build an `OpenAiCompatibleAvailableModel` for a bare gateway model id. Marks
/// it tool- and chat-completions-capable (the inline assistant + agent need tool
/// calls); the gateway is the real authority on per-model limits, so we only set
/// a conservative local `max_tokens`.
fn chat_model(id: String) -> OpenAiCompatibleAvailableModel {
    OpenAiCompatibleAvailableModel {
        name: id,
        display_name: None,
        max_tokens: DEFAULT_MAX_TOKENS,
        max_output_tokens: None,
        max_completion_tokens: None,
        reasoning_effort: None,
        capabilities: OpenAiCompatibleModelCapabilities {
            tools: true,
            images: false,
            parallel_tool_calls: false,
            prompt_cache_key: false,
            chat_completions: true,
            interleaved_reasoning: false,
        },
    }
}
