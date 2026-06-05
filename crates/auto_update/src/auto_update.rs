use anyhow::{Context as _, Result};
use client::Client;
use db::kvp::KeyValueStore;
use futures_lite::StreamExt;
use gpui::{
    App, AppContext as _, AsyncApp, BackgroundExecutor, Context, Entity, Global, Task, TaskExt,
    Window, actions,
};
use http_client::{HttpClient, HttpClientWithUrl};
use paths::remote_servers_dir;
use release_channel::{AppCommitSha, ReleaseChannel};
use semver::Version;
use serde::{Deserialize, Serialize};
use settings::{RegisterSetting, Settings, SettingsStore};
use smol::fs::File;
use smol::{fs, io::AsyncReadExt};
use std::mem;
use std::{
    env::{
        self,
        consts::{ARCH, OS},
    },
    ffi::OsStr,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use util::command::new_command;
use workspace::Workspace;

const SHOULD_SHOW_UPDATE_NOTIFICATION_KEY: &str = "auto-updater-should-show-updated-notification";

#[derive(Debug)]
struct MissingDependencyError(String);

impl std::fmt::Display for MissingDependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MissingDependencyError {}
const POLL_INTERVAL: Duration = Duration::from_secs(60 * 60);
const NIGHTLY_POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);
const REMOTE_SERVER_CACHE_LIMIT: usize = 5;

#[cfg(target_os = "linux")]
fn linux_rsync_install_hint() -> &'static str {
    let os_release = match std::fs::read_to_string("/etc/os-release") {
        Ok(os_release) => os_release,
        Err(_) => return "Please install rsync using your package manager",
    };

    let mut distribution_ids = Vec::new();
    for line in os_release.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("ID=") {
            distribution_ids.push(value.trim_matches('"').to_ascii_lowercase());
        } else if let Some(value) = trimmed.strip_prefix("ID_LIKE=") {
            for id in value.trim_matches('"').split_whitespace() {
                distribution_ids.push(id.to_ascii_lowercase());
            }
        }
    }

    let package_manager_hint = if distribution_ids
        .iter()
        .any(|distribution_id| distribution_id == "arch")
    {
        Some("Install it with: sudo pacman -S rsync")
    } else if distribution_ids
        .iter()
        .any(|distribution_id| distribution_id == "debian" || distribution_id == "ubuntu")
    {
        Some("Install it with: sudo apt install rsync")
    } else if distribution_ids.iter().any(|distribution_id| {
        distribution_id == "fedora"
            || distribution_id == "rhel"
            || distribution_id == "centos"
            || distribution_id == "rocky"
            || distribution_id == "almalinux"
    }) {
        Some("Install it with: sudo dnf install rsync")
    } else if distribution_ids
        .iter()
        .any(|distribution_id| distribution_id == "nixos")
    {
        Some("Install pkgs.rsync from nixpkgs")
    } else {
        None
    };

    package_manager_hint.unwrap_or("Please install rsync using your package manager")
}

actions!(
    auto_update,
    [
        /// Checks for available updates.
        Check,
        /// Dismisses the update error message.
        DismissMessage,
        /// Opens the release notes for the current version in a browser.
        ViewReleaseNotes,
    ]
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionCheckType {
    Sha(AppCommitSha),
    Semantic(Version),
}

#[derive(Serialize, Debug)]
pub struct AssetQuery<'a> {
    asset: &'a str,
    os: &'a str,
    arch: &'a str,
    metrics_id: Option<&'a str>,
    system_id: Option<&'a str>,
    is_staff: Option<bool>,
}

#[derive(Clone, Debug)]
pub enum AutoUpdateStatus {
    Idle,
    Checking,
    Downloading { version: VersionCheckType },
    Installing { version: VersionCheckType },
    Updated { version: VersionCheckType },
    Errored { error: Arc<anyhow::Error> },
}

impl PartialEq for AutoUpdateStatus {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (AutoUpdateStatus::Idle, AutoUpdateStatus::Idle) => true,
            (AutoUpdateStatus::Checking, AutoUpdateStatus::Checking) => true,
            (
                AutoUpdateStatus::Downloading { version: v1 },
                AutoUpdateStatus::Downloading { version: v2 },
            ) => v1 == v2,
            (
                AutoUpdateStatus::Installing { version: v1 },
                AutoUpdateStatus::Installing { version: v2 },
            ) => v1 == v2,
            (
                AutoUpdateStatus::Updated { version: v1 },
                AutoUpdateStatus::Updated { version: v2 },
            ) => v1 == v2,
            (AutoUpdateStatus::Errored { error: e1 }, AutoUpdateStatus::Errored { error: e2 }) => {
                e1.to_string() == e2.to_string()
            }
            _ => false,
        }
    }
}

impl AutoUpdateStatus {
    pub fn is_updated(&self) -> bool {
        matches!(self, Self::Updated { .. })
    }
}

pub struct AutoUpdater {
    status: AutoUpdateStatus,
    current_version: Version,
    client: Arc<Client>,
    pending_poll: Option<Task<Option<()>>>,
    quit_subscription: Option<gpui::Subscription>,
    update_check_type: UpdateCheckType,
    // VIBEDEV (agent-OTA): the version of a freshly-staged `agent.new/`, set once
    // `check_and_stage_agent_update` publishes a stage on the poll. Decoupled from
    // the app `status` (the agent version line is independent of the app version).
    // A later UI task reads this via `staged_agent_version()` to show a "restart to
    // apply the agent update" notice; this crate intentionally builds no UI for it.
    staged_agent_version: Option<Version>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ReleaseAsset {
    pub version: String,
    pub url: String,
}

struct MacOsUnmounter<'a> {
    mount_path: PathBuf,
    background_executor: &'a BackgroundExecutor,
}

impl Drop for MacOsUnmounter<'_> {
    fn drop(&mut self) {
        let mount_path = mem::take(&mut self.mount_path);
        self.background_executor
            .spawn(async move {
                let unmount_output = new_command("hdiutil")
                    .args(["detach", "-force"])
                    .arg(&mount_path)
                    .output()
                    .await;
                match unmount_output {
                    Ok(output) if output.status.success() => {
                        log::info!("Successfully unmounted the disk image");
                    }
                    Ok(output) => {
                        log::error!(
                            "Failed to unmount disk image: {:?}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    Err(error) => {
                        log::error!("Error while trying to unmount disk image: {:?}", error);
                    }
                }
            })
            .detach();
    }
}

#[derive(Clone, Copy, Debug, RegisterSetting)]
struct AutoUpdateSetting(bool);

/// Whether or not to automatically check for updates.
///
/// Default: true
impl Settings for AutoUpdateSetting {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self(content.auto_update.unwrap())
    }
}

#[derive(Default)]
struct GlobalAutoUpdate(Option<Entity<AutoUpdater>>);

impl Global for GlobalAutoUpdate {}

