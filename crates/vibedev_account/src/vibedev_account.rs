//! VibeDev account state + backend localhost data channel.
//!
//! The frontend never holds the sub2api signing key or token. A local
//! sidecar (`V2-PLAN-5-local-sidecar.md`, a long-lived bun process shipped
//! with the backend) owns the secrets, talks to sub2api with a signature, and
//! exposes read/write endpoints on `127.0.0.1`. This crate:
//!
//! 1. spawns + supervises that sidecar ([`VibedevSidecar`]),
//! 2. reads the `~/.vibedev/endpoint.json` handshake the sidecar writes
//!    (`{port, token}`, permissions locked by the sidecar),
//! 3. performs authed read/write fetches against the sidecar
//!    ([`fetch_account`], [`fetch_providers`], [`save_providers`]),
//! 4. drives the VibeDev (sub2api) login flow ([`start_login`]) and relays the
//!    custom-scheme callback payload back to the sidecar
//!    ([`relay_login_callback`]).
//!
//! Balance/plan come straight from the sidecar's sub2api numbers — never an
//! ACP cost estimate (Plan 5 audit #12).

use anyhow::{Context as _, Result};
use futures::AsyncReadExt as _;
use gpui::App;
use http_client::{AsyncBody, HttpClient, Request};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};

mod launcher;
// VIBEDEV: BYOK (bring-your-own-key) provider enumeration. Walks the
// authenticated providers in LanguageModelRegistry and serializes them to the
// frozen `VIBEDEV_BYOK` JSON contract the backend reads. Public so a later task
// (F2) can call `vibedev_account::byok::collect_byok_map` to inject the string
// into the agent child process env.
pub mod byok;
// VIBEDEV: persists the user's last per-session ACP picks (model / thinking /
// context) under `~/.vibedev/last-session-prefs.json` so a new session
// re-applies them instead of resetting to the agent's defaults (which
// claude-code-best does by design on every newSession).
pub mod session_prefs;
// VIBEDEV: 24h background poller against `aitoken.bigopen.cn/vibedev/latest.json`
// that exposes an `UpdateStatus` global for the account panel to render as a
// "v1.x.y available - download" banner. Replaces Zed's permanently-disabled
// upstream `auto_update` crate (see commit `dcb40a7bf0`).
pub mod update_check;
// VIBEDEV: handles `vibedev://auth-callback?payload=...` arriving from the
// system-registered URL scheme. Splits URL parsing from the relay-to-sidecar
// step so unit tests can exercise the parser without spinning the sidecar up.
pub mod url_handler;
// VIBEDEV (V2-PLAN-8): remote (SSH) ACP agent support — rewrite the
// client-resolved agent command so it runs the bundle provisioned on the
// remote host (a local Windows path can't execute on the remote Linux box).
pub mod remote_agent;
// VIBEDEV (V2-PLAN-8): reverse SSH tunnel so a remote agent without its own
// internet egress reaches the gateway through the client (which has egress).
pub mod remote_tunnel;

pub use launcher::{
    SIDECAR_HEALTH_TIMEOUT, VibedevSidecar, reconfigure_inline_assistant,
};

/// One-time crate-level initialization. Currently spawns the `update_check`
/// polling loop; the sidecar launcher is still kicked off separately from
/// `vibedev_ui::init` because it returns a handle the caller has to stash in a
/// global. Callers should invoke this once, from `main.rs` (the architectural
/// slot the now-disabled `auto_update::init` used to occupy).
pub fn init(cx: &mut App) {
    update_check::init(cx);
}

