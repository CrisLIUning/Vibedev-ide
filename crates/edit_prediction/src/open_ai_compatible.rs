use anyhow::{Context as _, Result};
use cloud_llm_client::predict_edits_v3::{RawCompletionRequest, RawCompletionResponse};
use futures::AsyncReadExt as _;
use gpui::{App, AppContext as _, Entity, Global, SharedString, Task, http_client};
use language::language_settings::{OpenAiCompatibleEditPredictionSettings, all_language_settings};
use language_model::{ApiKeyState, EnvVar, env_var};
use std::sync::Arc;

pub fn open_ai_compatible_api_url(cx: &App) -> SharedString {
    all_language_settings(None, cx)
        .edit_predictions
        .open_ai_compatible_api
        .as_ref()
        .map(|settings| settings.api_url.clone())
        .unwrap_or_default()
        .into()
}

pub const OPEN_AI_COMPATIBLE_CREDENTIALS_USERNAME: &str = "openai-compatible-api-token";
// VIBEDEV: name of the env var the FIM key is read from, kept as a named const so
// the runtime reseed path (`vibedev_configure_fim`) constructs an `EnvVar` with the
// exact same name the `env_var!` macro below bakes in.
pub const OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR_NAME: &str =
    "ZED_OPEN_AI_COMPATIBLE_EDIT_PREDICTION_API_KEY";
pub static OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR: std::sync::LazyLock<EnvVar> =
    env_var!(OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR_NAME);

struct GlobalOpenAiCompatibleApiKey(Entity<ApiKeyState>);

impl Global for GlobalOpenAiCompatibleApiKey {}

pub fn open_ai_compatible_api_token(cx: &mut App) -> Entity<ApiKeyState> {
    if let Some(global) = cx.try_global::<GlobalOpenAiCompatibleApiKey>() {
        return global.0.clone();
    }

    let entity = cx.new(|cx| {
        ApiKeyState::new(
            open_ai_compatible_api_url(cx),
            OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR.clone(),
        )
    });
    cx.set_global(GlobalOpenAiCompatibleApiKey(entity.clone()));
    entity
}

pub fn load_open_ai_compatible_api_token(
    cx: &mut App,
) -> Task<Result<(), language_model::AuthenticateError>> {
    let credentials_provider = zed_credentials_provider::global(cx);
    let api_url = open_ai_compatible_api_url(cx);
    open_ai_compatible_api_token(cx).update(cx, |key_state, cx| {
        key_state.load_if_needed(api_url, |s| s, credentials_provider, cx)
    })
}

pub fn load_open_ai_compatible_api_key_if_needed(
    provider: settings::EditPredictionProvider,
    cx: &mut App,
) -> Option<Arc<str>> {
    if provider != settings::EditPredictionProvider::OpenAiCompatibleApi {
        return None;
    }
    _ = load_open_ai_compatible_api_token(cx);
    let url = open_ai_compatible_api_url(cx);
    return open_ai_compatible_api_token(cx).read(cx).key(&url);
}