pub fn init(client: Arc<Client>, cx: &mut App) {
    // VIBEDEV: this body runs (the prior early-return was removed) so the
    // GlobalAutoUpdate is registered — required by SSH remote-server download
    // (download_remote_server_release / get_remote_server_release_url) AND by
    // app self-update, which share get_release_asset. That asset URL is
    // repointed at VibeDev's own server (see get_release_asset below); it never
    // hits zed.dev.
    //
    // App self-update POLLING is gated by two conditions (see below): the
    // release channel's `poll_for_updates()` (false on `dev`, true on
    // `stable`/`nightly`/`preview`) AND the `auto_update` setting
    // (`AutoUpdateSetting::get_global(cx).0`, which defaults to `true` in
    // assets/settings/default.json). So a `dev` build never polls for app
    // self-update regardless of the setting, while a `stable` build polls
    // VibeDev's own server (never zed.dev) unless the user sets
    // `auto_update: false`. The SettingsStore observer arms/disarms polling
    // live when the user toggles the setting.
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|_, action, window, cx| check(action, window, cx));

        workspace.register_action(|_, action, _, cx| {
            view_release_notes(action, cx);
        });
    })
    .detach();

    let version = release_channel::AppVersion::global(cx);
    let auto_updater = cx.new(|cx| {
        let updater = AutoUpdater::new(version, client, cx);

        let poll_for_updates = ReleaseChannel::try_global(cx)
            .map(|channel| channel.poll_for_updates())
            .unwrap_or(false);

        if option_env!("ZED_UPDATE_EXPLANATION").is_none()
            && env::var("ZED_UPDATE_EXPLANATION").is_err()
            && poll_for_updates
        {
            let mut update_subscription = AutoUpdateSetting::get_global(cx)
                .0
                .then(|| updater.start_polling(cx));

            cx.observe_global::<SettingsStore>(move |updater: &mut AutoUpdater, cx| {
                if AutoUpdateSetting::get_global(cx).0 {
                    if update_subscription.is_none() {
                        update_subscription = Some(updater.start_polling(cx))
                    }
                } else {
                    update_subscription.take();
                }
            })
            .detach();
        }

        updater
    });
    cx.set_global(GlobalAutoUpdate(Some(auto_updater)));
}

pub fn check(_: &Check, window: &mut Window, cx: &mut App) {
    if let Some(message) = option_env!("ZED_UPDATE_EXPLANATION")
        .map(ToOwned::to_owned)
        .or_else(|| env::var("ZED_UPDATE_EXPLANATION").ok())
    {
        drop(window.prompt(
            gpui::PromptLevel::Info,
            "Zed was installed via a package manager.",
            Some(&message),
            &["Ok"],
            cx,
        ));
        return;
    }

    if !ReleaseChannel::try_global(cx)
        .map(|channel| channel.poll_for_updates())
        .unwrap_or(false)
    {
        return;
    }

    if let Some(updater) = AutoUpdater::get(cx) {
        updater.update(cx, |updater, cx| updater.poll(UpdateCheckType::Manual, cx));
    } else {
        drop(window.prompt(
            gpui::PromptLevel::Info,
            "Could not check for updates",
            Some("Auto-updates disabled for non-bundled app."),
            &["Ok"],
            cx,
        ));
    }
}

pub fn release_notes_url(cx: &mut App) -> Option<String> {
    let release_channel = ReleaseChannel::try_global(cx)?;
    let url = match release_channel {
        ReleaseChannel::Stable | ReleaseChannel::Preview => {
            let auto_updater = AutoUpdater::get(cx)?;
            let auto_updater = auto_updater.read(cx);
            let mut current_version = auto_updater.current_version.clone();
            current_version.pre = semver::Prerelease::EMPTY;
            current_version.build = semver::BuildMetadata::EMPTY;
            let release_channel = release_channel.dev_name();
            let path = format!("/releases/{release_channel}/{current_version}");
            auto_updater.client.http_client().build_url(&path)
        }
        // VIBEDEV: release notes point at our fork's commit history, not zed-industries/zed.
        ReleaseChannel::Nightly => {
            "https://github.com/CrisLIUning/Vibedev-ide/commits/main/".to_string()
        }
        ReleaseChannel::Dev => {
            "https://github.com/CrisLIUning/Vibedev-ide/commits/main/".to_string()
        }
    };
    Some(url)
}

pub fn view_release_notes(_: &ViewReleaseNotes, cx: &mut App) -> Option<()> {
    let url = release_notes_url(cx)?;
    cx.open_url(&url);
    None
}

#[cfg(not(target_os = "windows"))]
struct InstallerDir(tempfile::TempDir);

