//! VIBEDEV (V2-PLAN-8): reverse SSH tunnel for remote ACP agents.
//!
//! A remote dev host often has no clean internet egress of its own (the test
//! box's LAN proxy is frequently down; many corporate boxes are inbound-NAT
//! only). The VibeDev agent running there still needs to reach the gateway
//! (`aitoken.bigopen.cn`). This module routes the agent's HTTPS traffic BACK
//! through the CLIENT (the IDE, which does have egress):
//!
//! ```text
//! remote agent  --HTTPS_PROXY=127.0.0.1:R-->  ssh -R  -->  client in-proc
//!   CONNECT proxy (127.0.0.1:L)  -->  real gateway
//! ```
//!
//! A plain HTTP CONNECT proxy is used (not a raw port forward) so the agent
//! keeps talking to `https://aitoken.bigopen.cn/v1` verbatim — the request
//! signature is computed over method+url+body, so the URL must not change.
//!
//! v1 scope: one session-lived tunnel per host (idempotent), key/agent-based
//! SSH auth (reuses the user's ssh config; a password-only host would prompt
//! and is unsupported here), a dynamically-allocated remote loopback port. The
//! proxy + `ssh -R` child are kept alive for the IDE session and reaped at app
//! quit by [`teardown_tunnels`] (the app-quit hook calls it).

use anyhow::{Context as _, Result};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

/// host -> remote proxy port, for idempotency across reconnects in one session.
fn tunnels() -> &'static Mutex<HashMap<String, u16>> {
    static TUNNELS: OnceLock<Mutex<HashMap<String, u16>>> = OnceLock::new();
    TUNNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The live `ssh -R` children, kept alive for the IDE session (dropping a
/// `Child` handle does NOT kill the process — it detaches it — so we must hold
/// the handles and explicitly kill them at teardown).
fn children() -> &'static Mutex<Vec<std::process::Child>> {
    static CHILDREN: OnceLock<Mutex<Vec<std::process::Child>>> = OnceLock::new();
    CHILDREN.get_or_init(|| Mutex::new(Vec::new()))
}

/// Keep the `ssh -R` child alive for the session by registering its handle.
fn keep_alive(child: std::process::Child) {
    if let Ok(mut v) = children().lock() {
        v.push(child);
    }
}

/// Kill every tracked `ssh -R` reverse-tunnel child and drain the registry.
///
/// Called from the app-quit hook ([`crate::launcher`]). On Windows there is no
/// process-tree reaping and Zed exits via `process::exit`, which skips `Drop`,
/// so without this the `ssh` children outlive the IDE and keep their remote
/// forwards bound (the orphan class tracked in
/// `reference_zed_orphan_bun_windows`). `ssh -R -N` children are leaf processes
/// (no descendants of their own), so a single `kill` + `wait` per child fully
/// reaps them. Best-effort: a child that already exited just errors and is
/// dropped from the registry anyway.
pub fn teardown_tunnels() {
    let Ok(mut v) = children().lock() else {
        return;
    };
    for mut child in v.drain(..) {
        let _ = child.kill();
        let _ = child.wait();
    }
    // The per-host idempotency map now points at forwards that no longer exist;
    // clear it so a later reconnect in the same process re-establishes cleanly.
    if let Ok(mut t) = tunnels().lock() {
        t.clear();
    }
}