// VIBEDEV: point the OpenAI-compatible FIM edit-prediction provider at the local
// VibeDev sidecar's loopback completions endpoint, using the per-launch bearer
// token from the sidecar handshake. Called from `vibedev_account` once the async
// sidecar handshake has yielded `port` + `token`.
//
// Two things have to happen, and ORDER + the global reseed both matter:
//
//  1. The bearer token must reach the FIM request. The request reads its key from
//     `ApiKeyState`, which is seeded from the `OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR`
//     `LazyLock<EnvVar>`. That env var is read EXACTLY ONCE (at first deref) and
//     cached forever, and `EditPredictionButton::new` derefs it unconditionally at
//     workspace creation — i.e. before this async handshake can resolve. So by the
//     time we have a token, the `LazyLock` (and the `GlobalOpenAiCompatibleApiKey`
//     global built from it) already hold an empty value: a plain
//     `std::env::set_var` after the fact loses the race and the request would go
//     out with no `Authorization` header (401 at the sidecar gate). We therefore
//     OVERWRITE the global `ApiKeyState` with one whose `EnvVar` already carries
//     the token. `open_ai_compatible_api_token` returns the existing global without
//     rebuilding, so this reseed wins for every later read. (`set_var` is still
//     done as belt-and-suspenders for any code path that builds a fresh `EnvVar`.)
//
//  2. The dynamic `api_url` (loopback port is only known at runtime) is written
//     into the user settings file via `SettingsStore`, flipping the provider to
//     `open_ai_compatible_api`. This is async; we run it AFTER the synchronous
//     reseed so that when the provider flips and the first FIM fires, the global
//     already has the token. The `api_url` we seed the `ApiKeyState` with is the
//     exact string we write to settings, so `ApiKeyState::key(url)` matches.
pub fn vibedev_configure_fim(cx: &mut App, port: u16, token: String) {
    // The request builder uses `settings.api_url` verbatim as the POST URI (no
    // path is appended), so this must be the FULL completions endpoint, not just
    // the `/v1` base.
    let api_url = format!("http://127.0.0.1:{port}/v1/completions");
    let api_url_shared: SharedString = api_url.clone().into();

    // (1) belt-and-suspenders: covers any future `EnvVar::new` read of this var.
    // Safe here: single-threaded GPUI main thread, done before the provider flips.
    unsafe {
        std::env::set_var(OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR_NAME, &token);
    }

    // (1) authoritative: overwrite the global key state with the live token. This
    // is what actually makes the token reach the request, regardless of what the
    // already-materialized `LazyLock` captured at startup.
    let env_var = EnvVar {
        name: OPEN_AI_COMPATIBLE_TOKEN_ENV_VAR_NAME.into(),
        value: Some(token),
    };
    let key_state = cx.new(|_| ApiKeyState::new(api_url_shared.clone(), env_var));
    cx.set_global(GlobalOpenAiCompatibleApiKey(key_state));

    // (2) inject the dynamic settings (provider + endpoint) at runtime. A static
    // default.json can't carry the per-launch port, so this is the runtime path.
    let fs = <dyn fs::Fs>::global(cx);
    settings::update_settings_file(fs, cx, move |settings, _| {
        let edit_predictions = settings
            .project
            .all_languages
            .edit_predictions
            .get_or_insert_default();
        edit_predictions.provider = Some(settings::EditPredictionProvider::OpenAiCompatibleApi);
        let open_ai_compatible = edit_predictions
            .open_ai_compatible_api
            .get_or_insert_default();
        open_ai_compatible.model = Some("deepseek-coder".to_string());
        open_ai_compatible.api_url = Some(api_url);
        open_ai_compatible.max_output_tokens = Some(256);
        open_ai_compatible.prompt_format =
            Some(settings::EditPredictionPromptFormatContent::DeepseekCoder);
    });
}

pub(crate) async fn send_custom_server_request(
    provider: settings::EditPredictionProvider,
    settings: &OpenAiCompatibleEditPredictionSettings,
    prompt: String,
    max_tokens: u32,
    stop_tokens: Vec<String>,
    api_key: Option<Arc<str>>,
    http_client: &Arc<dyn http_client::HttpClient>,
) -> Result<(String, String)> {
    match provider {
        settings::EditPredictionProvider::Ollama => {
            let response = crate::ollama::make_request(
                settings.clone(),
                prompt,
                stop_tokens,
                http_client.clone(),
            )
            .await?;
            Ok((response.response, response.created_at))
        }
        _ => {
            let request = RawCompletionRequest {
                model: settings.model.clone(),
                prompt,
                max_tokens: Some(max_tokens),
                temperature: None,
                stop: stop_tokens
                    .into_iter()
                    .map(std::borrow::Cow::Owned)
                    .collect(),
                environment: None,
            };

            let request_body = serde_json::to_string(&request)?;
            let mut http_request_builder = http_client::Request::builder()
                .method(http_client::Method::POST)
                .uri(settings.api_url.as_ref())
                .header("Content-Type", "application/json");

            if let Some(api_key) = api_key {
                http_request_builder =
                    http_request_builder.header("Authorization", format!("Bearer {}", api_key));
            }

            let http_request =
                http_request_builder.body(http_client::AsyncBody::from(request_body))?;

            let mut response = http_client.send(http_request).await?;
            let status = response.status();

            if !status.is_success() {
                let mut body = String::new();
                response.body_mut().read_to_string(&mut body).await?;
                anyhow::bail!("custom server error: {} - {}", status, body);
            }

            let mut body = String::new();
            response.body_mut().read_to_string(&mut body).await?;

            let parsed: RawCompletionResponse =
                serde_json::from_str(&body).context("Failed to parse completion response")?;
            let text = parsed
                .choices
                .into_iter()
                .next()
                .map(|choice| choice.text)
                .unwrap_or_default();
            Ok((text, parsed.id))
        }
    }
}
