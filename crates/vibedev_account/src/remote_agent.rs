//! VIBEDEV (V2-PLAN-8): remote (SSH) ACP agent support.
//!
//! When a user opens a REMOTE ssh project, Zed launches the ACP agent ON the
//! remote host, but the command is resolved from the CLIENT's settings — for
//! VibeDev that is a local (Windows) path like
//! `C:/Users/.../agent/vibedev-agent.exe --acp`, which does not exist on the
//! remote Linux box, so the remote `env` fails with
//! `env: '...': No such file or directory` and the agent exits 1.
//!
//! Fix: when the project is remote, replace the launch command with a small
//! POSIX shell program that (1) self-provisions a self-contained agent bundle
//! into `$HOME/.vibedev/agent` on first use, then (2) execs it. The bundle is
//! FLAT — `vibedev-agent` + `VERSION` + `vendor/ripgrep/x64-linux/rg` — hosted on
//! the VibeDev release server and verified on real hardware (see
//! `v2/V2-PLAN-8-remote-dev.md`).
//!
//! Why a `sh -c` program instead of pointing the command straight at
//! `<dir>/bin/bun`:
//!   - The SSH transport (`build_command_posix`) shell-QUOTES the program, so a
//!     leading `~`/`$HOME` in the program path would NOT expand. Wrapping the
//!     real work in `sh -c "…"` lets `$HOME` expand inside the remote shell, so
//!     we never need to resolve the absolute remote home on the client.
//!   - Provisioning (download + extract + `chmod +x`) and the exec live in one
//!     atomic command — no extra client⇄remote round-trips, no new transport
//!     plumbing. `chmod +x` is mandatory: a Windows-assembled tarball loses the
//!     executable bit (verified), so the bundle ships 0644 and we restore it
//!     here.
//!
//! All provisioning output is redirected to stderr (`1>&2`) so it never
//! pollutes stdout, which carries the ACP JSON-RPC stream.

/// Remote bundle download URL. v1 supports linux-x86_64 only (the single
/// platform we build + host today); P3 generalizes to `{os}-{arch}` resolved
/// from the release manifest. Base host matches `auto_update`'s
/// `VIBEDEV_RELEASES_BASE` / `vibedev_account::update_check`.
pub const AGENT_BUNDLE_URL: &str =
    "https://aitoken.bigopen.cn/downloads/vibedev-agent-linux-x86_64.tar.gz";

/// `$HOME`-relative bundle directory on the remote (expanded by the remote
/// shell inside the `sh -c` program).
pub const REMOTE_AGENT_DIR: &str = "$HOME/.vibedev/agent";