#[cfg(not(target_os = "windows"))]
impl InstallerDir {
    async fn new() -> Result<Self> {
        Ok(Self(
            tempfile::Builder::new()
                .prefix("zed-auto-update")
                .tempdir()?,
        ))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

#[cfg(target_os = "windows")]
struct InstallerDir(PathBuf);

#[cfg(target_os = "windows")]
impl InstallerDir {
    async fn new() -> Result<Self> {
        let installer_dir = std::env::current_exe()?
            .parent()
            .context("No parent dir for Zed.exe")?
            .join("updates");
        if smol::fs::metadata(&installer_dir).await.is_ok() {
            smol::fs::remove_dir_all(&installer_dir).await?;
        }
        smol::fs::create_dir(&installer_dir).await?;
        Ok(Self(installer_dir))
    }

    fn path(&self) -> &Path {
        self.0.as_path()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UpdateCheckType {
    Automatic,
    Manual,
}

impl UpdateCheckType {
    pub fn is_manual(self) -> bool {
        self == Self::Manual
    }
}

impl AutoUpdater {
    pub fn get(cx: &mut App) -> Option<Entity<Self>> {
        cx.default_global::<GlobalAutoUpdate>().0.clone()
    }

    fn new(current_version: Version, client: Arc<Client>, cx: &mut Context<Self>) -> Self {
        // On windows, executable files cannot be overwritten while they are
        // running, so we must wait to overwrite the application until quitting
        // or restarting. When quitting the app, we spawn the auto update helper
        // to finish the auto update process after Zed exits. When restarting
        // the app after an update, we use `set_restart_path` to run the auto
        // update helper instead of the app, so that it can overwrite the app
        // and then spawn the new binary.
        #[cfg(target_os = "windows")]
        let quit_subscription = Some(cx.on_app_quit(|_, _| finalize_auto_update_on_quit()));
        #[cfg(not(target_os = "windows"))]
        let quit_subscription = None;

        cx.on_app_restart(|this, _| {
            this.quit_subscription.take();
        })
        .detach();

        Self {
            status: AutoUpdateStatus::Idle,
            current_version,
            client,
            pending_poll: None,
            quit_subscription,
            update_check_type: UpdateCheckType::Automatic,
            staged_agent_version: None,
        }
    }

    pub fn start_polling(&self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let poll_interval =
            ReleaseChannel::try_global(cx).map_or(POLL_INTERVAL, |channel| match channel {
                ReleaseChannel::Nightly => NIGHTLY_POLL_INTERVAL,
                _ => POLL_INTERVAL,
            });

        cx.spawn(async move |this, cx| {
            if cfg!(target_os = "windows") {
                use util::ResultExt;

                cleanup_windows()
                    .await
                    .context("failed to cleanup old directories")
                    .log_err();
            }

            loop {
                this.update(cx, |this, cx| this.poll(UpdateCheckType::Automatic, cx))?;
                cx.background_executor().timer(poll_interval).await;
            }
        })
    }

    pub fn update_check_type(&self) -> UpdateCheckType {
        self.update_check_type
    }

    pub fn poll(&mut self, check_type: UpdateCheckType, cx: &mut Context<Self>) {
        if self.pending_poll.is_some() {
            if self.update_check_type == UpdateCheckType::Automatic {
                self.update_check_type = check_type;
                cx.notify();
            }
            return;
        }
        self.update_check_type = check_type;

        cx.notify();

        self.pending_poll = Some(cx.spawn(async move |this, cx| {
            let result = Self::update(this.upgrade()?, cx).await;
            this.update(cx, |this, cx| {
                this.pending_poll = None;
                if let Err(error) = result {
                    let is_missing_dependency =
                        error.downcast_ref::<MissingDependencyError>().is_some();
                    this.status = match check_type {
                        UpdateCheckType::Automatic if is_missing_dependency => {
                            log::warn!("auto-update: {}", error);
                            AutoUpdateStatus::Errored {
                                error: Arc::new(error),
                            }
                        }
                        // Be quiet if the check was automated (e.g. when offline)
                        UpdateCheckType::Automatic => {
                            log::info!("auto-update check failed: error:{:?}", error);
                            AutoUpdateStatus::Idle
                        }
                        UpdateCheckType::Manual => {
                            log::error!("auto-update failed: error:{:?}", error);
                            AutoUpdateStatus::Errored {
                                error: Arc::new(error),
                            }
                        }
                    };

                    cx.notify();
                }
            })
            .ok()
        }));
    }

    pub fn current_version(&self) -> Version {
        self.current_version.clone()
    }

    pub fn status(&self) -> AutoUpdateStatus {
        self.status.clone()
    }

    // VIBEDEV (agent-OTA): the version staged at `<install_dir>/agent.new/` by the
    // last successful poll, if any. `None` means no agent update is pending. A
    // later UI task uses this to surface a "restart to apply the agent update"
    // notice; the swap itself happens at next launch (see
    // `vibedev_account::launcher::apply_staged_agent_update`).
    pub fn staged_agent_version(&self) -> Option<&Version> {
        self.staged_agent_version.as_ref()
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) -> bool {
        if let AutoUpdateStatus::Idle = self.status {
            return false;
        }
        self.status = AutoUpdateStatus::Idle;
        cx.notify();
        true
    }

    // If you are packaging Zed and need to override the place it downloads SSH remotes from,
    // you can override this function. You should also update get_remote_server_release_url to return
    // Ok(None).
    pub async fn download_remote_server_release(
        release_channel: ReleaseChannel,
        version: Option<Version>,
        os: &str,
        arch: &str,
        set_status: impl Fn(&str, &mut AsyncApp) + Send + 'static,
        cx: &mut AsyncApp,
    ) -> Result<PathBuf> {
        let this = cx.update(|cx| {
            cx.default_global::<GlobalAutoUpdate>()
                .0
                .clone()
                .context("auto-update not initialized")
        })?;

        set_status("Fetching remote server release", cx);
        let release = Self::get_release_asset(
            &this,
            release_channel,
            version,
            "vibedev-remote-server",
            os,
            arch,
            cx,
        )
        .await?;

        let servers_dir = paths::remote_servers_dir();
        let channel_dir = servers_dir.join(release_channel.dev_name());
        let platform_dir = channel_dir.join(format!("{}-{}", os, arch));
        let version_path = platform_dir.join(format!("{}.gz", release.version));
        smol::fs::create_dir_all(&platform_dir).await.ok();

        let client = this.read_with(cx, |this, _| this.client.http_client());

        if smol::fs::metadata(&version_path).await.is_err() {
            log::info!(
                "downloading vibedev-remote-server {os} {arch} version {}",
                release.version
            );
            set_status("Downloading remote server", cx);
            download_remote_server_binary(&version_path, release, client).await?;
        }

        if let Err(error) =
            cleanup_remote_server_cache(&platform_dir, &version_path, REMOTE_SERVER_CACHE_LIMIT)
                .await
        {
            log::warn!(
                "Failed to clean up remote server cache in {:?}: {error:#}",
                platform_dir
            );
        }

        Ok(version_path)
    }

    pub async fn get_remote_server_release_url(
        channel: ReleaseChannel,
        version: Option<Version>,
        os: &str,
        arch: &str,
        cx: &mut AsyncApp,
    ) -> Result<Option<String>> {
        let this = cx.update(|cx| {
            cx.default_global::<GlobalAutoUpdate>()
                .0
                .clone()
                .context("auto-update not initialized")
        })?;

        let release =
            Self::get_release_asset(&this, channel, version, "vibedev-remote-server", os, arch, cx)
                .await?;

        Ok(Some(release.url))
    }

    async fn get_release_asset(
        this: &Entity<Self>,
        release_channel: ReleaseChannel,
        version: Option<Version>,
        asset: &str,
        os: &str,
        arch: &str,
        cx: &mut AsyncApp,
    ) -> Result<ReleaseAsset> {
        let client = this.read_with(cx, |this, _| this.client.clone());

        let (system_id, metrics_id, is_staff) = if client.telemetry().metrics_enabled() {
            (
                client.telemetry().system_id(),
                client.telemetry().metrics_id(),
                client.telemetry().is_staff(),
            )
        } else {
            (None, None, None)
        };

        let version = if let Some(mut version) = version {
            version.pre = semver::Prerelease::EMPTY;
            version.build = semver::BuildMetadata::EMPTY;
            version.to_string()
        } else {
            "latest".to_string()
        };
        let http_client = client.http_client();

        // VIBEDEV: the release asset URL must point at VibeDev's own server and
        // must NEVER hit zed.dev / cloud.zed.dev / zed-industries. This is the
        // single shared download endpoint used by BOTH SSH remote-server
        // provisioning (download_remote_server_release /
        // get_remote_server_release_url) AND (future) app self-update.
        //
        // Upstream built this via http_client.build_zed_cloud_url_with_query(),
        // whose base resolves to cloud.zed.dev. We intentionally do NOT use that
        // helper here (it stays untouched because cloud_api_client relies on it
        // for sign-in / llm_tokens). Instead we build the URL directly against
        // VIBEDEV_RELEASES_BASE, mirroring the helper's shape:
        //   {base}{path}?{serde_urlencoded(query)}
        const VIBEDEV_RELEASES_BASE: &str = "https://aitoken.bigopen.cn/vibedev";

        let path = format!("/releases/{}/{}/asset", release_channel.dev_name(), version,);
        let query = serde_urlencoded::to_string(AssetQuery {
            os,
            arch,
            asset,
            metrics_id: metrics_id.as_deref(),
            system_id: system_id.as_deref(),
            is_staff,
        })?;
        let url = format!("{VIBEDEV_RELEASES_BASE}{path}?{query}");

        let mut response = http_client
            .get(url.as_str(), Default::default(), true)
            .await?;
        let mut body = Vec::new();
        response.body_mut().read_to_end(&mut body).await?;

        anyhow::ensure!(
            response.status().is_success(),
            "failed to fetch release: {:?}",
            String::from_utf8_lossy(&body),
        );

        serde_json::from_slice(body.as_slice()).with_context(|| {
            format!(
                "error deserializing release {:?}",
                String::from_utf8_lossy(&body),
            )
        })
    }

    /// VIBEDEV (agent-OTA): the detect + download + STAGE half of agent-OTA. Runs
    /// on the auto-update poll (see `update`). Checks the separately-versioned
    /// `vibedev-agent` release asset against the local `<install_dir>/agent/VERSION`
    /// and, if newer (and nothing is already staged), downloads the tarball,
    /// extracts + TRANSFORMS it into the bundled flattened layout, and publishes it
    /// at `<install_dir>/agent.new/`. A separate already-landed task
    /// (`vibedev_account::launcher::apply_staged_agent_update`) swaps `agent.new/`
    /// → `agent/` on the next launch; a later task shows a notice.
    ///
    /// LAYOUT (single-binary): the `vibedev-agent-<os>-x86_64.tar.gz` asset is now
    /// FLAT — `vibedev-agent[.exe]`, `VERSION`, `vendor/...` at the tarball root —
    /// which is EXACTLY the bundled `agent/` layout the launcher reads. So there is
    /// no dist/→flatten transform anymore: the extracted tree IS the agent, and we
    /// just move it into the stage. (The old split layout shipped `bin/bun[.exe]` +
    /// `dist/cli-bun.js`; the compiled single binary embeds the Bun runtime, so
    /// there is no separate bun runtime + entry script.)
    ///
    /// Non-fatal by contract: every failure path returns `Err` (the caller logs a
    /// warning and continues the app-update poll). On ANY error during
    /// download/extract/transform/verify, ALL partials are removed so a failed
    /// attempt leaves NO `agent.new/` — the next poll retries cleanly. The atomic
    /// `rename(agent.new.partial -> agent.new)` is the ONLY step that makes the
    /// stage "exist", so a half-built stage is never observed by the swap task.
    async fn check_and_stage_agent_update(
        this: &Entity<Self>,
        release_channel: ReleaseChannel,
        client: Arc<HttpClientWithUrl>,
        cx: &mut AsyncApp,
    ) -> Result<()> {
        // a. Resolve the install dir + the agent paths. This duplicates the 2-line
        // `current_exe().parent()` resolution that `InstallerDir::new` (windows)
        // and `cleanup_windows`/`install_release_windows` already use here; the
        // canonical copy is `vibedev_account::launcher::install_dir()`, but we do
        // NOT add a cross-crate dependency on it for this.
        let install_dir = std::env::current_exe()?
            .parent()
            .context("agent-OTA: no parent dir for the running executable")?
            .to_path_buf();
        let agent = install_dir.join("agent");
        let staged = install_dir.join("agent.new");

        // a (cont). If a stage already exists, do NOT re-download. The swap-on-launch
        // task / the notice handle the pending stage; clobbering it would risk
        // tearing a stage another (earlier) poll already published.
        if smol::fs::metadata(&staged).await.is_ok() {
            log::info!(
                "auto-update (agent-OTA): agent update already staged at {}; skipping",
                staged.display()
            );
            return Ok(());
        }

        // b. Fetch the `vibedev-agent` asset. A fetch failure (e.g. the manifest has
        // no `vibedev-agent` entry yet) is NON-fatal: log info and return Ok so the
        // app-update poll continues unaffected.
        let release = match Self::get_release_asset(
            this,
            release_channel,
            None,
            "vibedev-agent",
            OS,
            ARCH,
            cx,
        )
        .await
        {
            Ok(release) => release,
            Err(error) => {
                log::info!(
                    "auto-update (agent-OTA): no vibedev-agent asset available (skipping): {error:#}"
                );
                return Ok(());
            }
        };
        let manifest_version = release
            .version
            .parse::<Version>()
            .with_context(|| format!("agent-OTA: bad manifest agent version {:?}", release.version))?;

        // c. Read the local `<install_dir>/agent/VERSION` (trimmed semver) and
        // decide. The decision is extracted into the pure `should_stage_agent`
        // helper (unit-tested). A missing/unparseable local VERSION counts as
        // "stage it" (a known-good newer build should win over an unknown local).
        let local_version = smol::fs::read_to_string(agent.join("VERSION"))
            .await
            .ok()
            .and_then(|contents| contents.trim().parse::<Version>().ok());
        if !should_stage_agent(&manifest_version, local_version.as_ref()) {
            log::info!(
                "auto-update (agent-OTA): agent up to date (local {}, manifest {manifest_version})",
                local_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            );
            return Ok(());
        }
        log::info!(
            "auto-update (agent-OTA): staging agent update (local {} -> manifest {manifest_version})",
            local_version
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string()),
        );

        // d-h. Download → extract → transform → verify → publish, with a single
        // cleanup site for every partial on any error.
        // Fixed name (not PID-suffixed): the single-instance mutex already prevents
        // concurrent polls, so a PID buys no safety — it only prevents the pre-clean
        // below from reclaiming a tarball orphaned by a crashed prior poll (a dead
        // PID's name would never be swept = an unbounded ~100MB+ leak in the install
        // dir). A fixed name self-heals: the next poll's pre-clean removes it.
        let tarball = install_dir.join("agent.dl.tar.gz");
        let extract_dir = install_dir.join("agent.new.extract");
        let partial = install_dir.join("agent.new.partial");

        let result = Self::download_extract_transform_agent(
            &release,
            client,
            &install_dir,
            &tarball,
            &extract_dir,
            &partial,
            &staged,
        )
        .await;

        // i. Cleanup. The temp tarball and the extraction dir are always removed
        // (whether or not the stage published). On error, the partial stage is also
        // removed so NO `agent.new/` (and no `agent.new.partial/`) is left behind —
        // the next poll retries cleanly.
        let _ = smol::fs::remove_file(&tarball).await;
        let _ = smol::fs::remove_dir_all(&extract_dir).await;
        if result.is_err() {
            let _ = smol::fs::remove_dir_all(&partial).await;
        }

        result?;

        // On success, record the staged version on the entity so a later UI task can
        // show a notice. Decoupled from `status`.
        let staged_version = manifest_version.clone();
        this.update(cx, |this, cx| {
            this.staged_agent_version = Some(staged_version);
            cx.notify();
        });

        log::info!(
            "auto-update (agent-OTA): staged agent {manifest_version} at {}",
            staged.display()
        );
        Ok(())
    }

    /// VIBEDEV (agent-OTA): the download → extract → TRANSFORM → verify → publish
    /// pipeline. Split out from `check_and_stage_agent_update` so the latter owns a
    /// single cleanup site for all partials (any `Err` from here triggers removal of
    /// `agent.new.partial/` + the temp tarball + `agent.new.extract/`). All fs ops
    /// use `smol::fs` (async) so the large `dist/` copy/move never blocks the UI
    /// thread.
    async fn download_extract_transform_agent(
        release: &ReleaseAsset,
        client: Arc<HttpClientWithUrl>,
        install_dir: &Path,
        tarball: &Path,
        extract_dir: &Path,
        partial: &Path,
        staged: &Path,
    ) -> Result<()> {
        // Pre-clean any debris from a previously-interrupted attempt so the renames
        // below don't trip over a leftover partial/extract dir.
        let _ = smol::fs::remove_file(tarball).await;
        let _ = smol::fs::remove_dir_all(extract_dir).await;
        let _ = smol::fs::remove_dir_all(partial).await;

        // d. Download the tarball next to the install dir (same volume as the final
        // stage, so the publish rename is a cheap metadata move).
        download_release(tarball, release.clone(), client)
            .await
            .with_context(|| format!("agent-OTA: downloading agent tarball to {}", tarball.display()))?;

        // e. Extract via `tar -xzf`, mirroring `install_release_linux`. Windows tar
        // gotcha: a GNU `tar` on PATH reads an absolute `C:\...` path as
        // `host:path`. To dodge that on every platform, run `tar` from the install
        // dir (`current_dir`) and pass RELATIVE paths for both the archive and the
        // `-C` target. The extraction dir must exist first (tar `-C` does not create
        // it).
        smol::fs::create_dir_all(extract_dir)
            .await
            .with_context(|| format!("agent-OTA: creating extraction dir {}", extract_dir.display()))?;
        let tarball_rel = tarball
            .file_name()
            .context("agent-OTA: tarball has no file name")?;
        let extract_rel = extract_dir
            .file_name()
            .context("agent-OTA: extraction dir has no file name")?;
        let mut cmd = new_command("tar");
        cmd.current_dir(install_dir)
            .arg("-xzf")
            .arg(tarball_rel)
            .arg("-C")
            .arg(extract_rel);
        let output = cmd
            .output()
            .await
            .context("agent-OTA: failed to run tar")?;
        anyhow::ensure!(
            output.status.success(),
            "agent-OTA: extracting {} failed: {:?}",
            tarball.display(),
            String::from_utf8_lossy(&output.stderr)
        );

        // f. The tarball is FLAT (vibedev-agent[.exe] + VERSION + vendor/ at root),
        // i.e. already the bundled agent/ layout — so the "transform" is just moving
        // the extracted tree into the stage. One rename of the whole extraction dir
        // brings the binary + VERSION + vendor/ across; no dist/→flatten, no
        // separate bun runtime to place.
        smol::fs::rename(extract_dir, partial)
            .await
            .with_context(|| {
                format!("agent-OTA: moving extracted agent -> {}", partial.display())
            })?;

        // g. Verify the staged contract before publishing. These are exactly what
        // the launcher reads from `agent/` after the swap: the compiled agent binary
        // + `VERSION` (the swap-on-launch version gate). Any missing → treat as
        // failure (caller cleans up the partial).
        let agent_bin = if cfg!(target_os = "windows") {
            "vibedev-agent.exe"
        } else {
            "vibedev-agent"
        };
        anyhow::ensure!(
            smol::fs::metadata(partial.join(agent_bin)).await.is_ok(),
            "agent-OTA: staged agent missing {agent_bin}"
        );
        anyhow::ensure!(
            smol::fs::metadata(partial.join("VERSION")).await.is_ok(),
            "agent-OTA: staged agent missing VERSION"
        );

        // h. Atomically publish: rename the verified partial to `agent.new/`. Only
        // after this does the stage "exist" for the swap-on-launch task — a
        // half-built stage is never observed.
        smol::fs::rename(partial, staged)
            .await
            .with_context(|| format!("agent-OTA: publishing stage -> {}", staged.display()))?;

        Ok(())
    }

    async fn update(this: Entity<Self>, cx: &mut AsyncApp) -> Result<()> {
        let (client, installed_version, previous_status, release_channel) =
            this.read_with(cx, |this, cx| {
                (
                    this.client.http_client(),
                    this.current_version.clone(),
                    this.status.clone(),
                    ReleaseChannel::try_global(cx).unwrap_or(ReleaseChannel::Stable),
                )
            });

        Self::check_dependencies()?;

        this.update(cx, |this, cx| {
            this.status = AutoUpdateStatus::Checking;
            log::info!("Auto Update: checking for updates");
            cx.notify();
        });

        // VIBEDEV (agent-OTA): on the same poll, check the separately-versioned
        // bundled backend ("agent") and stage `agent.new/` if a newer one is
        // available. This is DECOUPLED from the app version line: it must run even
        // when the app itself is up to date (the early `return` for a not-newer app
        // is below). Its failure must NEVER abort the app-update flow — log a
        // warning and continue. The swap-on-launch + notice are separate tasks.
        if let Err(error) =
            Self::check_and_stage_agent_update(&this, release_channel, client.clone(), cx).await
        {
            log::warn!("auto-update: agent-OTA check/stage failed (non-fatal): {error:#}");
        }

        let fetched_release_data =
            // VIBEDEV: app self-update asset is "vibedev" (the installer), parallel
            // to the SSH "vibedev-remote-server" asset; the manifest keys on these.
            Self::get_release_asset(&this, release_channel, None, "vibedev", OS, ARCH, cx).await?;
        let fetched_version = fetched_release_data.clone().version;
        let app_commit_sha = Ok(cx.update(|cx| AppCommitSha::try_global(cx).map(|sha| sha.full())));
        let newer_version = Self::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_version,
            previous_status.clone(),
        )?;

        let Some(newer_version) = newer_version else {
            this.update(cx, |this, cx| {
                let status = match previous_status {
                    AutoUpdateStatus::Updated { .. } => previous_status,
                    _ => AutoUpdateStatus::Idle,
                };
                this.status = status;
                cx.notify();
            });
            return Ok(());
        };

        this.update(cx, |this, cx| {
            this.status = AutoUpdateStatus::Downloading {
                version: newer_version.clone(),
            };
            cx.notify();
        });

        let installer_dir = InstallerDir::new()
            .await
            .context("Failed to create installer dir")?;
        let target_path = Self::target_path(&installer_dir).await?;
        download_release(&target_path, fetched_release_data, client)
            .await
            .with_context(|| format!("Failed to download update to {}", target_path.display()))?;

        this.update(cx, |this, cx| {
            this.status = AutoUpdateStatus::Installing {
                version: newer_version.clone(),
            };
            cx.notify();
        });

        let new_binary_path = Self::install_release(installer_dir, &target_path, cx)
            .await
            .with_context(|| format!("Failed to install update at: {}", target_path.display()))?;
        if let Some(new_binary_path) = new_binary_path {
            cx.update(|cx| cx.set_restart_path(new_binary_path));
        }

        this.update(cx, |this, cx| {
            this.set_should_show_update_notification(true, cx)
                .detach_and_log_err(cx);
            this.status = AutoUpdateStatus::Updated {
                version: newer_version,
            };
            cx.notify();
        });
        Ok(())
    }

    fn check_if_fetched_version_is_newer(
        release_channel: ReleaseChannel,
        app_commit_sha: Result<Option<String>>,
        installed_version: Version,
        fetched_version: String,
        status: AutoUpdateStatus,
    ) -> Result<Option<VersionCheckType>> {
        let parsed_fetched_version = fetched_version.parse::<Version>();

        if let AutoUpdateStatus::Updated { version, .. } = status {
            match version {
                VersionCheckType::Sha(cached_version) => {
                    let should_download =
                        parsed_fetched_version.as_ref().ok().is_none_or(|version| {
                            version.build.as_str().rsplit('.').next()
                                != Some(&cached_version.full())
                        });
                    let newer_version = should_download
                        .then(|| VersionCheckType::Sha(AppCommitSha::new(fetched_version)));
                    return Ok(newer_version);
                }
                VersionCheckType::Semantic(cached_version) => {
                    return Self::check_if_fetched_version_is_newer_non_nightly(
                        cached_version,
                        parsed_fetched_version?,
                    );
                }
            }
        }

        match release_channel {
            ReleaseChannel::Nightly => {
                let should_download = app_commit_sha
                    .ok()
                    .flatten()
                    .map(|sha| {
                        parsed_fetched_version.as_ref().ok().is_none_or(|version| {
                            version.build.as_str().rsplit('.').next() != Some(&sha)
                        })
                    })
                    .unwrap_or(true);
                let newer_version = should_download
                    .then(|| VersionCheckType::Sha(AppCommitSha::new(fetched_version)));
                Ok(newer_version)
            }
            _ => Self::check_if_fetched_version_is_newer_non_nightly(
                installed_version,
                parsed_fetched_version?,
            ),
        }
    }

    fn check_dependencies() -> Result<()> {
        #[cfg(target_os = "linux")]
        if which::which("rsync").is_err() {
            let install_hint = linux_rsync_install_hint();
            return Err(MissingDependencyError(format!(
                "rsync is required for auto-updates but is not installed. {install_hint}"
            ))
            .into());
        }

        #[cfg(target_os = "macos")]
        anyhow::ensure!(
            which::which("rsync").is_ok(),
            "Could not auto-update because the required rsync utility was not found."
        );

        Ok(())
    }

    async fn target_path(installer_dir: &InstallerDir) -> Result<PathBuf> {
        let filename = match OS {
            "macos" => anyhow::Ok("Zed.dmg"),
            "linux" => Ok("zed.tar.gz"),
            "windows" => Ok("Zed.exe"),
            unsupported_os => anyhow::bail!("not supported: {unsupported_os}"),
        }?;

        Ok(installer_dir.path().join(filename))
    }

    async fn install_release(
        installer_dir: InstallerDir,
        target_path: &Path,
        cx: &AsyncApp,
    ) -> Result<Option<PathBuf>> {
        #[cfg(test)]
        if let Some(test_install) =
            cx.try_read_global::<tests::InstallOverride, _>(|g, _| g.0.clone())
        {
            return test_install(target_path, cx);
        }
        match OS {
            "macos" => install_release_macos(&installer_dir, target_path, cx).await,
            "linux" => install_release_linux(&installer_dir, target_path, cx).await,
            "windows" => install_release_windows(target_path).await,
            unsupported_os => anyhow::bail!("not supported: {unsupported_os}"),
        }
    }

    fn check_if_fetched_version_is_newer_non_nightly(
        mut installed_version: Version,
        fetched_version: Version,
    ) -> Result<Option<VersionCheckType>> {
        // For non-nightly releases, ignore build and pre-release fields as they're not provided by our endpoints right now.
        installed_version.pre = semver::Prerelease::EMPTY;
        installed_version.build = semver::BuildMetadata::EMPTY;
        let should_download = fetched_version > installed_version;
        let newer_version = should_download.then(|| VersionCheckType::Semantic(fetched_version));
        Ok(newer_version)
    }

    pub fn set_should_show_update_notification(
        &self,
        should_show: bool,
        cx: &App,
    ) -> Task<Result<()>> {
        let kvp = KeyValueStore::global(cx);
        cx.background_spawn(async move {
            if should_show {
                kvp.write_kvp(
                    SHOULD_SHOW_UPDATE_NOTIFICATION_KEY.to_string(),
                    "".to_string(),
                )
                .await?;
            } else {
                kvp.delete_kvp(SHOULD_SHOW_UPDATE_NOTIFICATION_KEY.to_string())
                    .await?;
            }
            Ok(())
        })
    }

    pub fn should_show_update_notification(&self, cx: &App) -> Task<Result<bool>> {
        let kvp = KeyValueStore::global(cx);
        cx.background_spawn(async move {
            Ok(kvp.read_kvp(SHOULD_SHOW_UPDATE_NOTIFICATION_KEY)?.is_some())
        })
    }
}

async fn download_remote_server_binary(
    target_path: &PathBuf,
    release: ReleaseAsset,
    client: Arc<HttpClientWithUrl>,
) -> Result<()> {
    let temp = tempfile::Builder::new().tempfile_in(remote_servers_dir())?;
    let mut temp_file = File::create(&temp).await?;

    let mut response = client.get(&release.url, Default::default(), true).await?;
    anyhow::ensure!(
        response.status().is_success(),
        "failed to download remote server release: {:?}",
        response.status()
    );
    smol::io::copy(response.body_mut(), &mut temp_file).await?;
    smol::fs::rename(&temp, &target_path).await?;

    Ok(())
}

async fn cleanup_remote_server_cache(
    platform_dir: &Path,
    keep_path: &Path,
    limit: usize,
) -> Result<()> {
    if limit == 0 {
        return Ok(());
    }

    let mut entries = smol::fs::read_dir(platform_dir).await?;
    let now = SystemTime::now();
    let mut candidates = Vec::new();

    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("gz")) {
            continue;
        }

        let mtime = if path == keep_path {
            now
        } else {
            smol::fs::metadata(&path)
                .await
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        };

        candidates.push((path, mtime));
    }

