//! VibeDev update-check: poll a remote `latest.json` and expose an
//! [`UpdateStatus`] global other panels can render as a "new version" banner.
//!
//! VibeDev disabled Zed's `auto_update` crate entirely (commit `dcb40a7bf0`)
//! because Zed's updater pulls bytes from `zed.dev` (wrong vendor) and runs an
//! in-process install. This module replaces that signal with the minimum useful
//! piece: a non-intrusive periodic check against a JSON file we host on
//! `aitoken.bigopen.cn`, populating a global the UI can read to render a
//! "v1.1 available - download" hint. No bytes are downloaded, no install
//! happens - clicking through to the download URL is opt-in.
//!
//! Wiring: `vibedev_account::init` (called from `main.rs`) invokes [`init`],
//! which `cx.spawn`s a long-lived task. The task fetches
//! `https://aitoken.bigopen.cn/vibedev/latest.json`, compares semver against
//! `AppVersion::global(cx)`, calls `cx.set_global(UpdateStatus { ... })`, then
//! sleeps 24h and repeats. HTTP/parse failures are logged at `warn` and leave
//! the previous (or default) status in place - the panel stays silent on the
//! sad path.
//!
//! Read side: any UI surface calls [`current`] to get the latest snapshot
//! and renders accordingly. The first read before the first successful poll
//! returns the default `available: false` snapshot.

use anyhow::{Context as _, Result};
use futures::AsyncReadExt as _;
use gpui::{App, Global};
use http_client::{AsyncBody, HttpClient, Request};
use release_channel::AppVersion;
use semver::Version;
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

/// Where the client polls. Served as a static file by the release host's nginx
/// (the deploy host + on-disk path are environment-specific and deliberately
/// not stored in this repo); see `integrations/update-check/README.md` for the
/// deploy procedure.
const LATEST_JSON_URL: &str = "https://aitoken.bigopen.cn/vibedev/latest.json";

/// How often to re-poll. 24h is intentional: this is a non-urgent "by the way,
/// a new release exists" signal, not a hot-patch channel. Users who launch
/// daily see new releases within a day of `latest.json` being pushed.
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Snapshot of what the remote `latest.json` says, post-comparison against the
/// running binary's version. Stored as a [`Global`] so any panel can read it
/// synchronously off the app.
///
/// `available: false` is the both the default (before the first poll) and the
/// steady state when the running binary is at or ahead of the remote version.
/// The optional fields are populated when (and only when) `available: true`,
/// so a renderer can match on `available` and trust the rest is `Some`.
#[derive(Clone, Debug, Default)]
pub struct UpdateStatus {
    pub available: bool,
    pub latest_version: Option<String>,
    pub download_url: Option<String>,
    pub release_notes: Option<String>,
}

impl Global for UpdateStatus {}

/// Schema of `latest.json` as hosted on the server. Kept private - callers
/// only see the post-comparison [`UpdateStatus`].
#[derive(Debug, Deserialize)]
struct LatestJson {
    version: String,
    #[serde(default)]
    #[allow(dead_code)] // future surface (e.g. "released N days ago" copy)
    released_at: Option<String>,
    download_url: String,
    #[serde(default)]
    release_notes: Option<String>,
}

/// Read the current update status. Returns the default
/// (`available: false`, all `None`) until the first successful poll completes
/// - every other path through the poll loop preserves whatever was last set
/// rather than clobbering it with a default on transient failures.
pub fn current(cx: &App) -> UpdateStatus {
    cx.try_global::<UpdateStatus>()
        .cloned()
        .unwrap_or_default()
}

