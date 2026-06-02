//! VIBEDEV: persist the user's last per-session ACP picks so a new session
//! re-applies them instead of falling back to the agent's defaults.
//!
//! claude-code-best's `src/services/acp/agent.ts` (~line 710) intentionally
//! resets `thinkingLevel` / `contextOverride` to `undefined` on every
//! `newSession`, with the comment "a fresh session starts at the model
//! defaults". The IDE-side selectors are the only state outside the agent
//! that survives across new sessions, so it's the IDE's job to remember the
//! user's last choice and seed each new session from it.
//!
//! Layout: `~/.vibedev/last-session-prefs.json`, atomic write (write to
//! `<file>.tmp`, fsync, rename). Locked-down perms aren't necessary — the
//! file contains no secrets, just three short strings — but the atomic write
//! protects against torn writes if the IDE crashes mid-save. Home dir lookup
//! mirrors the `vibedev_home()` pattern in `vibedev_account.rs`
//! (`VIBEDEV_HOME` env override → `dirs::home_dir().join(".vibedev")`), so a
//! dev pointing the sidecar at a scratch dir picks up scratch prefs too.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{io::Write as _, path::PathBuf};

/// The three IDE-controlled selectors we want to survive a new session.
///
/// Each field is `Option<String>`: `None` means "leave the agent's default in
/// place" (so a freshly installed VibeDev with no prefs file behaves identically
/// to upstream Zed). The strings are the same opaque ids the ACP wire format
/// uses — `acp::ModelId` for `model`, `acp::SessionConfigValueId` for the
/// `thinking` / `context` selects — so the load path can hand them straight to
/// `SetSessionModelRequest` / `SetSessionConfigOptionRequest` without parsing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl Prefs {
    /// `true` when every field is `None`, i.e. nothing to apply.
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.thinking.is_none() && self.context.is_none()
    }
}

/// `~/.vibedev/last-session-prefs.json`, with the same `VIBEDEV_HOME` override
/// as the sidecar handshake so a dev's scratch dir gets its own prefs file.
fn prefs_path() -> Result<PathBuf> {
    let home = if let Some(dir) = std::env::var_os("VIBEDEV_HOME") {
        PathBuf::from(dir)
    } else {
        dirs::home_dir().context("no home dir")?.join(".vibedev")
    };
    Ok(home.join("last-session-prefs.json"))
}

/// Read the persisted prefs. Returns `None` if the file is missing (first run
/// on a fresh install), unreadable, or malformed — every failure mode is a
/// soft fall-through to "use the agent's defaults", because losing the
/// preference is far better than refusing to spawn a session.
pub fn load() -> Option<Prefs> {
    let path = prefs_path().ok()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<Prefs>(&raw) {
        Ok(prefs) if !prefs.is_empty() => Some(prefs),
        Ok(_) => None,
        Err(err) => {
            log::warn!(
                "vibedev session prefs at {} could not be parsed: {err:#}; ignoring",
                path.display()
            );
            None
        }
    }
}

/// Atomically persist `prefs`. Writes `<file>.tmp` first, then renames over
/// the target so a crash mid-write can't leave behind a half-written JSON
/// blob. Creates the parent dir on demand (first run, the user might not
/// have logged into the sidecar yet, so `~/.vibedev/` may not exist).
///
/// Errors are logged but not propagated to the caller — the read path is the
/// one with consequences (a bad prefs file would block new sessions), and a
/// failed save just means the *next* new session re-applies whatever the
/// last successful save persisted.
pub fn save(prefs: &Prefs) {
    if let Err(err) = save_inner(prefs) {
        log::warn!("vibedev session prefs not persisted: {err:#}");
    }
}

fn save_inner(prefs: &Prefs) -> Result<()> {
    let path = prefs_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(prefs).context("serialize prefs")?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(&json)
            .with_context(|| format!("write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("fsync {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Convenience: read-modify-write. Loads current prefs (or default), applies
/// `mutate`, and writes back. Used by the IDE-side set_model /
/// set_config_option call sites so each field can be updated independently
/// without clobbering the others. Save errors are logged but not propagated.
pub fn update(mutate: impl FnOnce(&mut Prefs)) {
    let mut prefs = load().unwrap_or_default();
    let before = prefs.clone();
    mutate(&mut prefs);
    if prefs != before {
        save(&prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// VIBEDEV: `VIBEDEV_HOME` is a process-wide env var, so concurrent tests
    /// that flip it race each other. Serialize the whole module's tests behind
    /// one mutex rather than splitting them and chasing a `--test-threads=1`
    /// flag every time.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    /// Round-trip through the on-disk JSON + assert the all-None case is
    /// treated as "no preference". One combined test so they share the env
    /// guard without dancing around `std::env::set_var` thread-safety.
    #[test]
    fn round_trip_and_empty() {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: ENV_GUARD serializes test threads that touch VIBEDEV_HOME.
        // The 2024-edition `set_var` unsafety is about cross-thread reads, and
        // we hold the only writer for the duration of the test.
        unsafe { std::env::set_var("VIBEDEV_HOME", tmp.path()) };

        assert!(load().is_none(), "no file yet");

        // All-None prefs are treated as "no preference": the file gets created
        // but load() returns None so the new-session path falls through to the
        // agent's defaults instead of firing a no-op SetSessionModel(None).
        save(&Prefs::default());
        assert!(load().is_none(), "empty file == no prefs");

        let prefs = Prefs {
            model: Some("claude-sonnet-4-5".into()),
            thinking: Some("medium".into()),
            context: Some("200000".into()),
        };
        save(&prefs);

        let loaded = load().expect("loaded");
        assert_eq!(loaded, prefs);

        // partial update via update() should not clobber other fields
        update(|p| p.thinking = Some("high".into()));
        let updated = load().expect("loaded");
        assert_eq!(updated.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(updated.thinking.as_deref(), Some("high"));
        assert_eq!(updated.context.as_deref(), Some("200000"));

        unsafe { std::env::remove_var("VIBEDEV_HOME") };
    }
}