    if candidates.len() <= limit {
        return Ok(());
    }

    candidates.sort_by(|(path_a, time_a), (path_b, time_b)| {
        time_b.cmp(time_a).then_with(|| path_a.cmp(path_b))
    });

    for (index, (path, _)) in candidates.into_iter().enumerate() {
        if index < limit || path == keep_path {
            continue;
        }

        if let Err(error) = smol::fs::remove_file(&path).await {
            log::warn!(
                "Failed to remove old remote server archive {:?}: {}",
                path,
                error
            );
        }
    }

    Ok(())
}

/// VIBEDEV (agent-OTA): the pure version-comparison decision for staging a new
/// agent. Stage iff the local agent VERSION is absent/unparseable (`None`) OR the
/// manifest's agent version is strictly greater than the local one. Extracted as a
/// free, side-effect-free fn so it is unit-testable without touching the network or
/// the filesystem (see the `should_stage_agent_*` tests). Mirrors the version gate
/// in `vibedev_account::launcher::apply_staged_agent_update`, but on the DOWNLOAD
/// side: a missing local version is treated as "older" so a known-good newer build
/// still wins.
fn should_stage_agent(manifest: &Version, local: Option<&Version>) -> bool {
    match local {
        Some(local) => manifest > local,
        None => true,
    }
}

