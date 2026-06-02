use anyhow::{Context as _, bail};
use collections::HashMap;
use dap::{
    StartDebuggingRequestArguments,
    adapters::{
        DebugTaskDefinition, DownloadedFileType, TcpArguments, download_adapter_from_github,
        latest_github_release,
    },
};
use fs::Fs;
use futures::StreamExt;
use gpui::{AsyncApp, SharedString};
use language::LanguageName;
use log::warn;
use serde_json::{Map, Value};
use task::TcpArgumentsTemplate;
use util;

use std::{
    env::consts,
    ffi::OsStr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::OnceLock,
};

use crate::*;

#[derive(Default, Debug)]
pub(crate) struct GoDebugAdapter {
    shim_path: OnceLock<PathBuf>,
}

impl GoDebugAdapter {
    const ADAPTER_NAME: &'static str = "Delve";
    async fn fetch_latest_adapter_version(
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<AdapterVersion> {
        let release = latest_github_release(
            "zed-industries/delve-shim-dap",
            true,
            false,
            delegate.http_client(),
        )
        .await?;

        let os = match consts::OS {
            "macos" => "apple-darwin",
            "linux" => "unknown-linux-gnu",
            "windows" => "pc-windows-msvc",
            other => bail!("Running on unsupported os: {other}"),
        };
        let suffix = if consts::OS == "windows" {
            ".zip"
        } else {
            ".tar.gz"
        };
        let asset_name = format!("delve-shim-dap-{}-{os}{suffix}", consts::ARCH);
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .with_context(|| format!("no asset found matching `{asset_name:?}`"))?;

        Ok(AdapterVersion {
            tag_name: release.tag_name,
            url: asset.browser_download_url.clone(),
        })
    }
    async fn install_shim(&self, delegate: &Arc<dyn DapDelegate>) -> anyhow::Result<PathBuf> {
        if let Some(path) = self.shim_path.get().cloned() {
            return Ok(path);
        }

        let adapter_dir = paths::debug_adapters_dir().join("delve-shim-dap");

        match Self::fetch_latest_adapter_version(delegate).await {
            Ok(asset) => {
                let ty = if consts::OS == "windows" {
                    DownloadedFileType::Zip
                } else {
                    DownloadedFileType::GzipTar
                };
                download_adapter_from_github(
                    "delve-shim-dap".into(),
                    asset.clone(),
                    ty,
                    delegate.as_ref(),
                )
                .await?;

                let path = adapter_dir
                    .join(format!("delve-shim-dap_{}", asset.tag_name))
                    .join(format!("delve-shim-dap{}", consts::EXE_SUFFIX));
                self.shim_path.set(path.clone()).ok();

                Ok(path)
            }
            Err(error) => {
                let binary_name = format!("delve-shim-dap{}", consts::EXE_SUFFIX);
                let mut cached = None;
                if let Ok(mut entries) = delegate.fs().read_dir(&adapter_dir).await {
                    while let Some(entry) = entries.next().await {
                        if let Ok(version_dir) = entry {
                            let candidate = version_dir.join(&binary_name);
                            if delegate
                                .fs()
                                .metadata(&candidate)
                                .await
                                .is_ok_and(|m| m.is_some())
                            {
                                cached = Some(candidate);
                                break;
                            }
                        }
                    }
                }

                if let Some(path) = cached {
                    warn!("获取最新 delve-shim-dap 失败,使用缓存版本: {error:#}");
                    self.shim_path.set(path.clone()).ok();
                    Ok(path)
                } else {
                    Err(error)
                }
            }
        }
    }
}

#[async_trait(?Send)]
impl DebugAdapter for GoDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        DebugAdapterName(Self::ADAPTER_NAME.into())
    }

    fn adapter_language_name(&self) -> Option<LanguageName> {
        Some(SharedString::new_static("Go").into())
    }

    fn dap_schema(&self) -> serde_json::Value {
        // Create common properties shared between launch and attach
        let common_properties = json!({
            "debugAdapter": {
                "enum": ["legacy", "dlv-dap"],
                "description": "选择用于此配置的调试适配器。",
                "default": "dlv-dap"
            },
            "stopOnEntry": {
                "type": "boolean",
                "description": "启动或附加后自动停止程序。",
                "default": false
            },
            "showLog": {
                "type": "boolean",
                "description": "显示 Delve 调试器的日志输出。对应 dlv 的 `--log` 标志。",
                "default": false
            },
            "cwd": {
                "type": "string",
                "description": "被调试程序工作目录的相对于工作区或绝对的路径。",
                "default": "${ZED_WORKTREE_ROOT}"
            },
            "dlvFlags": {
                "type": "array",
                "description": "`dlv` 的额外标志。请参阅 `dlv help` 获取支持的标志完整列表。",
                "items": {
                    "type": "string"
                },
                "default": []
            },
            "port": {
                "type": "number",
                "description": "调试服务器端口。对于远程配置,这是要连接的端口。",
                "default": 2345
            },
            "host": {
                "type": "string",
                "description": "调试服务器主机。对于远程配置,这是要连接的主机。",
                "default": "127.0.0.1"
            },
            "substitutePath": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "from": {
                            "type": "string",
                            "description": "要被替换的本地绝对路径。"
                        },
                        "to": {
                            "type": "string",
                            "description": "要替换为的远程绝对路径。"
                        }
                    }
                },
                "description": "用于调试的本地到远程路径映射。",
                "default": []
            },
            "trace": {
                "type": "string",
                "enum": ["verbose", "trace", "log", "info", "warn", "error"],
                "default": "error",
                "description": "调试日志级别。"
            },
            "backend": {
                "type": "string",
                "enum": ["default", "native", "lldb", "rr"],
                "description": "Delve 使用的后端。对应 `dlv` 的 `--backend` 标志。"
            },
            "logOutput": {
                "type": "string",
                "enum": ["debugger", "gdbwire", "lldbout", "debuglineerr", "rpc", "dap"],
                "description": "应产生调试输出的组件。",
                "default": "debugger"
            },
            "logDest": {
                "type": "string",
                "description": "Delve 的日志目标位置。"
            },
            "stackTraceDepth": {
                "type": "number",
                "description": "堆栈跟踪的最大深度。",
                "default": 50
            },
            "showGlobalVariables": {
                "type": "boolean",
                "default": false,
                "description": "在变量窗格中显示全局包变量。"
            },
            "showRegisters": {
                "type": "boolean",
                "default": false,
                "description": "在变量窗格中显示寄存器变量。"
            },
            "hideSystemGoroutines": {
                "type": "boolean",
                "default": false,
                "description": "在调用堆栈视图中隐藏系统协程。"
            },
            "console": {
                "default": "internalConsole",
                "description": "调试器的启动位置。",
                "enum": ["internalConsole", "integratedTerminal"]
            },
            "asRoot": {
                "default": false,
                "description": "以提升的权限进行调试(在 Unix 上)。",
                "type": "boolean"
            }
        });

        // Create launch-specific properties
        let launch_properties = json!({
            "program": {
                "type": "string",
                "description": "要调试的程序文件夹或文件的路径。",
                "default": "${ZED_WORKTREE_ROOT}"
            },
            "args": {
                "type": ["array", "string"],
                "description": "程序的命令行参数。",
                "items": {
                    "type": "string"
                },
                "default": []
            },
            "env": {
                "type": "object",
                "description": "被调试程序的环境变量。",
                "default": {}
            },
            "envFile": {
                "type": ["string", "array"],
                "items": {
                    "type": "string"
                },
                "description": "包含环境变量的文件路径。",
                "default": ""
            },
            "buildFlags": {
                "type": ["string", "array"],
                "items": {
                    "type": "string"
                },
                "description": "Go 编译器的标志。",
                "default": []
            },
            "output": {
                "type": "string",
                "description": "二进制文件的输出路径。",
                "default": "debug"
            },
            "mode": {
                "enum": [ "debug", "test", "exec", "replay", "core"],
                "description": "启动配置的调试模式。",
            },
            "traceDirPath": {
                "type": "string",
                "description": "记录跟踪的目录(用于 'replay' 模式)。",
                "default": ""
            },
            "coreFilePath": {
                "type": "string",
                "description": "Core dump 文件的路径(用于 'core' 模式)。",
                "default": ""
            }
        });

        // Create attach-specific properties
        let attach_properties = json!({
            "processId": {
                "anyOf": [
                    {
                        "enum": ["${command:pickProcess}", "${command:pickGoProcess}"],
                        "description": "使用进程选择器选择一个进程。"
                    },
                    {
                        "type": "string",
                        "description": "要附加到的进程名称。"
                    },
                    {
                        "type": "number",
                        "description": "要附加到的进程 ID。"
                    }
                ],
                "default": 0
            },
            "mode": {
                "enum": ["local", "remote"],
                "description": "本地或远程调试。",
                "default": "local"
            },
            "remotePath": {
                "type": "string",
                "description": "远程机器上的源代码路径。",
                "markdownDeprecationMessage": "请改用 `substitutePath`。",
                "default": ""
            }
        });

        // Create the final schema
        json!({
            "oneOf": [
                {
                    "allOf": [
                        {
                            "type": "object",
                            "required": ["request"],
                            "properties": {
                                "request": {
                                    "type": "string",
                                    "enum": ["launch"],
                                    "description": "启动新进程的请求"
                                }
                            }
                        },
                        {
                            "type": "object",
                            "properties": common_properties
                        },
                        {
                            "type": "object",
                            "required": ["program", "mode"],
                            "properties": launch_properties
                        }
                    ]
                },
                {
                    "allOf": [
                        {
                            "type": "object",
                            "required": ["request"],
                            "properties": {
                                "request": {
                                    "type": "string",
                                    "enum": ["attach"],
                                    "description": "附加到现有进程的请求"
                                }
                            }
                        },
                        {
                            "type": "object",
                            "properties": common_properties
                        },
                        {
                            "type": "object",
                            "required": ["mode"],
                            "properties": attach_properties
                        }
                    ]
                }
            ]
        })
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut args = match &zed_scenario.request {
            dap::DebugRequest::Attach(attach_config) => {
                json!({
                    "request": "attach",
                    "mode": "local",
                    "processId": attach_config.process_id,
                })
            }
            dap::DebugRequest::Launch(launch_config) => {
                let mode = if launch_config.program != "." {
                    "exec"
                } else {
                    "debug"
                };

                json!({
                    "request": "launch",
                    "mode": mode,
                    "program": launch_config.program,
                    "cwd": launch_config.cwd,
                    "args": launch_config.args,
                    "env": launch_config.env_json()
                })
            }
        };

        let map = args.as_object_mut().unwrap();

        if let Some(stop_on_entry) = zed_scenario.stop_on_entry {
            map.insert("stopOnEntry".into(), stop_on_entry.into());
        }

        Ok(DebugScenario {
            adapter: zed_scenario.adapter,
            label: zed_scenario.label,
            build: None,
            config: args,
            tcp_connection: None,
        })
    }

    async fn get_binary(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        task_definition: &DebugTaskDefinition,
        user_installed_path: Option<PathBuf>,
        user_args: Option<Vec<String>>,
        user_env: Option<HashMap<String, String>>,
        _cx: &mut AsyncApp,
    ) -> Result<DebugAdapterBinary> {
        let adapter_path = paths::debug_adapters_dir().join(&Self::ADAPTER_NAME);
        let dlv_binary = format!("dlv{}", consts::EXE_SUFFIX);
        let dlv_path = adapter_path.join(&dlv_binary);

        let delve_path = if let Some(path) = user_installed_path {
            path.to_string_lossy().into_owned()
        } else if let Some(path) = delegate.which(OsStr::new("dlv")).await {
            path.to_string_lossy().into_owned()
        } else if delegate.fs().is_file(&dlv_path).await {
            dlv_path.to_string_lossy().into_owned()
        } else {
            let go = delegate
                .which(OsStr::new("go"))
                .await
                .context("Go not found in path. Please install Go first, then Dlv will be installed automatically.")?;

            let adapter_path = paths::debug_adapters_dir().join(&Self::ADAPTER_NAME);

            let install_output = util::command::new_command(&go)
                .env("GO111MODULE", "on")
                .env("GOBIN", &adapter_path)
                .args(&["install", "github.com/go-delve/delve/cmd/dlv@latest"])
                .output()
                .await?;

            if !install_output.status.success() {
                bail!(
                    "通过 `go install` 安装 dlv 失败。标准输出: {:?}, 标准错误: {:?}\n 请尝试使用 'go install github.com/go-delve/delve/cmd/dlv@latest' 手动安装",
                    String::from_utf8_lossy(&install_output.stdout),
                    String::from_utf8_lossy(&install_output.stderr)
                );
            }

            adapter_path
                .join(&dlv_binary)
                .to_string_lossy()
                .into_owned()
        };

        let cwd = Some(
            task_definition
                .config
                .get("cwd")
                .and_then(|s| s.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| delegate.worktree_root_path().to_path_buf()),
        );

        let arguments;
        let command;
        let connection;

        let mut configuration = task_definition.config.clone();
        let mut envs = user_env.unwrap_or_default();

        if let Some(configuration) = configuration.as_object_mut() {
            configuration
                .entry("cwd")
                .or_insert_with(|| delegate.worktree_root_path().to_string_lossy().into());

            handle_envs(
                configuration,
                &mut envs,
                cwd.as_deref(),
                delegate.fs().clone(),
            )
            .await;
        }

        if let Some(connection_options) = &task_definition.tcp_connection {
            command = None;
            arguments = vec![];
            let (host, port, timeout) =
                crate::configure_tcp_connection(connection_options.clone()).await?;
            connection = Some(TcpArguments {
                host,
                port,
                timeout,
            });
        } else {
            let minidelve_path = self.install_shim(delegate).await?;
            let (host, port, _) =
                crate::configure_tcp_connection(TcpArgumentsTemplate::default()).await?;
            command = Some(minidelve_path.to_string_lossy().into_owned());
            connection = None;
            arguments = if let Some(mut args) = user_args {
                args.insert(0, delve_path);
                args
            } else if cfg!(windows) {
                vec![
                    delve_path,
                    "dap".into(),
                    "--listen".into(),
                    format!("{}:{}", host, port),
                    "--headless".into(),
                ]
            } else {
                vec![
                    delve_path,
                    "dap".into(),
                    "--listen".into(),
                    format!("{}:{}", host, port),
                ]
            };
        }
        Ok(DebugAdapterBinary {
            command,
            arguments,
            cwd,
            envs,
            connection,
            request_args: StartDebuggingRequestArguments {
                configuration,
                request: self.request_kind(&task_definition.config).await?,
            },
        })
    }
}

