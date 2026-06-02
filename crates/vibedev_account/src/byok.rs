//! VIBEDEV BYOK (bring-your-own-key) provider enumeration.
//!
//! Task F1 of the BYOK feature. The VibeDev backend (separate repo) reads the
//! `VIBEDEV_BYOK` env var to let its ACP agent use the *user's own* provider
//! keys instead of the gateway. This module is the Zed half: it walks every
//! authenticated provider in [`LanguageModelRegistry`], reads each provider's
//! api_url from [`AllLanguageModelSettings`], pulls the matching key out of the
//! system keychain, and serializes the whole thing to the frozen JSON contract.
//!
//! The JSON shape is a **FROZEN contract** — the backend parses exactly these
//! field names (`baseURL` / `apiKey` are camelCase via serde rename). Do not
//! rename fields without coordinating the backend parser.
//!
//! A LATER task (F2) injects the produced string into the agent child process
//! env. This module only produces the collector + types; it performs no
//! injection.

use anyhow::Result;
use gpui::{App, AsyncApp, Task};
use language_model::{ApiKey, LanguageModelRegistry};
use language_models::AllLanguageModelSettings;
use serde::Serialize;
use std::collections::HashMap;
// `get_global` is a `settings::Settings` trait method; the trait must be in
// scope to call `AllLanguageModelSettings::get_global`.
use settings::Settings as _;

/// One authenticated provider entry in the `VIBEDEV_BYOK` contract.
///
/// `protocol` tells the backend which wire format to speak to `baseURL`
/// ("openai" | "anthropic" | "gemini"). `apiKey` is the raw key read from the
/// system keychain (empty string if the keychain read failed — the backend
/// treats an empty key as "skip this provider").
#[derive(Serialize)]
pub struct ByokProvider {
    pub id: String,
    pub label: String,
    pub protocol: &'static str, // "openai" | "anthropic" | "gemini"
    #[serde(rename = "baseURL")]
    pub base_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub models: Vec<String>,
}

/// Top-level `VIBEDEV_BYOK` payload: `{"providers":[...]}`.
#[derive(Serialize)]
pub struct ByokMap {
    pub providers: Vec<ByokProvider>,
}

/// provider id → 协议（wire protocol）。`None` = 不纳入 BYOK（gateway-only /
/// 无独立 key 概念：`vibedev` 走 gateway，`bedrock` 用 AWS 凭证而非 api_url，
/// `zed.dev` 是 Zed 自家后端）。openai_compatible 的自定义 provider 不在此表里，
/// 由 [`protocol_and_url`] 的 `_ =>` 分支统一按 "openai" 处理。
fn protocol_for(id: &str) -> Option<&'static str> {
    match id {
        "vibedev" | "bedrock" | "zed.dev" => None,
        "anthropic" => Some("anthropic"),
        "google" => Some("gemini"),
        "openai" | "deepseek" | "mistral" | "ollama" | "lmstudio" | "open_router" | "x_ai"
        | "vercel.ai_gateway" => Some("openai"),
        // openai_compatible 自定义 provider（除上面的内建）一律 openai 协议。
        _ => Some("openai"),
    }
}

/// provider id → (协议, api_url)；`None` = 不纳入 BYOK。协议沿用
/// [`protocol_for`]（单一事实来源），api_url 从内建 provider 的专属 settings 字段
/// 取；未知 id（openai_compatible 自定义 provider）按 id 在 `s.openai_compatible`
/// 里查 api_url，查不到则不纳入。
fn protocol_and_url(id: &str, s: &AllLanguageModelSettings) -> Option<(&'static str, String)> {
    let protocol = protocol_for(id)?;
    let api_url = match id {
        "anthropic" => s.anthropic.api_url.clone(),
        "google" => s.google.api_url.clone(),
        "openai" => s.openai.api_url.clone(),
        "deepseek" => s.deepseek.api_url.clone(),
        "mistral" => s.mistral.api_url.clone(),
        "ollama" => s.ollama.api_url.clone(),
        "lmstudio" => s.lmstudio.api_url.clone(),
        "open_router" => s.open_router.api_url.clone(),
        "x_ai" => s.x_ai.api_url.clone(),
        "vercel.ai_gateway" => s.vercel_ai_gateway.api_url.clone(),
        // openai_compatible 自定义 provider：没有专属字段，按 id 查 map。
        other => s.openai_compatible.get(other)?.api_url.clone(),
    };
    Some((protocol, api_url))
}