async fn download_release(
    target_path: &Path,
    release: ReleaseAsset,
    client: Arc<HttpClientWithUrl>,
) -> Result<()> {
    let mut target_file = File::create(&target_path).await?;

    let mut response = client.get(&release.url, Default::default(), true).await?;
    anyhow::ensure!(
        response.status().is_success(),
        "failed to download update: {:?}",
        response.status()
    );
    smol::io::copy(response.body_mut(), &mut target_file).await?;
    log::info!("downloaded update. path:{:?}", target_path);

    Ok(())
}

async fn install_release_linux(
    temp_dir: &InstallerDir,
    downloaded_tar_gz: &Path,
    cx: &AsyncApp,
) -> Result<Option<PathBuf>> {
    let channel = cx.update(|cx| ReleaseChannel::global(cx).dev_name());
    let home_dir = PathBuf::from(env::var("HOME").context("no HOME env var set")?);
    let running_app_path = cx.update(|cx| cx.app_path())?;

    let extracted = temp_dir.path().join("zed");
    fs::create_dir_all(&extracted)
        .await
        .context("failed to create directory into which to extract update")?;

    let mut cmd = new_command("tar");
    cmd.arg("-xzf")
        .arg(&downloaded_tar_gz)
        .arg("-C")
        .arg(&extracted);
    let output = cmd
        .output()
        .await
        .with_context(|| "failed to extract: {cmd}")?;

    anyhow::ensure!(
        output.status.success(),
        "failed to extract {:?} to {:?}: {:?}",
        downloaded_tar_gz,
        extracted,
        String::from_utf8_lossy(&output.stderr)
    );

    let suffix = if channel != "stable" {
        format!("-{}", channel)
    } else {
        String::default()
    };
    let app_folder_name = format!("zed{}.app", suffix);

    let from = extracted.join(&app_folder_name);
    let mut to = home_dir.join(".local");

    let expected_suffix = format!("{}/libexec/zed-editor", app_folder_name);

    if let Some(prefix) = running_app_path
        .to_str()
        .and_then(|str| str.strip_suffix(&expected_suffix))
    {
        to = PathBuf::from(prefix);
    }

    let mut cmd = new_command("rsync");
    cmd.args(["-av", "--delete"]).arg(&from).arg(&to);
    let output = cmd
        .output()
        .await
        .with_context(|| "failed to rsync: {cmd}")?;

    anyhow::ensure!(
        output.status.success(),
        "failed to copy Zed update from {:?} to {:?}: {:?}",
        from,
        to,
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(Some(to.join(expected_suffix)))
}

async fn install_release_macos(
    temp_dir: &InstallerDir,
    downloaded_dmg: &Path,
    cx: &AsyncApp,
) -> Result<Option<PathBuf>> {
    let running_app_path = cx.update(|cx| cx.app_path())?;
    let running_app_filename = running_app_path
        .file_name()
        .with_context(|| format!("invalid running app path {running_app_path:?}"))?;

    // VIBEDEV: `-mountroot` below mounts the dmg volume at <temp>/<volume-name>. Our dmg's
    // volume name is "VibeDev" (script/bundle-mac: `hdiutil create -volname VibeDev`), NOT
    // upstream's "Zed". This name MUST track that -volname, or the mounted app path is wrong
    // and auto-update install fails (rsync: "No such file or directory"). Re-apply on sync.
    let mount_path = temp_dir.path().join("VibeDev");
    let mut mounted_app_path: OsString = mount_path.join(running_app_filename).into();

    mounted_app_path.push("/");
    let mut cmd = new_command("hdiutil");
    cmd.args(["attach", "-nobrowse"])
        .arg(&downloaded_dmg)
        .arg("-mountroot")
        .arg(temp_dir.path());
    let output = cmd
        .output()
        .await
        .with_context(|| "failed to mount: {cmd}")?;

    anyhow::ensure!(
        output.status.success(),
        "failed to mount: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Create an MacOsUnmounter that will be dropped (and thus unmount the disk) when this function exits
    let _unmounter = MacOsUnmounter {
        mount_path: mount_path.clone(),
        background_executor: cx.background_executor(),
    };

    let mut cmd = new_command("rsync");
    cmd.args(["-av", "--delete", "--exclude", "Icon?"])
        .arg(&mounted_app_path)
        .arg(&running_app_path);
    let output = cmd
        .output()
        .await
        .with_context(|| "failed to rsync: {cmd}")?;

    anyhow::ensure!(
        output.status.success(),
        "failed to copy app: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(None)
}

async fn cleanup_windows() -> Result<()> {
    let parent = std::env::current_exe()?
        .parent()
        .context("No parent dir for Zed.exe")?
        .to_owned();

    // keep in sync with crates/auto_update_helper/src/updater.rs
    _ = smol::fs::remove_dir(parent.join("updates")).await;
    _ = smol::fs::remove_dir(parent.join("install")).await;
    _ = smol::fs::remove_dir(parent.join("old")).await;

    Ok(())
}

async fn install_release_windows(downloaded_installer: &Path) -> Result<Option<PathBuf>> {
    // VIBEDEV: strip the Mark-of-the-Web (`:Zone.Identifier` alternate data stream
    // Windows attaches to internet-downloaded files) BEFORE running the installer.
    // VibeDev's installer is currently unsigned (free auto-update path); removing
    // the MOTW keeps Defender/SmartScreen from treating this silent programmatic
    // install as a suspicious "downloaded" exe. Best-effort: no MOTW => harmless
    // NotFound, ignored.
    let _ = smol::fs::remove_file(format!(
        "{}:Zone.Identifier",
        downloaded_installer.display()
    ))
    .await;
    let mut cmd = new_command(downloaded_installer);
    cmd.arg("/verysilent")
        .arg("/update=true")
        .arg("/MERGETASKS=!desktopicon");
    let output = cmd.output().await?;
    anyhow::ensure!(
        output.status.success(),
        "failed to start installer: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    // We return the path to the update helper program, because it will
    // perform the final steps of the update process, copying the new binary,
    // deleting the old one, and launching the new binary.
    let helper_path = std::env::current_exe()?
        .parent()
        .context("No parent dir for Zed.exe")?
        .join("tools")
        .join("auto_update_helper.exe");
    Ok(Some(helper_path))
}

pub async fn finalize_auto_update_on_quit() {
    let Some(installer_path) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("updates")))
    else {
        return;
    };

    // The installer will create a flag file after it finishes updating
    let flag_file = installer_path.join("versions.txt");
    if flag_file.exists()
        && let Some(helper) = installer_path
            .parent()
            .map(|p| p.join("tools").join("auto_update_helper.exe"))
    {
        let mut command = util::command::new_command(helper);
        command.arg("--launch");
        command.arg("false");
        if let Ok(mut cmd) = command.spawn() {
            _ = cmd.status().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use client::Client;
    use clock::FakeSystemClock;
    use futures::channel::oneshot;
    use gpui::TestAppContext;
    use http_client::{FakeHttpClient, Response};
    use settings::default_settings;
    use std::{
        rc::Rc,
        sync::{
            Arc,
            atomic::{self, AtomicBool},
        },
    };
    use tempfile::tempdir;

    #[ctor::ctor(unsafe)]
    fn init_logger() {
        zlog::init_test();
    }

    use super::*;

    pub(super) struct InstallOverride(pub Rc<dyn Fn(&Path, &AsyncApp) -> Result<Option<PathBuf>>>);
    impl Global for InstallOverride {}

    #[gpui::test]
    fn test_auto_update_defaults_to_true(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut store = SettingsStore::new(cx, &settings::default_settings());
            store
                .set_default_settings(&default_settings(), cx)
                .expect("Unable to set default settings");
            store
                .set_user_settings("{}", cx)
                .expect("Unable to set user settings");
            cx.set_global(store);
            assert!(AutoUpdateSetting::get_global(cx).0);
        });
    }

    #[gpui::test]
    async fn test_auto_update_downloads(cx: &mut TestAppContext) {
        cx.background_executor.allow_parking();
        zlog::init_test();
        let release_available = Arc::new(AtomicBool::new(false));

        let (dmg_tx, dmg_rx) = oneshot::channel::<String>();

        cx.update(|cx| {
            settings::init(cx);

            let current_version = semver::Version::new(0, 100, 0);
            release_channel::init_test(current_version, ReleaseChannel::Stable, cx);

            let clock = Arc::new(FakeSystemClock::new());
            let release_available = Arc::clone(&release_available);
            let dmg_rx = Arc::new(parking_lot::Mutex::new(Some(dmg_rx)));
            let fake_client_http = FakeHttpClient::create(move |req| {
                let release_available = release_available.load(atomic::Ordering::Relaxed);
                let dmg_rx = dmg_rx.clone();
                async move {
                if req.uri().path() == "/releases/stable/latest/asset" {
                    if release_available {
                        return Ok(Response::builder().status(200).body(
                            r#"{"version":"0.100.1","url":"https://test.example/new-download"}"#.into()
                        ).unwrap());
                    } else {
                        return Ok(Response::builder().status(200).body(
                            r#"{"version":"0.100.0","url":"https://test.example/old-download"}"#.into()
                        ).unwrap());
                    }
                } else if req.uri().path() == "/new-download" {
                    return Ok(Response::builder().status(200).body({
                        let dmg_rx = dmg_rx.lock().take().unwrap();
                        dmg_rx.await.unwrap().into()
                    }).unwrap());
                }
                Ok(Response::builder().status(404).body("".into()).unwrap())
                }
            });
            let client = Client::new(clock, fake_client_http, cx);
            crate::init(client, cx);
        });

        let auto_updater = cx.update(|cx| AutoUpdater::get(cx).expect("auto updater should exist"));

        cx.background_executor.run_until_parked();

        auto_updater.read_with(cx, |updater, _| {
            assert_eq!(updater.status(), AutoUpdateStatus::Idle);
            assert_eq!(updater.current_version(), semver::Version::new(0, 100, 0));
        });

        release_available.store(true, atomic::Ordering::SeqCst);
        cx.background_executor.advance_clock(POLL_INTERVAL);
        cx.background_executor.run_until_parked();

        loop {
            cx.background_executor.timer(Duration::from_millis(0)).await;
            cx.run_until_parked();
            let status = auto_updater.read_with(cx, |updater, _| updater.status());
            if !matches!(status, AutoUpdateStatus::Idle) {
                break;
            }
        }
        let status = auto_updater.read_with(cx, |updater, _| updater.status());
        assert_eq!(
            status,
            AutoUpdateStatus::Downloading {
                version: VersionCheckType::Semantic(semver::Version::new(0, 100, 1))
            }
        );

        dmg_tx.send("<fake-zed-update>".to_owned()).unwrap();

        let tmp_dir = Arc::new(tempdir().unwrap());

        cx.update(|cx| {
            let tmp_dir = tmp_dir.clone();
            cx.set_global(InstallOverride(Rc::new(move |target_path, _cx| {
                let tmp_dir = tmp_dir.clone();
                let dest_path = tmp_dir.path().join("zed");
                std::fs::copy(&target_path, &dest_path)?;
                Ok(Some(dest_path))
            })));
        });

        loop {
            cx.background_executor.timer(Duration::from_millis(0)).await;
            cx.run_until_parked();
            let status = auto_updater.read_with(cx, |updater, _| updater.status());
            if !matches!(status, AutoUpdateStatus::Downloading { .. }) {
                break;
            }
        }
        let status = auto_updater.read_with(cx, |updater, _| updater.status());
        assert_eq!(
            status,
            AutoUpdateStatus::Updated {
                version: VersionCheckType::Semantic(semver::Version::new(0, 100, 1))
            }
        );
        let will_restart = cx.expect_restart();
        cx.update(|cx| cx.restart());
        let path = will_restart.await.unwrap().unwrap();
        assert_eq!(path, tmp_dir.path().join("zed"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "<fake-zed-update>");
    }

    #[test]
    fn test_stable_does_not_update_when_fetched_version_is_not_higher() {
        let release_channel = ReleaseChannel::Stable;
        let app_commit_sha = Ok(Some("a".to_string()));
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Idle;
        let fetched_version = semver::Version::new(1, 0, 0);

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_version.to_string(),
            status,
        );

        assert_eq!(newer_version.unwrap(), None);
    }

    #[test]
    fn test_stable_does_update_when_fetched_version_is_higher() {
        let release_channel = ReleaseChannel::Stable;
        let app_commit_sha = Ok(Some("a".to_string()));
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Idle;
        let fetched_version = semver::Version::new(1, 0, 1);

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_version.to_string(),
            status,
        );

        assert_eq!(
            newer_version.unwrap(),
            Some(VersionCheckType::Semantic(fetched_version))
        );
    }

    #[test]
    fn test_stable_does_not_update_when_fetched_version_is_not_higher_than_cached() {
        let release_channel = ReleaseChannel::Stable;
        let app_commit_sha = Ok(Some("a".to_string()));
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Updated {
            version: VersionCheckType::Semantic(semver::Version::new(1, 0, 1)),
        };
        let fetched_version = semver::Version::new(1, 0, 1);

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_version.to_string(),
            status,
        );

        assert_eq!(newer_version.unwrap(), None);
    }

    #[test]
    fn test_stable_does_update_when_fetched_version_is_higher_than_cached() {
        let release_channel = ReleaseChannel::Stable;
        let app_commit_sha = Ok(Some("a".to_string()));
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Updated {
            version: VersionCheckType::Semantic(semver::Version::new(1, 0, 1)),
        };
        let fetched_version = semver::Version::new(1, 0, 2);

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_version.to_string(),
            status,
        );

        assert_eq!(
            newer_version.unwrap(),
            Some(VersionCheckType::Semantic(fetched_version))
        );
    }

    #[test]
    fn test_nightly_does_not_update_when_fetched_sha_is_same() {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(Some("a".to_string()));
        let mut installed_version = semver::Version::new(1, 0, 0);
        installed_version.build = semver::BuildMetadata::new("a").unwrap();
        let status = AutoUpdateStatus::Idle;
        let fetched_sha = "1.0.0+a".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha,
            status,
        );

        assert_eq!(newer_version.unwrap(), None);
    }

    #[test]
    fn test_nightly_does_update_when_fetched_sha_is_not_same() {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(Some("a".to_string()));
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Idle;
        let fetched_sha = "b".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha.clone(),
            status,
        );

        assert_eq!(
            newer_version.unwrap(),
            Some(VersionCheckType::Sha(AppCommitSha::new(fetched_sha)))
        );
    }

    #[test]
    fn test_nightly_does_not_update_when_fetched_version_is_same_as_cached() {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(Some("a".to_string()));
        let mut installed_version = semver::Version::new(1, 0, 0);
        installed_version.build = semver::BuildMetadata::new("a").unwrap();
        let status = AutoUpdateStatus::Updated {
            version: VersionCheckType::Sha(AppCommitSha::new("b".to_string())),
        };
        let fetched_sha = "1.0.0+b".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha,
            status,
        );

        assert_eq!(newer_version.unwrap(), None);
    }

    #[test]
    fn test_nightly_does_update_when_fetched_sha_is_not_same_as_cached() {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(Some("a".to_string()));
        let mut installed_version = semver::Version::new(1, 0, 0);
        installed_version.build = semver::BuildMetadata::new("a").unwrap();
        let status = AutoUpdateStatus::Updated {
            version: VersionCheckType::Sha(AppCommitSha::new("b".to_string())),
        };
        let fetched_sha = "1.0.0+c".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha.clone(),
            status,
        );

        assert_eq!(
            newer_version.unwrap(),
            Some(VersionCheckType::Sha(AppCommitSha::new(fetched_sha)))
        );
    }

    #[test]
    fn test_nightly_does_update_when_installed_versions_sha_cannot_be_retrieved() {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(None);
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Idle;
        let fetched_sha = "a".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha.clone(),
            status,
        );

        assert_eq!(
            newer_version.unwrap(),
            Some(VersionCheckType::Sha(AppCommitSha::new(fetched_sha)))
        );
    }

    #[test]
    fn test_nightly_does_not_update_when_cached_update_is_same_as_fetched_and_installed_versions_sha_cannot_be_retrieved()
     {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(None);
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Updated {
            version: VersionCheckType::Sha(AppCommitSha::new("b".to_string())),
        };
        let fetched_sha = "1.0.0+b".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha,
            status,
        );

        assert_eq!(newer_version.unwrap(), None);
    }

    #[test]
    fn test_nightly_does_update_when_cached_update_is_not_same_as_fetched_and_installed_versions_sha_cannot_be_retrieved()
     {
        let release_channel = ReleaseChannel::Nightly;
        let app_commit_sha = Ok(None);
        let installed_version = semver::Version::new(1, 0, 0);
        let status = AutoUpdateStatus::Updated {
            version: VersionCheckType::Sha(AppCommitSha::new("b".to_string())),
        };
        let fetched_sha = "c".to_string();

        let newer_version = AutoUpdater::check_if_fetched_version_is_newer(
            release_channel,
            app_commit_sha,
            installed_version,
            fetched_sha.clone(),
            status,
        );

        assert_eq!(
            newer_version.unwrap(),
            Some(VersionCheckType::Sha(AppCommitSha::new(fetched_sha)))
        );
    }

    // VIBEDEV (agent-OTA): tests for the pure staging decision. The download/extract
    // pipeline (network + tar) is deliberately NOT unit-tested here — it's verified
    // in a later joint build; the decision helper carries the logic worth asserting.

    #[test]
    fn test_should_stage_agent_when_manifest_is_newer() {
        // Manifest strictly newer than local → stage it.
        let manifest = semver::Version::new(1, 0, 1);
        let local = semver::Version::new(1, 0, 0);
        assert!(should_stage_agent(&manifest, Some(&local)));
    }

    #[test]
    fn test_should_not_stage_agent_when_manifest_is_older() {
        // Manifest strictly older than local → do not stage (no downgrade).
        let manifest = semver::Version::new(1, 0, 0);
        let local = semver::Version::new(1, 0, 1);
        assert!(!should_stage_agent(&manifest, Some(&local)));
    }

    #[test]
    fn test_should_not_stage_agent_when_versions_are_equal() {
        // Manifest equal to local → do not stage (already up to date).
        let manifest = semver::Version::new(1, 2, 3);
        let local = semver::Version::new(1, 2, 3);
        assert!(!should_stage_agent(&manifest, Some(&local)));
    }

    #[test]
    fn test_should_stage_agent_when_local_is_missing() {
        // No local VERSION (file absent) → `None` → stage the known-good build.
        let manifest = semver::Version::new(1, 0, 0);
        assert!(should_stage_agent(&manifest, None));
    }

    #[test]
    fn test_should_stage_agent_when_local_is_unparseable() {
        // An unparseable local VERSION collapses to `None` at the read boundary
        // (see `check_and_stage_agent_update`), so it maps to the same "stage it"
        // decision as a missing file. Assert the `None` branch holds for that case.
        let manifest = semver::Version::new(1, 0, 0);
        let unparseable_local = "not-a-semver".parse::<semver::Version>().ok();
        assert!(unparseable_local.is_none());
        assert!(should_stage_agent(&manifest, unparseable_local.as_ref()));
    }
}