/// VIBEDEV (agent-OTA): the local app's bundled agent version — the version we
/// expect the remote to be running, so a remote that was provisioned by an OLDER
/// client (or has a missing/absent `VERSION`) gets re-provisioned and kept
/// in lockstep with the local app. Reads `<install_dir>/agent/VERSION`, trimmed;
/// `None` if the install dir / file can't be resolved, is unreadable, or is
/// empty (e.g. dev mode with no bundled agent next to the exe). A tiny file read
/// at provision time on the CLIENT — sync `std::fs` is fine and matches the
/// other `install_dir()`-relative readers in `launcher.rs`.
pub fn bundled_agent_version() -> Option<String> {
    let path = crate::launcher::install_dir()?.join("agent").join("VERSION");
    let trimmed = std::fs::read_to_string(path).ok()?.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// VIBEDEV (agent-OTA): whether a version string is safe to inject inside a
/// DOUBLE-quoted POSIX shell string (`"…__VERSION__…"`). Restricted to a
/// plausible semver charset — ASCII alphanumerics plus `.`, `+`, `-`. Anything
/// else (a `"`, `$`, backtick, whitespace, …) could break the quoting or, worse,
/// inject shell, so we reject it and the caller falls back to the absent-only
/// check. The version comes from a `VERSION` file we control, but this guard is
/// cheap and removes any doubt.
fn is_shell_safe_version(v: &str) -> bool {
    !v.is_empty()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

/// Build the remote launch command for the VibeDev ACP agent.
///
/// `original_args` is the agent's client-configured argv (now just the flags,
/// e.g. `--acp`, since the compiled single binary IS the program — there is no
/// entry script). All flags are forwarded to the remote `vibedev-agent`.
///
/// VIBEDEV (agent-OTA): `expected_version` is the local app's bundled agent
/// version (from `bundled_agent_version`). When it is `Some(v)` and `v` is
/// shell-safe, the provisioning guard ALSO fires when the remote's
/// `dist/VERSION` is absent or doesn't match `v`, so a stale remote is
/// re-provisioned and kept in lockstep with the local app. When it is `None`
/// (unknown — dev mode, no bundled agent) or `v` is unsafe, the guard is the
/// original absent-only check (unchanged behavior). The function is pure (no
/// I/O): the version is passed in so it's fully unit-testable.
///
/// Returns `(program, args)` = `("sh", ["-c", <script>])` to assign onto the
/// remote `AgentServerCommand`.
pub fn remote_launch_command(
    original_args: &[String],
    expected_version: Option<&str>,
) -> (String, Vec<String>) {
    // Forward the agent flags. The compiled single binary IS the program (there's
    // no entry script in args anymore), so forward ALL args (`--acp`, ...). ACP
    // flags are simple tokens; join with spaces for the inner `exec`.
    let flags = original_args
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let flag_suffix = if flags.is_empty() {
        String::new()
    } else {
        format!(" {flags}")
    };

    // VIBEDEV (agent-OTA): build the provisioning guard. Baseline is the
    // absent-check (the agent binary missing). When a shell-safe expected
    // version is known, add a version mismatch as well:
    //   [ "$(cat "$D/VERSION" 2>/dev/null)" != "__VERSION__" ]
    // POSIX `$(...)` strips trailing newlines, so `$(cat .../VERSION)` yields the
    // trimmed version (the VERSION file ends in a newline) — no `tr`/`[:space:]`
    // needed. If `VERSION` is ABSENT, `cat` fails to an empty string, which
    // `!= "v"` makes true → re-provision; so the version compare subsumes the
    // "VERSION missing" case (no separate `! -f VERSION` test). An unsafe version
    // string is treated as unknown (falls back to the absent-only guard) so it
    // can never break the quoting or inject shell. `__VERSION__` is substituted
    // like `__URL__` to keep the `format!` brace-escape hazard out of the script.
    let guard = match expected_version {
        Some(v) if is_shell_safe_version(v) => {
            "[ ! -x \"$D/vibedev-agent\" ] \
|| [ \"$(cat \"$D/VERSION\" 2>/dev/null)\" != \"__VERSION__\" ]"
        }
        _ => "[ ! -x \"$D/vibedev-agent\" ]",
    };

    // Single POSIX program (one `-c` arg). The SSH transport single-quotes this
    // whole string for the outer shell; `$HOME`/`$D` expand on the remote.
    // `__URL__` / `__FLAGS__` / `__GUARD__` are substituted to avoid `format!`
    // brace-escaping against the shell's `{ …; }` group.
    let template = "D=\"$HOME/.vibedev/agent\"; \
if __GUARD__; then \
{ mkdir -p \"$D\" \
&& curl -fL --connect-timeout 20 \"__URL__\" -o \"$D/.bundle.tgz\" \
&& tar xzf \"$D/.bundle.tgz\" -C \"$D\" \
&& rm -f \"$D/.bundle.tgz\" \
&& chmod +x \"$D/vibedev-agent\" \"$D/vendor/ripgrep/x64-linux/rg\"; } 1>&2 \
|| { echo \"vibedev: agent bundle provisioning failed\" 1>&2; exit 1; }; \
fi; \
if [ -n \"$VIBEDEV_AUTH_JSON\" ]; then \
{ mkdir -p \"$HOME/.vibedev\" \
&& printf %s \"$VIBEDEV_AUTH_JSON\" > \"$HOME/.vibedev/auth.json\" \
&& chmod 600 \"$HOME/.vibedev/auth.json\"; } 1>&2; \
fi; \
exec \"$D/vibedev-agent\"__FLAGS__";

    // Substitute the guard FIRST (it may itself carry `__VERSION__`), then fill
    // the remaining placeholders. `__VERSION__` only survives the guard branch
    // that injected it; the absent-only branch contains no `__VERSION__`, so the
    // (harmless) replace is a no-op there.
    let version_token = expected_version
        .filter(|v| is_shell_safe_version(v))
        .unwrap_or("");
    let script = template
        .replace("__GUARD__", guard)
        .replace("__URL__", AGENT_BUNDLE_URL)
        .replace("__VERSION__", version_token)
        .replace("__FLAGS__", &flag_suffix);

    ("sh".to_string(), vec!["-c".to_string(), script])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script_of(args: &[String], expected_version: Option<&str>) -> String {
        let (program, argv) = remote_launch_command(args, expected_version);
        assert_eq!(program, "sh");
        assert_eq!(argv[0], "-c");
        argv[1].clone()
    }

    #[test]
    fn wraps_in_sh_dash_c() {
        let (program, argv) = remote_launch_command(&["--acp".to_string()], None);
        assert_eq!(program, "sh");
        assert_eq!(argv.len(), 2);
        assert_eq!(argv[0], "-c");
    }

    #[test]
    fn self_provisions_and_execs_remote_bundle() {
        let s = script_of(&["--acp".to_string()], None);
        // provisioning is guarded on the bundle being absent
        assert!(s.contains("if [ ! -x \"$D/vibedev-agent\" ]"));
        // downloads the hosted bundle and restores the executable bit
        assert!(s.contains(AGENT_BUNDLE_URL));
        assert!(s.contains("tar xzf"));
        assert!(s.contains("chmod +x \"$D/vibedev-agent\" \"$D/vendor/ripgrep/x64-linux/rg\""));
        // provisioning output goes to stderr so stdout stays clean for ACP
        assert!(s.contains("1>&2"));
        // writes the account auth.json (api_key + available_models) from env so
        // the remote agent is signed in and shows the gateway models
        assert!(s.contains("printf %s \"$VIBEDEV_AUTH_JSON\" > \"$HOME/.vibedev/auth.json\""));
        // finally execs the single-binary agent with the forwarded flag
        assert!(s.contains("exec \"$D/vibedev-agent\" --acp"));
    }

    #[test]
    fn forwards_all_flags() {
        // The compiled single binary IS the program — args are just the flags
        // (no leading entry script to skip), so ALL are forwarded to the exec.
        let s = script_of(&["--acp".to_string(), "--verbose".to_string()], None);
        assert!(s.ends_with("exec \"$D/vibedev-agent\" --acp --verbose"));
    }

    #[test]
    fn handles_no_extra_flags() {
        let s = script_of(&[], None);
        assert!(s.ends_with("exec \"$D/vibedev-agent\""));
    }

    // VIBEDEV (agent-OTA): version-check guard tests.

    /// With a known shell-safe version, the guard ALSO re-provisions on a
    /// version mismatch (the `VERSION != v` arm) while STILL keeping the
    /// absent-check arm — they're OR'd, so a stale OR absent bundle both
    /// re-provision.
    #[test]
    fn version_check_present_when_expected_version_known() {
        let s = script_of(&["--acp".to_string()], Some("2.6.5"));
        // the version-mismatch re-provision arm (cat the remote VERSION, compare)
        assert!(s.contains("[ \"$(cat \"$D/VERSION\" 2>/dev/null)\" != \"2.6.5\" ]"));
        // the absent-check arm is still present (OR'd in front)
        assert!(s.contains("[ ! -x \"$D/vibedev-agent\" ]"));
        // and it still execs the bundle (with the forwarded flag) regardless
        assert!(s.contains("exec \"$D/vibedev-agent\" --acp"));
    }

    /// With NO expected version (dev / no bundled agent), the guard is the pure
    /// absent-check — the script never mentions `VERSION` (old behavior).
    #[test]
    fn absent_only_when_no_expected_version() {
        let s = script_of(&["--acp".to_string()], None);
        assert!(s.contains("[ ! -x \"$D/vibedev-agent\" ]"));
        // no version comparison at all
        assert!(!s.contains("$D/VERSION"));
    }

    /// Shell-safety: an unsafe version string (contains `"` / `$` / shell
    /// metacharacters) is treated as unknown — it must NOT appear in the script
    /// AND the script must fall back to the pure absent-check (no `VERSION`
    /// comparison). This proves the injection guard.
    #[test]
    fn unsafe_version_falls_back_to_absent_check() {
        let evil = "2.6.5\"; rm -rf /";
        let s = script_of(&["--acp".to_string()], Some(evil));
        // the malicious payload never reaches the script
        assert!(!s.contains("rm -rf"));
        assert!(!s.contains(evil));
        // and we fell back to the absent-only guard (no version comparison)
        assert!(!s.contains("$D/VERSION"));
        assert!(s.contains("[ ! -x \"$D/vibedev-agent\" ]"));
        // still execs normally
        assert!(s.contains("exec \"$D/vibedev-agent\" --acp"));
    }

    /// The exec line is identical regardless of whether a version is known — the
    /// version only changes the provisioning GUARD, never the launch.
    #[test]
    fn exec_line_is_version_independent() {
        let with = script_of(&["--acp".to_string()], Some("9.9.9"));
        let without = script_of(&["--acp".to_string()], None);
        assert!(with.ends_with("exec \"$D/vibedev-agent\" --acp"));
        assert!(without.ends_with("exec \"$D/vibedev-agent\" --acp"));
    }

    /// `is_shell_safe_version` accepts a plausible semver charset and rejects
    /// anything with quoting / shell metacharacters or empties.
    #[test]
    fn shell_safe_version_charset() {
        assert!(is_shell_safe_version("2.6.5"));
        assert!(is_shell_safe_version("1.0.0-rc.1+build.7"));
        assert!(is_shell_safe_version("v19j-patch72"));
        assert!(!is_shell_safe_version(""));
        assert!(!is_shell_safe_version("2.6.5\""));
        assert!(!is_shell_safe_version("2.6.5; rm -rf /")); // space + ;
        assert!(!is_shell_safe_version("$(whoami)"));
        assert!(!is_shell_safe_version("2.6.5`id`"));
    }
}