/// Read-only account data the dashboard needs. The sidecar (which holds the
/// signing key and talks to sub2api) populates this; the frontend only reads.
///
/// `balance`/`plan` are the authoritative sub2api numbers, not ACP estimates.
/// When the user is not signed in (or the sub2api token expired), `signed_in`
/// is `false` and `reason` carries a human-readable hint rather than a bare
/// 401 (Plan 5 degradation table).
///
/// `profile`/`usage`/`subscriptions` are the richer dashboard fields the
/// sidecar populates from its sub2api-authed endpoints (account/me,
/// usage/dashboard/stats, subscriptions/summary). They are all optional: a
/// sidecar that hasn't been upgraded yet simply omits them, and the panel
/// renders only the sections whose data is present.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AccountSnapshot {
    pub signed_in: bool,
    pub email: Option<String>,
    pub balance: Option<f64>,
    pub plan: Option<String>,
    pub reason: Option<String>,
    #[serde(default)]
    pub profile: Option<AccountProfile>,
    #[serde(default)]
    pub usage: Option<AccountUsage>,
    #[serde(default)]
    pub subscriptions: Option<Vec<AccountSubscription>>,
}

/// Identity extras beyond `email`/`plan`/`balance`: the avatar, the account
/// role/tier label, and the lifetime recharge total. All optional.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AccountProfile {
    pub avatar_url: Option<String>,
    pub role: Option<String>,
    pub total_recharged: Option<f64>,
}

/// Aggregate usage counters (lifetime totals + today's slice + current rate).
/// Mirrors the sidecar's `usage/dashboard/stats` payload. Required fields here
/// are only required *within* the object — the whole `usage` object is itself
/// optional on [`AccountSnapshot`].
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AccountUsage {
    pub total_requests: u64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub today_requests: u64,
    pub today_tokens: u64,
    pub today_cost: f64,
    pub rpm: f64,
    pub tpm: f64,
    pub avg_latency_ms: f64,
}

/// One subscription/plan entry with its per-period (daily/weekly/monthly)
/// spend against an optional cap, plus an optional expiry. Mirrors one element
/// of the sidecar's `subscriptions/summary` array.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AccountSubscription {
    pub group_name: String,
    pub status: String,
    pub daily_used_usd: Option<f64>,
    pub daily_limit_usd: Option<f64>,
    pub weekly_used_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_used_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub expires_at: Option<String>,
}

/// Backend provider configuration, mirrored from the sidecar's
/// `GET /vibedev/providers`. This is the config the ACP agent actually uses
/// (backend `settings.json`), NOT Zed's native `language_models`. The sidecar
/// speaks camelCase JSON (`activeProvider`, `modelType`, ...).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersConfig {
    /// Currently active provider key (e.g. `firstParty`, `openai`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    /// Provider family: `anthropic` | `openai` | `gemini` | `grok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_type: Option<String>,
    /// Selected default model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Allowlist of selectable models.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_models: Vec<String>,
    /// Anthropic-name -> provider model id overrides.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub model_overrides: serde_json::Map<String, serde_json::Value>,
}

/// Which backend settings layer a [`ProvidersUpdate`] writes. The sidecar
/// validates these exact values (Plan 5 S-5 settings sources).
pub const PROVIDERS_SOURCE_USER: &str = "userSettings";

/// Payload accepted by `POST /vibedev/providers`. `target_source` selects which
/// settings layer the sidecar writes (`userSettings` | `projectSettings` |
/// `localSettings`); serialized as camelCase to match the sidecar.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_overrides: Option<serde_json::Map<String, serde_json::Value>>,
}

/// The handshake file the sidecar writes on startup (`~/.vibedev/endpoint.json`,
/// permissions locked to the current user): the loopback port plus a random
/// per-launch bearer token that gates every `/vibedev/*` call.
#[derive(Clone, Debug, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

/// `~/.vibedev` (overridable via `VIBEDEV_HOME`, matching the sidecar so a dev
/// can repoint both halves at a scratch dir).
fn vibedev_home() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("VIBEDEV_HOME") {
        return Ok(PathBuf::from(dir));
    }
    Ok(dirs::home_dir().context("no home dir")?.join(".vibedev"))
}

