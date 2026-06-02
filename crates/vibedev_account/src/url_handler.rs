//! Handle the `vibedev://auth-callback?payload=<base64>` custom-scheme URL.
//!
//! Lifecycle (Windows):
//!
//! 1. Browser finishes OAuth at `aitoken.bigopen.cn/vibedev-link`, redirects
//!    to `vibedev://auth-callback?payload=<urlencoded base64>`.
//! 2. Windows shell looks up `HKCU\Software\Classes\vibedev\shell\open\command`
//!    (installed by `crates/zed/resources/windows/zed.iss`) and launches the
//!    VibeDev exe with the URL as argv[1].
//! 3. If an instance is already running, `crates/zed/src/zed/windows_only_instance.rs`
//!    forwards the URL over its named pipe; otherwise the freshly-started
//!    process picks it up out of `Args::paths_or_urls` directly.
//! 4. Either way the URL flows through `OpenRequest::parse` into the
//!    `VibedevAuthCallback { payload }` open-request kind, which `main.rs`
//!    dispatches to [`handle`].
//!
//! What [`handle`] actually does:
//!   - URL-decode the `payload` query parameter (it was `urlencode`d by the
//!     browser; the sidecar wants the raw base64 string).
//!   - POST `{"payload": "<base64>"}` to the sidecar's
//!     `http://127.0.0.1:<port>/vibedev/login/callback` with the per-launch
//!     bearer token (so a third party who knows the port can't impersonate the
//!     OAuth flow). The sidecar decodes the blob, validates the signature, and
//!     persists `api_key` to `~/.vibedev/auth.json`.
//!   - Bring the VibeDev window forward (`cx.activate(true)`) so the user
//!     lands back in the IDE instead of staring at their browser.
//!
//! Failure surface: any error (sidecar down, network blip, invalid payload) is
//! logged at `error` and ignored. The account panel's existing 20-second poll
//! will surface the not-signed-in state on its own; we don't pop a modal here.
//!
//! Why this module lives in `vibedev_account` rather than `zed`: it talks to
//! the sidecar and knows the sub2api payload shape. The `zed` crate stays
//! responsible only for *delivering* the URL string.

use anyhow::{Context as _, Result};
use gpui::{App, Global};
use http_client::HttpClient;
use std::sync::Arc;

/// "Account just got updated by an OAuth callback" signal.
///
/// gpui doesn't have a free-standing `App`-level event bus, so we model this as
/// a monotonically-incrementing [`Global`]. Subscribers register via
/// `cx.observe_global::<VibedevAccountUpdated>(...)`; gpui notifies them when
/// `cx.set_global` is called with a new value of the same type, even if the
/// inner counter is the only thing that changed.
///
/// Why a counter rather than a snapshot: the actual account data lives in the
/// sidecar and the panel re-fetches it on notification. Carrying the snapshot
/// here would create a second source of truth for `signed_in`/`balance` and
/// the two would drift the moment sub2api state changes server-side.
#[derive(Clone, Copy, Debug, Default)]
pub struct VibedevAccountUpdated {
    /// Bumped once per successful auth-callback relay. Subscribers should
    /// re-fetch the snapshot rather than read meaning out of this number.
    pub revision: u64,
}

impl Global for VibedevAccountUpdated {}

/// Entry point called from `zed::handle_open_request`'s
/// `OpenRequestKind::VibedevAuthCallback` arm. Spawns a background task on the
/// app's executor (we shouldn't block the foreground while the sidecar replies)
/// and brings the window forward synchronously so the user sees the activation
/// even if the network round-trip is slow.
pub fn handle(payload: String, cx: &mut App) {
    // VIBEDEV: bring the running VibeDev window to the foreground before the
    // sidecar round-trip starts. The browser handed control back to us; if we
    // wait until the POST returns the user sees the IDE pop up "for no reason"
    // a second after their browser stops being active.
    cx.activate(true);

    let http = cx.http_client();
    cx.spawn(async move |cx| {
        match relay(http, payload).await {
            Ok(()) => {
                // Bump the global revision so any panel observing
                // `VibedevAccountUpdated` re-fetches immediately, instead of
                // waiting up to `ACCOUNT_POLL_INTERVAL` (20s) for the next poll.
                let _ = cx.update(|cx| {
                    let next = cx
                        .try_global::<VibedevAccountUpdated>()
                        .map(|s| s.revision.saturating_add(1))
                        .unwrap_or(1);
                    cx.set_global(VibedevAccountUpdated { revision: next });
                });
            }
            Err(error) => log::error!("vibedev auth callback relay failed: {error:#}"),
        }
    })
    .detach();
}

