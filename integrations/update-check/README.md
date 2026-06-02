# VibeDev × Update Check

Passive "new version available" channel for the VibeDev desktop app. The
client polls a single JSON file on `aitoken.bigopen.cn`; if the remote
`version` is greater than the running binary's `AppVersion`, the account
panel surfaces a "v1.5.1 available — download" hint that links straight to
the prebuilt installer URL.

No bytes are auto-downloaded, no installer auto-runs. Zed's upstream
`auto_update` crate is permanently disabled in the VibeDev fork (commit
`dcb40a7bf0`) because it pulls from `zed-industries/zed`'s GitHub releases
and would clobber a VibeDev install with vanilla Zed. This integration is
the minimal replacement: a fetch, a semver compare, and a click-through to
the user's browser.

## How it works

- Client: `crates/vibedev_account/src/update_check.rs`. Wired from
  `crates/zed/src/main.rs` (the architectural slot the old
  `auto_update::init` used to occupy).
- Endpoint: `https://aitoken.bigopen.cn/vibedev/latest.json`. Single
  static file — no auth, no path parameters, ~300 B over the wire.
- Cadence: once at startup, then every 24 h. HTTP failures are logged at
  `warn` and leave the previous snapshot in place (network down ≠ user
  problem).
- Surface: `UpdateStatus { available, latest_version, download_url,
  release_notes }` stored as a `gpui::Global`. Any panel calls
  `vibedev_account::update_check::current(cx)` for a synchronous read.

## Schema (`latest.json`)

```json
{
  "version": "1.5.0",
  "released_at": "2026-05-28T17:33:00+08:00",
  "download_url": "https://aitoken.bigopen.cn/downloads/VibeDev-x86_64.exe",
  "notes": "Initial public release."
}
```

| Field          | Type                  | Required | Notes                                                                                                |
| -------------- | --------------------- | -------- | ---------------------------------------------------------------------------------------------------- |
| `version`      | semver string         | yes      | Strict semver (`1.5.0`, `1.5.1-rc.2`); unparseable → "no update" (logged at `warn`).                 |
| `released_at`  | ISO-8601 datetime     | no       | Currently informational; reserved for a future "released N days ago" copy in the panel.              |
| `download_url` | absolute https URL    | yes      | Opens in the user's default browser when the panel's "Download" button is clicked.                   |
| `notes`        | short markdown string | no       | Single-paragraph release headline. Long-form changelogs belong on the download page, not here.       |

## Releasing a new VibeDev version

1. Build + sign the installer with the bundler
   (`crates/zed/resources/windows/zed.iss` for Windows; macOS/Linux
   bundlers per platform). Copy the artifact to the release webroot's
   `downloads/` directory. The deploy host, webroot path, and SSH access
   are environment-specific and are deliberately NOT stored in this repo —
   they live in the team's internal ops notes / deploy secrets
   (`~/.vibedev/deploy-secrets.json`, see below).
2. Bump the `version` field in `latest.json` to match the new install
   artifact's semver, refresh `released_at`, update `download_url` and
   `notes`.
3. Run `deploy-latest-json.ps1` (this directory) to upload the file. It
   reads SSH credentials from `~/.vibedev/deploy-secrets.json` so the
   actual key never touches the repo. The script also re-applies
   `www:www` ownership and `644` perms — nginx is picky.
4. Verify with `curl -sI https://aitoken.bigopen.cn/vibedev/latest.json`
   (HTTP/2 200, `content-type: application/json`).

Running clients pick the new version up at their next 24 h tick, or on
the next launch.

## Rollback

Edit `latest.json` to set `version` back to the previous good release
(do not edit the published installer artifact — leave it in place at
its original URL so any client already mid-download finishes cleanly),
then re-run the deploy script. Clients fetch the file fresh on next
poll; the update banner clears itself when local `>= remote`.