/// 收集已认证 provider → `VIBEDEV_BYOK` JSON 字符串（空时 `{"providers":[]}`）。
///
/// 同步阶段先枚举 registry（`read_global` 借用在 `.providers()` 处即释放，返回的
/// 是 owned `Vec<Arc<..>>`），把每个 provider 的 id/协议/api_url/label/models 读进
/// `pending`——`AllLanguageModelSettings` 不实现 `Clone`，所以不把 settings 引用带
/// 进异步闭包，而是在同步阶段就把 api_url 读出来。异步阶段只做 keychain 读取
/// （`load_from_system_keychain` 需要 `&AsyncApp` + 可 `.await`）。
///
/// keychain 读取失败（无 key / 非 utf8 / 该 url 下没存）按空 key 处理而非整体失败，
/// 后端把空 key 当作"跳过该 provider"。
pub fn collect_byok_map(cx: &mut App) -> Task<Result<String>> {
    let credentials = zed_credentials_provider::global(cx);

    cx.spawn(async move |cx| {
        // CRITICAL: load each candidate provider's key from the system keychain
        // FIRST. `is_authenticated()` is false until `authenticate()` runs (the
        // key is lazy-loaded — the exact valve the inline assistant hit, see
        // vibedev_inline_assistant). Without this, a freshly-launched agent session
        // enumerates ZERO authenticated BYOK providers and ships an empty
        // VIBEDEV_BYOK, so the backend has no user models to merge and the picker
        // shows only gateway models.
        authenticate_byok_providers(cx).await;

        // Now enumerate authenticated providers + read their api_url/models
        // synchronously (settings isn't Clone, so we don't hold it across awaits).
        // (id, protocol, api_url, label, models)
        let pending: Vec<(String, &'static str, String, String, Vec<String>)> = cx
            .update(|cx| {
                let settings = AllLanguageModelSettings::get_global(cx);
                let mut pending = Vec::new();
                for provider in LanguageModelRegistry::read_global(cx).providers() {
                    if !provider.is_authenticated(cx) {
                        continue;
                    }
                    let id = provider.id().0.to_string();
                    let Some((protocol, api_url)) = protocol_and_url(&id, settings) else {
                        continue;
                    };
                    let models: Vec<String> = provider
                        .provided_models(cx)
                        .iter()
                        .map(|m| m.id().0.to_string())
                        .collect();
                    if models.is_empty() {
                        continue;
                    }
                    let label = format!("{}（你的 Key）", provider.name().0);
                    pending.push((id, protocol, api_url, label, models));
                }
                pending
            });

        let mut providers = Vec::new();
        for (id, protocol, api_url, label, models) in pending {
            let api_key = ApiKey::load_from_system_keychain(&api_url, credentials.as_ref(), cx)
                .await
                .ok()
                .map(|k| k.key().to_string())
                .unwrap_or_default();
            providers.push(ByokProvider {
                id,
                label,
                protocol,
                base_url: api_url,
                api_key,
                models,
            });
        }
        Ok(serde_json::to_string(&ByokMap { providers })?)
    })
}

/// Load every BYOK-candidate provider's API key from the system keychain by
/// calling `authenticate()` on it. A provider's `is_authenticated()` returns
/// false until this runs (keys are lazy-loaded), so both [`collect_byok_map`]
/// and the picker grouping ([`AcpModelSelector::list_models`]) MUST call this
/// first or they skip providers the user has actually configured. Idempotent:
/// `authenticate` early-returns once the key is loaded. Errors are ignored — a
/// provider with no key stays unauthenticated and is correctly excluded.
pub async fn authenticate_byok_providers(cx: &mut AsyncApp) {
    // `AsyncApp::update` in this fork returns the closure value directly (not a
    // Result) — see vibedev_inline_assistant.
    let providers = cx.update(|cx| LanguageModelRegistry::read_global(cx).providers());
    for provider in providers {
        if protocol_for(provider.id().0.as_ref()).is_none() {
            continue; // gateway / no-standalone-key provider — never a BYOK source
        }
        // `update`'s closure is FnOnce + moves its captures, so clone a handle
        // for the auth-check and keep `provider` for the authenticate call
        // (Arc clone = a cheap refcount bump).
        let probe = provider.clone();
        if cx.update(move |cx| probe.is_authenticated(cx)) {
            continue; // key already loaded
        }
        let task = cx.update(move |cx| provider.authenticate(cx));
        let _ = task.await;
    }
}

/// model_id → provider 展示名（仅含已认证的 BYOK provider；网关/排除项不在内）。
/// 供 ACP 模型选择器分组用——只需 label + models，无需 keychain，同步即可。
///
/// 与 [`collect_byok_map`] 共用同一套准入条件（`is_authenticated` +
/// [`protocol_and_url`] 非 `None`），但跳过 keychain 异步读取：分组只关心
/// "这个 model 属于哪个厂商"，不碰 key。`vibedev`/`bedrock`/`zed.dev` 及无
/// api_url 的 provider 不在结果里，其 model 在调用方落入"网关"兜底分组。
pub fn collect_byok_groups(cx: &App) -> HashMap<String, String> {
    let settings = AllLanguageModelSettings::get_global(cx);
    let mut groups = HashMap::new();
    for provider in LanguageModelRegistry::read_global(cx).providers() {
        if !provider.is_authenticated(cx) {
            continue;
        }
        let id = provider.id().0.to_string();
        if protocol_and_url(&id, settings).is_none() {
            continue; // 排除 vibedev/bedrock/zed.dev 及无 api_url 的
        }
        let label = format!("{}（你的 Key）", provider.name().0);
        for m in provider.provided_models(cx) {
            groups.insert(m.id().0.to_string(), label.clone());
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::protocol_for;

    // protocol_and_url needs a live `AllLanguageModelSettings` (no public ctor,
    // not Clone), so we unit-test the id→protocol mapping via the extracted
    // `protocol_for` helper instead. protocol_and_url defers to the same mapping
    // for its protocol component; the api_url plumbing is exercised by
    // collect_byok_map at runtime against a real registry (F2 / manual).
    #[test]
    fn known_provider_protocols() {
        // gateway-only / no-standalone-key providers are excluded.
        assert_eq!(protocol_for("vibedev"), None);
        assert_eq!(protocol_for("bedrock"), None);
        assert_eq!(protocol_for("zed.dev"), None);

        // anthropic + gemini speak their own wire protocols.
        assert_eq!(protocol_for("anthropic"), Some("anthropic"));
        assert_eq!(protocol_for("google"), Some("gemini"));

        // everything else is openai-compatible.
        assert_eq!(protocol_for("openai"), Some("openai"));
        assert_eq!(protocol_for("deepseek"), Some("openai"));
        assert_eq!(protocol_for("mistral"), Some("openai"));
        assert_eq!(protocol_for("ollama"), Some("openai"));
        assert_eq!(protocol_for("lmstudio"), Some("openai"));
        assert_eq!(protocol_for("open_router"), Some("openai"));
        assert_eq!(protocol_for("x_ai"), Some("openai"));
        assert_eq!(protocol_for("vercel.ai_gateway"), Some("openai"));

        // unknown id == openai_compatible custom provider → openai protocol.
        assert_eq!(protocol_for("my_custom_provider"), Some("openai"));
    }
}
