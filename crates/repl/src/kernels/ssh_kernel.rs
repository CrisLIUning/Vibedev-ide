use super::{KernelSession, RunningKernel, SshRemoteKernelSpecification, start_kernel_tasks};
use anyhow::{Context as _, Result};
use client::proto;

use futures::{
    AsyncBufReadExt as _, StreamExt as _,
    channel::mpsc::{self},
    io::BufReader,
};
use gpui::{App, Entity, Task, Window};
use project::Project;
use runtimelib::{ExecutionState, JupyterMessage, KernelInfoReply};
use std::path::PathBuf;
use util::ResultExt;

#[derive(Debug)]
pub struct SshRunningKernel {
    request_tx: mpsc::Sender<JupyterMessage>,
    stdin_tx: mpsc::Sender<JupyterMessage>,
    execution_state: ExecutionState,
    kernel_info: Option<KernelInfoReply>,
    working_directory: PathBuf,
    _ssh_tunnel_process: util::command::Child,
    _local_connection_file: PathBuf,
    kernel_id: String,
    project: Entity<Project>,
    project_id: u64,
}

impl SshRunningKernel {
    pub fn new<S: KernelSession + 'static>(
        kernel_spec: SshRemoteKernelSpecification,
        working_directory: PathBuf,
        project: Entity<Project>,
        session: Entity<S>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Box<dyn RunningKernel>>> {
        let client = project.read(cx).client();
        let remote_client = project.read(cx).remote_client();
        let project_id = project
            .read(cx)
            .remote_id()
            .unwrap_or(proto::REMOTE_SERVER_PROJECT_ID);

        window.spawn(cx, async move |cx| {
            let command = kernel_spec
                .kernelspec
                .argv
                .first()
                .cloned()
                .unwrap_or_default();
            let args = kernel_spec
                .kernelspec
                .argv
                .iter()
                .skip(1)
                .cloned()
                .collect();

            let request = proto::SpawnKernel {
                kernel_name: kernel_spec.name.clone(),
                working_directory: working_directory.to_string_lossy().to_string(),
                project_id,
                command,
                args,
            };
            let response = if let Some(remote_client) = remote_client.as_ref() {
                remote_client
                    .read_with(cx, |client, _| client.proto_client())
                    .request(request)
                    .await?
            } else {
                client.request(request).await?
            };

            let kernel_id = response.kernel_id.clone();
            let connection_info: serde_json::Value =
                serde_json::from_str(&response.connection_file)?;

            // Setup SSH Tunneling - allocate local ports
            let mut local_ports = Vec::new();
            for _ in 0..5 {
                let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
                let port = listener.local_addr()?.port();
                drop(listener);
                local_ports.push(port);
            }

            let remote_shell_port = connection_info["shell_port"]
                .as_u64()
                .context("缺少 shell_port")? as u16;
            let remote_iopub_port = connection_info["iopub_port"]
                .as_u64()
                .context("缺少 iopub_port")? as u16;
            let remote_stdin_port = connection_info["stdin_port"]
                .as_u64()
                .context("缺少 stdin_port")? as u16;
            let remote_control_port = connection_info["control_port"]
                .as_u64()
                .context("缺少 control_port")? as u16;
            let remote_hb_port = connection_info["hb_port"]
                .as_u64()
                .context("缺少 hb_port")? as u16;

            let forwards = vec![
                (local_ports[0], "127.0.0.1".to_string(), remote_shell_port),
                (local_ports[1], "127.0.0.1".to_string(), remote_iopub_port),
                (local_ports[2], "127.0.0.1".to_string(), remote_stdin_port),
                (local_ports[3], "127.0.0.1".to_string(), remote_control_port),
                (local_ports[4], "127.0.0.1".to_string(), remote_hb_port),
            ];

            let remote_client = remote_client.ok_or_else(|| anyhow::anyhow!("无远程客户端"))?;
            let command_template = cx.update(|_window, cx| {
                remote_client.read(cx).build_forward_ports_command(forwards)
            })??;

            let mut command = util::command::new_command(&command_template.program);
            command.args(&command_template.args);
            command.envs(&command_template.env);

            let mut ssh_tunnel_process = command.spawn().context("无法启动 SSH 隧道")?;

            let stderr = ssh_tunnel_process.stderr.take();
            cx.spawn(async move |_cx| {
                if let Some(stderr) = stderr {
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Some(Ok(line)) = lines.next().await {
                        log::warn!("SSH 隧道标准错误: {}", line);
                    }
                }
            })
            .detach();

            let stdout = ssh_tunnel_process.stdout.take();
            cx.spawn(async move |_cx| {
                if let Some(stdout) = stdout {
                    let reader = BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Some(Ok(line)) = lines.next().await {
                        log::debug!("SSH 隧道标准输出: {}", line);
                    }
                }
            })
            .detach();

            // We might or might not need this, perhaps we can just wait for a second or test it this way
            let shell_port = local_ports[0];
            let max_attempts = 100;
            let mut connected = false;
            for attempt in 0..max_attempts {
                match smol::net::TcpStream::connect(format!("127.0.0.1:{}", shell_port)).await {
                    Ok(_) => {
                        connected = true;
                        log::info!(
                            "内核 {} 的 SSH 隧道已在第 {} 次尝试时建立",
                            kernel_id,
                            attempt + 1
                        );
                        // giving the tunnel a moment to fully establish forwarding
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(500))
                            .await;
                        break;
                    }
                    Err(err) => {
                        if attempt % 10 == 0 {
                            log::debug!(
                                "等待 SSH 隧道 (尝试 {}/{}): {}",
                                attempt + 1,
                                max_attempts,
                                err
                            );
                        }
                        if attempt < max_attempts - 1 {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(100))
                                .await;
                        }
                    }
                }
            }
            if !connected {
                anyhow::bail!(
                    "SSH 隧道在 {} 次尝试后仍无法建立",
                    max_attempts
                );
            }

            let mut local_connection_info = connection_info.clone();
            local_connection_info["shell_port"] = serde_json::json!(local_ports[0]);
            local_connection_info["iopub_port"] = serde_json::json!(local_ports[1]);
            local_connection_info["stdin_port"] = serde_json::json!(local_ports[2]);
            local_connection_info["control_port"] = serde_json::json!(local_ports[3]);
            local_connection_info["hb_port"] = serde_json::json!(local_ports[4]);
            local_connection_info["ip"] = serde_json::json!("127.0.0.1");

            let local_connection_file =
                std::env::temp_dir().join(format!("zed_ssh_kernel_{}.json", kernel_id));
            std::fs::write(
                &local_connection_file,
                serde_json::to_string_pretty(&local_connection_info)?,
            )?;

            // Parse connection info and create ZMQ connections
            let connection_info_struct: runtimelib::ConnectionInfo =
                serde_json::from_value(local_connection_info)?;
            let session_id = uuid::Uuid::new_v4().to_string();

            let output_socket = runtimelib::create_client_iopub_connection(
                &connection_info_struct,
                "",
                &session_id,
            )
            .await
            .context("创建 iopub 连接失败。远程环境中是否安装了 `ipykernel`?请尝试在远程主机上运行 `pip install ipykernel`。")?;

            let peer_identity = runtimelib::peer_identity_for_session(&session_id)?;
            let shell_socket = runtimelib::create_client_shell_connection_with_identity(
                &connection_info_struct,
                &session_id,
                peer_identity.clone(),
            )
            .await
            .context("无法创建 Shell 连接")?;
            let control_socket =
                runtimelib::create_client_control_connection(&connection_info_struct, &session_id)
                    .await
                    .context("无法创建 control 连接")?;
            let stdin_socket = runtimelib::create_client_stdin_connection_with_identity(
                &connection_info_struct,
                &session_id,
                peer_identity,
            )
            .await
            .context("无法创建 stdin 连接")?;

            let (request_tx, stdin_tx) = start_kernel_tasks(
                session.clone(),
                output_socket,
                shell_socket,
                control_socket,
                stdin_socket,
                cx,
            );

            Ok(Box::new(SshRunningKernel {
                request_tx,
                stdin_tx,
                execution_state: ExecutionState::Idle,
                kernel_info: None,
                working_directory,
                _ssh_tunnel_process: ssh_tunnel_process,
                _local_connection_file: local_connection_file,
                kernel_id,
                project,
                project_id,
            }) as Box<dyn RunningKernel>)
        })
    }
}