// delve doesn't do anything with the envFile setting, so we intercept it
async fn handle_envs(
    config: &mut Map<String, Value>,
    envs: &mut HashMap<String, String>,
    cwd: Option<&Path>,
    fs: Arc<dyn Fs>,
) -> Option<()> {
    let env_files = match config.get("envFile")? {
        Value::Array(arr) => arr.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
        Value::String(s) => vec![Some(s.as_str())],
        _ => return None,
    };

    let rebase_path = |path: PathBuf| {
        if path.is_absolute() {
            Some(path)
        } else {
            cwd.map(|p| p.join(path))
        }
    };

    let mut env_vars = HashMap::default();
    for path in env_files {
        let Some(path) = path
            .and_then(|s| PathBuf::from_str(s).ok())
            .and_then(rebase_path)
        else {
            continue;
        };

        if let Ok(file) = fs.open_sync(&path).await {
            let file_envs: HashMap<String, String> = dotenvy::from_read_iter(file)
                .filter_map(Result::ok)
                .collect();
            envs.extend(file_envs.iter().map(|(k, v)| (k.clone(), v.clone())));
            env_vars.extend(file_envs);
        } else {
            warn!("While starting Go debug session: failed to read env file {path:?}");
        };
    }

    let mut env_obj: serde_json::Map<String, Value> = serde_json::Map::new();

    for (k, v) in env_vars {
        env_obj.insert(k, Value::String(v));
    }

    if let Some(existing_env) = config.get("env").and_then(|v| v.as_object()) {
        for (k, v) in existing_env {
            env_obj.insert(k.clone(), v.clone());
        }
    }

    if !env_obj.is_empty() {
        config.insert("env".to_string(), Value::Object(env_obj));
    }

    // remove envFile now that it's been handled
    config.remove("envFile");
    Some(())
}