/// Ensure a reverse tunnel is up for `host` and return the remote loopback port
/// the agent should use as `HTTPS_PROXY` (`http://127.0.0.1:<port>`). Idempotent
/// per host for the session. Returns an error if the local proxy or `ssh -R`
/// could not be started — callers should then fall back to the remote's own
/// egress rather than fail the agent.
///
/// NOTE: this blocks (up to a few seconds) while verifying the forward — call it
/// from a background thread/task, not the UI executor.
pub fn ensure_tunnel(
    host: &str,
    port: Option<u16>,
    username: Option<&str>,
    extra_args: &[String],
) -> Result<u16> {
    if let Some(&existing) = tunnels().lock().unwrap().get(host) {
        return Ok(existing);
    }

    // Local in-process CONNECT proxy on an ephemeral loopback port.
    let listener = TcpListener::bind("127.0.0.1:0").context("bind local CONNECT proxy")?;
    let local_port = listener.local_addr()?.port();
    std::thread::Builder::new()
        .name("vibedev-tunnel-proxy".into())
        .spawn(move || run_connect_proxy(listener))
        .context("spawn CONNECT proxy thread")?;

    // Establish `ssh -R` with a DYNAMIC remote port, verified and retried. A
    // fixed port collided with a still-bound orphan tunnel from a previous IDE
    // run (Windows leaves the ssh child running on process exit), and spawning
    // without verifying let the agent point `HTTPS_PROXY` at a forward that
    // never actually came up — surfacing later as "API Error: Connection error".
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=3u32 {
        match spawn_reverse_tunnel(host, port, username, extra_args, local_port) {
            Ok(remote_port) => {
                tunnels()
                    .lock()
                    .unwrap()
                    .insert(host.to_string(), remote_port);
                log::info!(
                    "vibedev reverse tunnel up for {host}: remote 127.0.0.1:{remote_port} -> local CONNECT proxy 127.0.0.1:{local_port}"
                );
                return Ok(remote_port);
            }
            Err(error) => {
                log::warn!(
                    "vibedev reverse tunnel attempt {attempt}/3 for {host} failed: {error:#}"
                );
                last_err = Some(error);
                std::thread::sleep(Duration::from_millis(400 * attempt as u64));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("reverse tunnel could not be established")))
}

/// Spawn `ssh -R 127.0.0.1:0:127.0.0.1:<local_port> <host>` and block until ssh
/// reports the dynamically-allocated remote port via its
/// `Allocated port N for remote forward to …` stderr line — which only prints
/// once the forward is actually live. Returns that port. Errors (so the caller
/// retries) if ssh exits or no port is allocated within the timeout.
fn spawn_reverse_tunnel(
    host: &str,
    port: Option<u16>,
    username: Option<&str>,
    extra_args: &[String],
    local_port: u16,
) -> Result<u16> {
    let mut cmd = Command::new("ssh");
    if let Some(p) = port {
        cmd.arg("-p").arg(p.to_string());
    }
    if let Some(u) = username {
        cmd.arg("-l").arg(u);
    }
    for arg in extra_args {
        cmd.arg(arg);
    }
    // `-v` makes ssh print "Allocated port N for remote forward …" (the verify
    // signal). `0` asks the remote sshd to pick a free port — no collision with
    // an orphaned forward. `ExitOnForwardFailure` turns a refused forward into a
    // clean exit (caller retries) instead of a half-up tunnel.
    cmd.arg("-v")
        .arg("-N")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-R")
        .arg(format!("127.0.0.1:0:127.0.0.1:{local_port}"))
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().context("spawn `ssh -R` reverse tunnel")?;
    let stderr = child.stderr.take().context("capture ssh stderr")?;
    let (tx, rx) = mpsc::channel::<u16>();
    std::thread::Builder::new()
        .name("vibedev-tunnel-watch".into())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            let mut reported = false;
            for line in reader.lines().map_while(std::result::Result::ok) {
                if !reported {
                    if let Some(p) = parse_allocated_port(&line) {
                        let _ = tx.send(p);
                        reported = true;
                    }
                }
                // keep draining so a chatty `ssh -v` never blocks on a full pipe
            }
        })
        .ok();
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(remote_port) => {
            keep_alive(child);
            Ok(remote_port)
        }
        Err(_) => {
            let _ = child.kill();
            anyhow::bail!("`ssh -R` did not establish a remote forward within 8s")
        }
    }
}

/// Parse OpenSSH's `Allocated port 12345 for remote forward to 127.0.0.1:…`.
/// Tolerant of a leading log prefix (some builds print `debug1: Allocated …`).
fn parse_allocated_port(line: &str) -> Option<u16> {
    const MARKER: &str = "Allocated port ";
    let idx = line.find(MARKER)?;
    line[idx + MARKER.len()..]
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Blocking accept loop: one thread per accepted connection.
fn run_connect_proxy(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(client) = stream else { continue };
        std::thread::spawn(move || {
            if let Err(error) = handle_connect(client) {
                log::debug!("vibedev tunnel proxy connection ended: {error:#}");
            }
        });
    }
}