/// Forward the decoded payload to the sidecar. Public for the
/// `tests` module and for callers that want to await the POST (none today).
async fn relay(http: Arc<dyn HttpClient>, payload: String) -> Result<()> {
    let payload = decode_payload(&payload)?;
    let email = crate::relay_login_callback(http, payload).await?;
    if let Some(email) = email {
        log::info!("vibedev sign-in completed via auth-callback for {email}");
    } else {
        log::info!("vibedev sign-in completed via auth-callback");
    }
    Ok(())
}

/// URL-decode the `payload` query parameter. The browser percent-encodes the
/// base64 string (notably `+` → `%2B`, `=` → `%3D`); the sidecar wants the
/// original base64 untouched.
///
/// Errors only when the percent-encoded bytes aren't valid UTF-8 — base64
/// itself is ASCII, so a well-formed payload always round-trips cleanly.
fn decode_payload(raw: &str) -> Result<String> {
    let decoded = urlencoding::decode(raw)
        .context("payload was not valid percent-encoded UTF-8")?;
    Ok(decoded.into_owned())
}

/// Extract the `payload` query parameter from a `vibedev://auth-callback?...`
/// URL. Returns `Err` on missing/empty `payload` or unexpected host/path.
///
/// Kept here (rather than in `open_listener.rs`) so the URL-shape contract
/// stays next to the handler that consumes it. The `zed` crate calls this
/// during `OpenRequest::parse` and stores the raw payload on the open-request
/// kind so the handler doesn't have to re-parse the URL.
pub fn parse_callback_url(url: &str) -> Result<String> {
    let rest = url
        .strip_prefix("vibedev://")
        .context("not a vibedev:// URL")?;
    // Tolerate trailing-slash variants: `auth-callback?...`, `auth-callback/?...`.
    let (host_path, query) = rest
        .split_once('?')
        .context("missing query string on vibedev:// URL")?;
    let host_path = host_path.trim_end_matches('/');
    anyhow::ensure!(
        host_path == "auth-callback",
        "unexpected vibedev:// route: {host_path:?} (expected `auth-callback`)"
    );

    let payload = url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(key, value)| (key == "payload").then_some(value))
        .filter(|payload| !payload.is_empty())
        .context("vibedev://auth-callback missing `payload` query parameter")?
        .into_owned();
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_payload_from_well_formed_url() {
        let payload = parse_callback_url(
            "vibedev://auth-callback?payload=eyJrIjoidiJ9",
        )
        .unwrap();
        assert_eq!(payload, "eyJrIjoidiJ9");
    }

    #[test]
    fn tolerates_trailing_slash_before_query() {
        let payload = parse_callback_url(
            "vibedev://auth-callback/?payload=eyJrIjoidiJ9",
        )
        .unwrap();
        assert_eq!(payload, "eyJrIjoidiJ9");
    }

    #[test]
    fn url_decodes_payload_value() {
        // base64 commonly contains `+` and `=` which the browser percent-encodes.
        let payload = parse_callback_url(
            "vibedev://auth-callback?payload=a%2Bb%3D%3D",
        )
        .unwrap();
        // form_urlencoded::parse decodes the value for us; no second pass needed.
        assert_eq!(payload, "a+b==");
    }

    #[test]
    fn rejects_unknown_route() {
        let err = parse_callback_url("vibedev://something-else?payload=x").unwrap_err();
        assert!(format!("{err}").contains("unexpected vibedev:// route"));
    }

    #[test]
    fn rejects_missing_payload() {
        let err = parse_callback_url("vibedev://auth-callback?state=x").unwrap_err();
        assert!(format!("{err}").contains("missing `payload`"));
    }

    #[test]
    fn rejects_empty_payload() {
        let err = parse_callback_url("vibedev://auth-callback?payload=").unwrap_err();
        assert!(format!("{err}").contains("missing `payload`"));
    }

    #[test]
    fn rejects_non_vibedev_scheme() {
        let err = parse_callback_url("https://example.com/?payload=x").unwrap_err();
        assert!(format!("{err}").contains("not a vibedev:// URL"));
    }

    #[test]
    fn decode_payload_passes_plain_base64_through() {
        assert_eq!(decode_payload("eyJrIjoidiJ9").unwrap(), "eyJrIjoidiJ9");
    }

    #[test]
    fn decode_payload_unescapes_percent_encoded() {
        assert_eq!(decode_payload("a%2Bb%3D%3D").unwrap(), "a+b==");
    }
}