/// Spawn the 24h polling loop. Idempotent-ish: calling twice would launch two
/// loops, but `vibedev_account::init` is invoked once from `main.rs` (the slot
/// previously held by `auto_update::init`).
pub fn init(cx: &mut App) {
    // VIBEDEV: seed the global so `current(cx)` is always safe to call. A
    // missing global before the first poll would force every reader to
    // distinguish "no global yet" from "default snapshot", which is noise the
    // UI shouldn't have to carry.
    cx.set_global(UpdateStatus::default());
    let http = cx.http_client();
    // VIBEDEV: snapshot the running binary's version once at init. AppVersion
    // is a process-lifetime constant, so hopping back to the app on every
    // iteration just to re-read it would be pure ceremony.
    let current_version = AppVersion::global(cx);
    cx.spawn(async move |cx| {
        loop {
            match fetch_latest(&http).await {
                Ok(latest) => {
                    let status = compare(&current_version, &latest);
                    if status.available {
                        log::info!(
                            "vibedev update-check: remote {} > local {}, surfacing banner",
                            latest.version,
                            current_version
                        );
                    } else {
                        log::debug!(
                            "vibedev update-check: local {} is current (remote {})",
                            current_version,
                            latest.version
                        );
                    }
                    // VIBEDEV: AsyncApp::update panics if the app is gone
                    // (final shutdown), but at that point the whole process is
                    // tearing down - the detached task is already dead in the
                    // water either way, so no need to special-case it.
                    cx.update(|cx| cx.set_global(status));
                }
                Err(error) => {
                    log::warn!("vibedev update-check failed: {error:#}");
                }
            }
            cx.background_executor()
                .timer(UPDATE_CHECK_INTERVAL)
                .await;
        }
    })
    .detach();
}

/// Fetch + parse `latest.json`. Body size cap is implicit (the file is ~300 B);
/// the http_client uses the regular per-request defaults.
async fn fetch_latest(http: &Arc<dyn HttpClient>) -> Result<LatestJson> {
    let request = Request::get(LATEST_JSON_URL)
        .body(AsyncBody::empty())
        .context("building latest.json GET request")?;
    let mut response = http
        .send(request)
        .await
        .context("fetching latest.json")?;
    anyhow::ensure!(
        response.status().is_success(),
        "latest.json returned status {}",
        response.status()
    );
    let mut body = String::new();
    response
        .body_mut()
        .read_to_string(&mut body)
        .await
        .context("reading latest.json body")?;
    serde_json::from_str::<LatestJson>(&body).context("parsing latest.json")
}

/// Strict semver comparison: an update is "available" only when the remote
/// `version` parses to a valid semver AND is strictly greater than the local
/// version. Pre-release identifiers participate in semver's spec ordering;
/// build metadata is ignored. A junk remote `version` (unparseable) is logged
/// at `warn` and treated as "no update" rather than panicking - bad server
/// state shouldn't take the panel down.
fn compare(current_version: &Version, latest: &LatestJson) -> UpdateStatus {
    let latest_version = match Version::parse(&latest.version) {
        Ok(version) => version,
        Err(error) => {
            log::warn!(
                "vibedev update-check: remote `version` {:?} is not valid semver ({error}); skipping",
                latest.version
            );
            return UpdateStatus::default();
        }
    };
    if &latest_version > current_version {
        UpdateStatus {
            available: true,
            latest_version: Some(latest.version.clone()),
            download_url: Some(latest.download_url.clone()),
            release_notes: latest.release_notes.clone(),
        }
    } else {
        UpdateStatus::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_latest(version: &str) -> LatestJson {
        LatestJson {
            version: version.to_string(),
            released_at: None,
            download_url: format!("https://aitoken.bigopen.cn/downloads/VibeDev-{version}-x86_64.exe"),
            release_notes: Some("notes".into()),
        }
    }

    #[test]
    fn newer_remote_surfaces_update() {
        let status = compare(&Version::new(1, 0, 0), &make_latest("1.5.0"));
        assert!(status.available);
        assert_eq!(status.latest_version.as_deref(), Some("1.5.0"));
        assert!(status.download_url.is_some());
    }

    #[test]
    fn same_or_older_remote_is_silent() {
        let same = compare(&Version::new(1, 5, 0), &make_latest("1.5.0"));
        assert!(!same.available);
        let older = compare(&Version::new(1, 5, 0), &make_latest("1.4.9"));
        assert!(!older.available);
    }

    #[test]
    fn invalid_remote_version_is_silent_not_panic() {
        let status = compare(
            &Version::new(1, 0, 0),
            &LatestJson {
                version: "not-a-version".into(),
                released_at: None,
                download_url: "x".into(),
                release_notes: None,
            },
        );
        assert!(!status.available);
    }
}