/// Handle one HTTP `CONNECT host:port` request: dial the target and splice the
/// two sockets together in both directions.
fn handle_connect(mut client: TcpStream) -> Result<()> {
    // Read the request head up to the blank line.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if client.read(&mut byte)? == 0 {
            anyhow::bail!("client closed before request was complete");
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        anyhow::ensure!(head.len() <= 16 * 1024, "CONNECT request head too large");
    }
    let text = String::from_utf8_lossy(&head);
    let request_line = text.lines().next().context("empty CONNECT request")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    anyhow::ensure!(
        method.eq_ignore_ascii_case("CONNECT"),
        "tunnel proxy only supports CONNECT, got `{method}`"
    );

    let upstream = TcpStream::connect(target).with_context(|| format!("dial upstream {target}"))?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    client.flush()?;

    // Full-duplex splice. Each direction runs to EOF, then half-closes its
    // write side so the peer's copy also returns.
    let mut client_read = client.try_clone()?;
    let mut upstream_write = upstream.try_clone()?;
    let pump = std::thread::spawn(move || {
        let mut upstream_read = upstream;
        let mut client_write = client;
        let _ = std::io::copy(&mut upstream_read, &mut client_write);
        let _ = client_write.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut client_read, &mut upstream_write);
    let _ = upstream_write.shutdown(Shutdown::Write);
    let _ = pump.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{children, keep_alive, parse_allocated_port, teardown_tunnels};

    #[test]
    fn parses_openssh_allocated_port_line() {
        assert_eq!(
            parse_allocated_port("Allocated port 41273 for remote forward to 127.0.0.1:52001"),
            Some(41273)
        );
    }

    #[test]
    fn tolerates_log_prefix() {
        assert_eq!(
            parse_allocated_port("debug1: Allocated port 9 for remote forward to 127.0.0.1:1"),
            Some(9)
        );
    }

    #[test]
    fn ignores_unrelated_lines() {
        assert_eq!(parse_allocated_port("debug1: Authentication succeeded"), None);
        assert_eq!(parse_allocated_port("Allocated port  for remote forward"), None);
    }

    /// Spawn a long-lived leaf process standing in for an `ssh -R -N` child.
    /// Like the real thing it has no children of its own, so a single-PID kill
    /// fully reaps it.
    #[cfg(target_os = "windows")]
    fn spawn_test_sleeper() -> std::process::Child {
        std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn test sleeper")
    }

    #[cfg(not(target_os = "windows"))]
    fn spawn_test_sleeper() -> std::process::Child {
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn test sleeper")
    }

    /// Whether `pid` is a live, signalable process — checked out-of-band rather
    /// than through the `Child` handle (which `teardown_tunnels` consumed), so
    /// the assertion reflects the real OS process state. Locale-independent: the
    /// numeric PID column appears only while the process is alive.
    #[cfg(target_os = "windows")]
    fn pid_alive(pid: u32) -> bool {
        match std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn pid_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn wait_until_dead(pid: u32, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if !pid_alive(pid) {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// `teardown_tunnels` must actually KILL the tracked `ssh -R` children, not
    /// merely drop the handles: dropping a `std::process::Child` detaches it (the
    /// process keeps running), which is exactly the Windows orphan bug this fix
    /// closes. So spawn a real leaf process, register it via `keep_alive`, and
    /// assert teardown both ends the process and drains the registry.
    #[test]
    fn teardown_tunnels_kills_tracked_children() {
        let child = spawn_test_sleeper();
        let pid = child.id();
        assert!(pid_alive(pid), "sleeper (pid {pid}) should run before teardown");

        keep_alive(child);
        assert_eq!(
            children().lock().unwrap().len(),
            1,
            "keep_alive should register the child"
        );

        teardown_tunnels();

        assert!(
            children().lock().unwrap().is_empty(),
            "teardown_tunnels should drain the registry"
        );
        assert!(
            wait_until_dead(pid, std::time::Duration::from_secs(5)),
            "teardown_tunnels should kill the sleeper (pid {pid})"
        );
    }
}