impl RunningKernel for SshRunningKernel {
    fn request_tx(&self) -> mpsc::Sender<JupyterMessage> {
        self.request_tx.clone()
    }

    fn stdin_tx(&self) -> mpsc::Sender<JupyterMessage> {
        self.stdin_tx.clone()
    }

    fn working_directory(&self) -> &PathBuf {
        &self.working_directory
    }

    fn execution_state(&self) -> &ExecutionState {
        &self.execution_state
    }

    fn set_execution_state(&mut self, state: ExecutionState) {
        self.execution_state = state;
    }

    fn kernel_info(&self) -> Option<&KernelInfoReply> {
        self.kernel_info.as_ref()
    }

    fn set_kernel_info(&mut self, info: KernelInfoReply) {
        self.kernel_info = Some(info);
    }

    fn force_shutdown(&mut self, _window: &mut Window, cx: &mut App) -> Task<Result<()>> {
        let kernel_id = self.kernel_id.clone();
        let project_id = self.project_id;
        let client = self.project.read(cx).client();

        cx.background_executor().spawn(async move {
            let request = proto::KillKernel {
                kernel_id,
                project_id,
            };
            client.request::<proto::KillKernel>(request).await?;
            Ok(())
        })
    }

    fn kill(&mut self) {
        self._ssh_tunnel_process.kill().log_err();
    }
}