/// Read the chat-model ids VibeDev already cached to `~/.vibedev/auth.json`
/// (`available_models[].id`). This is the SAME list the chat model picker uses —
/// the sidecar's HTTP endpoints (`/vibedev/account`, `/vibedev/providers`) do not
/// currently expose it (they return an empty/absent `available_models`), so the
/// inline-assistant config reuses the cached file rather than re-fetching an
/// endpoint that yields no models at launch. Best-effort: any error (no file /
/// not signed in / shape change) yields an empty list; tolerates both object
/// (`{"id": ...}`) and bare-string entries.
pub fn read_cached_available_model_ids() -> Vec<String> {
    let Ok(home) = vibedev_home() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(home.join("auth.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(entries) = value
        .get("available_models")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| entry.as_str())
                .map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .collect()
}

/// VIBEDEV (V2-PLAN-8): read the gateway credentials VibeDev persisted to
/// `~/.vibedev/auth.json` (`api_key` + `proxy_url`). Used to inject
/// `OPENAI_API_KEY` / `OPENAI_BASE_URL` into a REMOTE agent's env so it is
/// "signed in" (and signs gateway requests) without copying `auth.json` onto
/// the remote host. Best-effort: any error (no file / not signed in / shape
/// change) yields `None`.
pub fn local_gateway_credentials() -> Option<(String, String)> {
    let home = vibedev_home().ok()?;
    let raw = std::fs::read_to_string(home.join("auth.json")).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    let api_key = value.get("api_key")?.as_str()?.to_string();
    let base_url = value.get("proxy_url")?.as_str()?.to_string();
    if api_key.is_empty() || base_url.is_empty() {
        return None;
    }
    Some((api_key, base_url))
}

/// Raw contents of the local `~/.vibedev/auth.json` — the full signed-in account
/// state (`api_key` + `proxy_url` + `available_models`). Injected into a REMOTE
/// agent's env as `VIBEDEV_AUTH_JSON` and written verbatim to the remote
/// `~/.vibedev/auth.json` by the self-provision script, so the remote agent is
/// signed in AND its model picker shows the account's real gateway models
/// (`getVibedevChatModels` reads `available_models`) instead of the default
/// claude tiers. `None` when not signed in / unreadable.
pub fn local_auth_json_raw() -> Option<String> {
    let home = vibedev_home().ok()?;
    let raw = std::fs::read_to_string(home.join("auth.json")).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    Some(raw)
}

/// Read `~/.vibedev/endpoint.json`. Errors while the sidecar is not yet up; the
/// panel surfaces that as a "backend not ready" placeholder rather than crashing.
pub fn read_endpoint() -> Result<Endpoint> {
    let path = vibedev_home()?.join("endpoint.json");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str::<Endpoint>(&raw).context("parse endpoint.json")
}

/// VIBEDEV: delete the sidecar handshake files (`endpoint.json` + the
/// single-instance `sidecar.lock` + the instance `refs` counter) from
/// `~/.vibedev`. Called at launch when a recorded endpoint exists but its port
/// is dead: a hard-killed previous IDE (kill -9 / crash / Task Manager) skips
/// the sidecar's SIGTERM cleanup, so these survive pointing at a now-dead port
/// (and `refs` leaks upward), and the account panel then reads them and reports
/// "Backend not ready" until — never — the fresh sidecar happens to reuse the
/// same rotated port. Purging them lets the fresh sidecar start from a clean
/// slate. Best-effort: a missing file is the normal case and not an error.
pub(crate) fn clear_stale_handshake() {
    let Ok(home) = vibedev_home() else {
        return;
    };
    for name in ["endpoint.json", "sidecar.lock", "refs"] {
        let path = home.join(name);
        if path.exists() {
            if let Err(error) = std::fs::remove_file(&path) {
                log::warn!(
                    "vibedev: failed to clear stale handshake file {}: {error:#}",
                    path.display()
                );
            }
        }
    }
}

/// Build a sidecar URL for the given path (always loopback).
fn sidecar_url(endpoint: &Endpoint, path: &str) -> String {
    format!("http://127.0.0.1:{}{}", endpoint.port, path)
}

/// Read the whole response body to a `String`.
async fn read_body(response: &mut http_client::Response<AsyncBody>) -> Result<String> {
    let mut body = String::new();
    response
        .body_mut()
        .read_to_string(&mut body)
        .await
        .context("reading response body")?;
    Ok(body)
}

/// Authed GET against the sidecar, returning the response body as a `String`.
async fn authed_get(http: &Arc<dyn HttpClient>, endpoint: &Endpoint, path: &str) -> Result<String> {
    let request = Request::get(sidecar_url(endpoint, path))
        .header("Authorization", format!("Bearer {}", endpoint.token))
        .body(AsyncBody::empty())
        .context("building sidecar GET request")?;
    let mut response = http
        .send(request)
        .await
        .with_context(|| format!("vibedev sidecar GET {path} failed"))?;
    anyhow::ensure!(
        response.status().is_success(),
        "sidecar {path} returned status {}",
        response.status()
    );
    read_body(&mut response).await
}

/// Authed JSON POST against the sidecar, returning the response body as a `String`.
async fn authed_post_json(
    http: &Arc<dyn HttpClient>,
    endpoint: &Endpoint,
    path: &str,
    json_body: String,
) -> Result<String> {
    let request = Request::post(sidecar_url(endpoint, path))
        .header("Authorization", format!("Bearer {}", endpoint.token))
        .header("Content-Type", "application/json")
        .body(AsyncBody::from(json_body))
        .context("building sidecar POST request")?;
    let mut response = http
        .send(request)
        .await
        .with_context(|| format!("vibedev sidecar POST {path} failed"))?;
    anyhow::ensure!(
        response.status().is_success(),
        "sidecar {path} returned status {}",
        response.status()
    );
    read_body(&mut response).await
}

/// Read-only account snapshot via the sidecar (which signs the sub2api call).
/// The frontend holds no secrets. Returns `Err` only on transport / handshake
/// failure; "not signed in" is a successful response with `signed_in: false`.
pub async fn fetch_account(http: Arc<dyn HttpClient>) -> Result<AccountSnapshot> {
    let endpoint = read_endpoint()?;
    let body = authed_get(&http, &endpoint, "/vibedev/account").await?;
    serde_json::from_str::<AccountSnapshot>(&body).context("parsing account snapshot")
}

/// Read the backend provider configuration the ACP agent actually uses.
pub async fn fetch_providers(http: Arc<dyn HttpClient>) -> Result<ProvidersConfig> {
    let endpoint = read_endpoint()?;
    let body = authed_get(&http, &endpoint, "/vibedev/providers").await?;
    serde_json::from_str::<ProvidersConfig>(&body).context("parsing providers config")
}

/// Persist a provider configuration change to the backend settings (the config
/// the agent reads on next session spawn). Takes effect for new sessions only.
pub async fn save_providers(http: Arc<dyn HttpClient>, update: ProvidersUpdate) -> Result<()> {
    let endpoint = read_endpoint()?;
    let json_body = serde_json::to_string(&update).context("serializing providers update")?;
    authed_post_json(&http, &endpoint, "/vibedev/providers", json_body).await?;
    Ok(())
}

/// Begin the VibeDev (sub2api) login flow.
///
/// NOTE: this is deliberately NOT Zed's `client.sign_in_with_optional_connect`
/// — that authenticates against zed.dev, the wrong backend. Here the sidecar
/// starts a sub2api login and returns a `login_url` to open in the browser. The
/// browser redirects to `vibedev://auth-callback?payload=<base64>`; the desktop
/// app relays that payload to the sidecar via [`relay_login_callback`]. The
/// sidecar exchanges/persists the api_key to `~/.vibedev/auth.json`. The panel
/// then polls [`fetch_account`] until `signed_in`.
pub async fn start_login(http: Arc<dyn HttpClient>) -> Result<String> {
    let endpoint = read_endpoint()?;
    let body = authed_post_json(
        &http,
        &endpoint,
        "/vibedev/login/start",
        "{}".to_string(),
    )
    .await?;
    let value: serde_json::Value = serde_json::from_str(&body).context("parsing login/start")?;
    let login_url = value
        .get("login_url")
        .and_then(serde_json::Value::as_str)
        .filter(|url| !url.is_empty())
        .context("login/start response missing login_url")?;
    Ok(login_url.to_string())
}

/// Relay an opaque login-callback payload to the sidecar.
///
/// The `vibedev://auth-callback?payload=<base64>` custom scheme is delivered to
/// the desktop app by the OS. The Zed fork extracts the opaque base64 `payload`
/// and hands it here; the sidecar decodes / validates / persists the api_key.
/// Zed never parses the api_key — it only forwards the blob (key isolation).
///
/// The actual OS-level `vibedev://` scheme *registration* is productization
/// (Plan 4); on Windows `gpui`'s `register_url_scheme` is unimplemented, so for
/// now this function is the callable relay a dev/manual trigger can drive (see
/// `VibedevSidecar::relay_login_callback`). Returns the verified email if the
/// sidecar reports one.
pub async fn relay_login_callback(http: Arc<dyn HttpClient>, payload: String) -> Result<Option<String>> {
    let endpoint = read_endpoint()?;
    let json_body = serde_json::json!({ "payload": payload }).to_string();
    let body = authed_post_json(&http, &endpoint, "/vibedev/login/callback", json_body).await?;
    let value: serde_json::Value =
        serde_json::from_str(&body).context("parsing login/callback")?;
    anyhow::ensure!(
        value.get("ok").and_then(serde_json::Value::as_bool) == Some(true),
        "login callback rejected by sidecar"
    );
    Ok(value
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string))
}

/// How often the account panel re-polls the sidecar for a fresh snapshot.
pub const ACCOUNT_POLL_INTERVAL: Duration = Duration::from_secs(20);

/// Drop the persistent api_key on the sidecar (`POST /vibedev/logout`).
///
/// The sidecar removes `~/.vibedev/auth.json` and flushes its in-memory
/// account snapshot, so the next `fetch_account` returns
/// `{signed_in:false, reason:'no_token'}`. The panel polls after this and
/// renders the signed-out state without a restart.
pub async fn sign_out(http: Arc<dyn HttpClient>) -> Result<()> {
    let endpoint = read_endpoint()?;
    let body = authed_post_json(&http, &endpoint, "/vibedev/logout", "{}".to_string()).await?;
    let value: serde_json::Value = serde_json::from_str(&body).context("parsing logout")?;
    anyhow::ensure!(
        value.get("ok").and_then(serde_json::Value::as_bool) == Some(true),
        "sidecar rejected logout: {body}"
    );
    Ok(())
}

/// Force-refresh the sidecar's cached model catalog
/// (`POST /vibedev/models/refresh`).
///
/// The sidecar re-fetches the gateway's `/v1/models` and rewrites the
/// `auth.json.available_models` cache, so the model picker reflects the
/// account's current upstream availability without waiting for a re-login.
/// Returns the number of models the gateway reported.
pub async fn refresh_models(http: Arc<dyn HttpClient>) -> Result<u64> {
    let endpoint = read_endpoint()?;
    let body =
        authed_post_json(&http, &endpoint, "/vibedev/models/refresh", "{}".to_string()).await?;
    let value: serde_json::Value =
        serde_json::from_str(&body).context("parsing models/refresh")?;
    anyhow::ensure!(
        value.get("ok").and_then(serde_json::Value::as_bool) == Some(true),
        "sidecar rejected models/refresh: {body}"
    );
    Ok(value
        .get("model_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0))
}
