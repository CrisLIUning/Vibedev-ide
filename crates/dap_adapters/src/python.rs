use crate::*;
use anyhow::{Context as _, bail};
use collections::HashMap;
use dap::{DebugRequest, StartDebuggingRequestArguments, adapters::DebugTaskDefinition};
use fs::RemoveOptions;
use futures::{StreamExt, TryStreamExt};
use gpui::http_client::AsyncBody;
use gpui::{AsyncApp, SharedString};
use json_dotpath::DotPaths;
use language::{LanguageName, Toolchain};
use paths::debug_adapters_dir;
use serde_json::Value;
use smol::fs::File;
use smol::io::AsyncReadExt;
use smol::lock::OnceCell;
use std::ffi::OsString;
use std::net::IpAddr;
use std::str::FromStr;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
use util::command::new_command;
use util::{ResultExt, paths::PathStyle, rel_path::RelPath};

enum DebugpyLaunchMode<'a> {
    Normal,
    AttachWithConnect { host: Option<&'a str> },
}

#[derive(Default)]
pub(crate) struct PythonDebugAdapter {
    base_venv_path: OnceCell<Result<Arc<Path>, String>>,
    debugpy_whl_base_path: OnceCell<Result<Arc<Path>, String>>,
}

impl PythonDebugAdapter {
    const ADAPTER_NAME: &'static str = "Debugpy";
    const DEBUG_ADAPTER_NAME: DebugAdapterName =
        DebugAdapterName(SharedString::new_static(Self::ADAPTER_NAME));

    const LANGUAGE_NAME: &'static str = "Python";

    async fn generate_debugpy_arguments<'a>(
        host: &'a IpAddr,
        port: u16,
        launch_mode: DebugpyLaunchMode<'a>,
        user_installed_path: Option<&'a Path>,
        user_args: Option<Vec<String>>,
    ) -> Result<Vec<String>> {
        let mut args = if let Some(user_installed_path) = user_installed_path {
            log::debug!(
                "使用用户安装的 debugpy 适配器,路径:{}",
                user_installed_path.display()
            );
            vec![user_installed_path.to_string_lossy().into_owned()]
        } else {
            let adapter_path = paths::debug_adapters_dir().join(Self::DEBUG_ADAPTER_NAME.as_ref());
            let path = adapter_path
                .join("debugpy")
                .join("adapter")
                .to_string_lossy()
                .into_owned();
            log::debug!("Using pip debugpy adapter from: {path}");
            vec![path]
        };

        args.extend(if let Some(args) = user_args {
            args
        } else {
            match launch_mode {
                DebugpyLaunchMode::Normal => {
                    vec![format!("--host={}", host), format!("--port={}", port)]
                }
                DebugpyLaunchMode::AttachWithConnect { host } => {
                    let mut args = vec!["connect".to_string()];

                    if let Some(host) = host {
                        args.push(format!("{host}:"));
                    }
                    args.push(format!("{port}"));
                    args
                }
            }
        });
        Ok(args)
    }

    async fn request_args(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        task_definition: &DebugTaskDefinition,
    ) -> Result<StartDebuggingRequestArguments> {
        let request = self.request_kind(&task_definition.config).await?;

        let mut configuration = task_definition.config.clone();
        if let Ok(console) = configuration.dot_get_mut("console") {
            // Use built-in Zed terminal if user did not explicitly provide a setting for console.
            if console.is_null() {
                *console = Value::String("integratedTerminal".into());
            }
        }

        if let Some(obj) = configuration.as_object_mut() {
            obj.entry("cwd")
                .or_insert(delegate.worktree_root_path().to_string_lossy().into());
        }

        Ok(StartDebuggingRequestArguments {
            configuration,
            request,
        })
    }

    async fn fetch_wheel(
        &self,
        toolchain: Option<Toolchain>,
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<Arc<Path>> {
        let download_dir = debug_adapters_dir().join(Self::ADAPTER_NAME).join("wheels");
        std::fs::create_dir_all(&download_dir)?;
        let venv_python = self.base_venv_path(toolchain, delegate).await?;

        let installation_succeeded = util::command::new_command(venv_python.as_ref())
            .args([
                "-m",
                "pip",
                "download",
                "debugpy",
                "--only-binary=:all:",
                "-d",
                download_dir.to_string_lossy().as_ref(),
            ])
            .output()
            .await
            .context("spawn system python")?
            .status
            .success();
        if !installation_succeeded {
            bail!("debugpy installation failed (could not fetch Debugpy's wheel)");
        }

        let wheel_path = std::fs::read_dir(&download_dir)?
            .find_map(|entry| {
                entry.ok().filter(|e| {
                    e.file_type().is_ok_and(|typ| typ.is_file())
                        && Path::new(&e.file_name()).extension() == Some("whl".as_ref())
                })
            })
            .with_context(|| format!("Did not find a .whl in {download_dir:?}"))?;

        util::archive::extract_zip(
            &debug_adapters_dir().join(Self::ADAPTER_NAME),
            File::open(&wheel_path.path()).await?,
        )
        .await?;

        Ok(Arc::from(wheel_path.path()))
    }

    async fn maybe_fetch_new_wheel(
        &self,
        toolchain: Option<Toolchain>,
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<()> {
        let latest_release = delegate
            .http_client()
            .get(
                "https://pypi.org/pypi/debugpy/json",
                AsyncBody::empty(),
                false,
            )
            .await
            .log_err();
        let response = latest_release
            .filter(|response| response.status().is_success())
            .context("getting latest release")?;

        let download_dir = debug_adapters_dir().join(Self::ADAPTER_NAME);
        std::fs::create_dir_all(&download_dir)?;

        let mut output = String::new();
        response.into_body().read_to_string(&mut output).await?;
        let as_json = serde_json::Value::from_str(&output)?;
        let latest_version = as_json
            .get("info")
            .and_then(|info| {
                info.get("version")
                    .and_then(|version| version.as_str())
                    .map(ToOwned::to_owned)
            })
            .context("parsing latest release information")?;
        let dist_info_dirname: OsString = format!("debugpy-{latest_version}.dist-info").into();
        let is_up_to_date = delegate
            .fs()
            .read_dir(&debug_adapters_dir().join(Self::ADAPTER_NAME))
            .await?
            .into_stream()
            .any(async |entry| {
                entry.is_ok_and(|e| e.file_name().is_some_and(|name| name == dist_info_dirname))
            })
            .await;

        if !is_up_to_date {
            delegate
                .fs()
                .remove_dir(
                    &debug_adapters_dir().join(Self::ADAPTER_NAME),
                    RemoveOptions {
                        recursive: true,
                        ignore_if_not_exists: true,
                    },
                )
                .await?;
            self.fetch_wheel(toolchain, delegate).await?;
        }
        anyhow::Ok(())
    }

    async fn fetch_debugpy_whl(
        &self,
        toolchain: Option<Toolchain>,
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<Arc<Path>, String> {
        self.debugpy_whl_base_path
            .get_or_init(|| async move {
                let adapter_path = debug_adapters_dir()
                    .join(Self::ADAPTER_NAME)
                    .join("debugpy")
                    .join("adapter");

                if let Err(error) = self.maybe_fetch_new_wheel(toolchain, delegate).await {
                    if delegate
                        .fs()
                        .metadata(&adapter_path)
                        .await
                        .is_ok_and(|m| m.is_some())
                    {
                        log::warn!(
                            "获取最新 debugpy 失败,使用缓存版本: {error:#}"
                        );
                    } else {
                        return Err(format!("{error}"));
                    }
                }

                Ok(Arc::from(adapter_path.as_ref()))
            })
            .await
            .clone()
    }

    async fn base_venv_path(
        &self,
        toolchain: Option<Toolchain>,
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<Arc<Path>> {
        let result = self.base_venv_path
            .get_or_init(|| async {
                let base_python = if let Some(toolchain) = toolchain {
                    toolchain.path.to_string()
                } else {
                    Self::system_python_name(delegate).await.ok_or_else(|| {
                        let mut message = "未找到 Python 安装".to_owned();
                        if cfg!(windows){
                            message.push_str("。请从 Microsoft Store 安装 Python,或手动从 https://www.python.org/downloads/windows/ 安装。")
                        }
                        message
                    })?
                };

                let debug_adapter_path = paths::debug_adapters_dir().join(Self::DEBUG_ADAPTER_NAME.as_ref());
                let output = util::command::new_command(&base_python)
                    .args(["-m", "venv", "zed_base_venv"])
                    .current_dir(
                        &debug_adapter_path,
                    )
                    .spawn()
                    .map_err(|e| format!("{e:#?}"))?
                    .output()
                    .await
                    .map_err(|e| format!("{e:#?}"))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let debug_adapter_path = debug_adapter_path.display();
                    return Err(format!("无法在以下位置使用 {base_python} 创建基础虚拟环境:\n{debug_adapter_path}\n标准错误:\n{stderr}\n标准输出:\n{stdout}\n"));
                }

                const PYTHON_PATH: &str = if cfg!(target_os = "windows") {
                    "Scripts/python.exe"
                } else {
                    "bin/python3"
                };
                Ok(Arc::from(
                    paths::debug_adapters_dir()
                        .join(Self::DEBUG_ADAPTER_NAME.as_ref())
                        .join("zed_base_venv")
                        .join(PYTHON_PATH)
                        .as_ref(),
                ))
            })
            .await
            .clone();
        match result {
            Ok(path) => Ok(path),
            Err(e) => Err(anyhow::anyhow!("{e}")),
        }
    }
    async fn system_python_name(delegate: &Arc<dyn DapDelegate>) -> Option<String> {
        const BINARY_NAMES: [&str; 3] = ["python3", "python", "py"];
        let mut name = None;

        for cmd in BINARY_NAMES {
            let Some(path) = delegate.which(OsStr::new(cmd)).await else {
                continue;
            };
            // Try to detect situations where `python3` exists but is not a real Python interpreter.
            // Notably, on fresh Windows installs, `python3` is a shim that opens the Microsoft Store app
            // when run with no arguments, and just fails otherwise.
            let Some(output) = new_command(&path)
                .args(["-c", "print(1 + 2)"])
                .output()
                .await
                .ok()
            else {
                continue;
            };
            if output.stdout.trim_ascii() != b"3" {
                continue;
            }
            name = Some(path.to_string_lossy().into_owned());
            break;
        }
        name
    }

    async fn get_installed_binary(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        config: &DebugTaskDefinition,
        user_installed_path: Option<PathBuf>,
        user_args: Option<Vec<String>>,
        user_env: Option<HashMap<String, String>>,
        python_from_toolchain: Option<String>,
    ) -> Result<DebugAdapterBinary> {
        let mut tcp_connection = config.tcp_connection.clone().unwrap_or_default();

        let (config_port, config_host) = config
            .config
            .get("connect")
            .map(|value| {
                (
                    value
                        .get("port")
                        .and_then(|val| val.as_u64().map(|p| p as u16)),
                    value.get("host").and_then(|val| val.as_str()),
                )
            })
            .unwrap_or_else(|| {
                (
                    config
                        .config
                        .get("port")
                        .and_then(|port| port.as_u64().map(|p| p as u16)),
                    config.config.get("host").and_then(|host| host.as_str()),
                )
            });

        let is_attach_with_connect = if config
            .config
            .get("request")
            .is_some_and(|val| val.as_str().is_some_and(|request| request == "attach"))
        {
            if tcp_connection.host.is_some() && config_host.is_some() {
                bail!("Cannot have two different hosts in debug configuration")
            } else if tcp_connection.port.is_some() && config_port.is_some() {
                bail!("Cannot have two different ports in debug configuration")
            }

            if let Some(hostname) = config_host {
                tcp_connection.host = Some(hostname.parse().context("无效的 IP 地址")?);
            }
            tcp_connection.port = config_port;
            DebugpyLaunchMode::AttachWithConnect { host: config_host }
        } else {
            DebugpyLaunchMode::Normal
        };

        let (host, port, timeout) = crate::configure_tcp_connection(tcp_connection).await?;

        let python_path = if let Some(toolchain) = python_from_toolchain {
            Some(toolchain)
        } else {
            Self::system_python_name(delegate).await
        };

        let python_command = python_path.context("failed to find binary path for Python")?;
        log::debug!("Using Python executable: {}", python_command);

        let arguments = Self::generate_debugpy_arguments(
            &host,
            port,
            is_attach_with_connect,
            user_installed_path.as_deref(),
            user_args,
        )
        .await?;

        log::debug!(
            "正在使用命令启动 debugpy 适配器:{} {}",
            python_command,
            arguments.join(" ")
        );

        Ok(DebugAdapterBinary {
            command: Some(python_command),
            arguments,
            connection: Some(adapters::TcpArguments {
                host,
                port,
                timeout,
            }),
            cwd: Some(delegate.worktree_root_path().to_path_buf()),
            envs: user_env.unwrap_or_default(),
            request_args: self.request_args(delegate, config).await?,
        })
    }
}

#[async_trait(?Send)]
impl DebugAdapter for PythonDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        Self::DEBUG_ADAPTER_NAME
    }

    fn adapter_language_name(&self) -> Option<LanguageName> {
        Some(SharedString::new_static("Python").into())
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut args = json!({
            "request": match zed_scenario.request {
                DebugRequest::Launch(_) => "launch",
                DebugRequest::Attach(_) => "attach",
            },
            "subProcess": true,
            "redirectOutput": true,
        });

        let map = args.as_object_mut().unwrap();
        match &zed_scenario.request {
            DebugRequest::Attach(attach) => {
                map.insert("processId".into(), attach.process_id.into());
            }
            DebugRequest::Launch(launch) => {
                map.insert("program".into(), launch.program.clone().into());
                map.insert("args".into(), launch.args.clone().into());
                if !launch.env.is_empty() {
                    map.insert("env".into(), launch.env_json());
                }

                if let Some(stop_on_entry) = zed_scenario.stop_on_entry {
                    map.insert("stopOnEntry".into(), stop_on_entry.into());
                }
                if let Some(cwd) = launch.cwd.as_ref() {
                    map.insert("cwd".into(), cwd.to_string_lossy().into_owned().into());
                }
            }
        }

        Ok(DebugScenario {
            adapter: zed_scenario.adapter,
            label: zed_scenario.label,
            config: args,
            build: None,
            tcp_connection: None,
        })
    }

    fn dap_schema(&self) -> serde_json::Value {
        json!({
            "properties": {
                "request": {
                    "type": "string",
                    "enum": ["attach", "launch"],
                    "description": "调试适配器请求类型"
                },
                "autoReload": {
                    "default": {},
                    "description": "配置编辑代码时自动重载。",
                    "properties": {
                        "enable": {
                            "default": false,
                            "description": "编辑代码时自动重载。",
                            "type": "boolean"
                        },
                        "exclude": {
                            "default": [
                                "**/.git/**",
                                "**/.metadata/**",
                                "**/__pycache__/**",
                                "**/node_modules/**",
                                "**/site-packages/**"
                            ],
                            "description": "排除自动重载的路径 Glob 模式。",
                            "items": {
                                "type": "string"
                            },
                            "type": "array"
                        },
                        "include": {
                            "default": [
                                "**/*.py",
                                "**/*.pyw"
                            ],
                            "description": "包含自动重载的路径 Glob 模式。",
                            "items": {
                                "type": "string"
                            },
                            "type": "array"
                        }
                    },
                    "type": "object"
                },
                "debugAdapterPath": {
                    "description": "Python 调试适配器可执行文件的(完整)路径。",
                    "type": "string"
                },
                "django": {
                    "default": false,
                    "description": "Django 调试。",
                    "type": "boolean"
                },
                "jinja": {
                    "default": null,
                    "description": "Jinja 模板调试(例如 Flask)。",
                    "enum": [
                        false,
                        null,
                        true
                    ]
                },
                "justMyCode": {
                    "default": true,
                    "description": "如果为 true,则仅显示和调试用户编写的代码。如果为 false,则显示和调试所有代码,包括库调用。",
                    "type": "boolean"
                },
                "logToFile": {
                    "default": false,
                    "description": "启用将调试器事件记录到日志文件。此文件可在 debugpy 扩展安装文件夹中找到。",
                    "type": "boolean"
                },
                "pathMappings": {
                    "default": [],
                    "items": {
                        "label": "路径映射",
                        "properties": {
                            "localRoot": {
                                "default": "${ZED_WORKTREE_ROOT}",
                                "label": "本地源码根目录。",
                                "type": "string"
                            },
                            "remoteRoot": {
                                "default": "",
                                "label": "远程源码根目录。",
                                "type": "string"
                            }
                        },
                        "required": [
                            "localRoot",
                            "remoteRoot"
                        ],
                        "type": "object"
                    },
                    "label": "路径映射。",
                    "type": "array"
                },
                "redirectOutput": {
                    "default": true,
                    "description": "重定向输出。",
                    "type": "boolean"
                },
                "showReturnValue": {
                    "default": true,
                    "description": "单步执行时显示函数的返回值。",
                    "type": "boolean"
                },
                "subProcess": {
                    "default": false,
                    "description": "是否启用子进程调试",
                    "type": "boolean"
                },
                "consoleName": {
                    "default": "Python 调试控制台",
                    "description": "调试控制台或终端的显示名称",
                    "type": "string"
                },
                "clientOS": {
                    "default": null,
                    "description": "VS Code 使用的操作系统。",
                    "enum": [
                        "windows",
                        null,
                        "unix"
                    ]
                }
            },
            "required": ["request"],
            "allOf": [
                {
                    "if": {
                        "properties": {
                            "request": {
                                "enum": ["attach"]
                            }
                        }
                    },
                    "then": {
                        "properties": {
                            "connect": {
                                "label": "通过套接字连接到 debugpy 进行附加。",
                                "properties": {
                                    "host": {
                                        "default": "127.0.0.1",
                                        "description": "要连接的主机名或 IP 地址。",
                                        "type": "string"
                                    },
                                    "port": {
                                        "description": "要连接的端口。",
                                        "type": [
                                            "number",
                                            "string"
                                        ]
                                    }
                                },
                                "required": [
                                    "port"
                                ],
                                "type": "object"
                            },
                            "listen": {
                                "label": "通过监听来自 debugpy 的传入套接字连接进行附加",
                                "properties": {
                                    "host": {
                                        "default": "127.0.0.1",
                                        "description": "要监听的接口的主机名或 IP 地址。",
                                        "type": "string"
                                    },
                                    "port": {
                                        "description": "要监听的端口。",
                                        "type": [
                                            "number",
                                            "string"
                                        ]
                                    }
                                },
                                "required": [
                                    "port"
                                ],
                                "type": "object"
                            },
                            "processId": {
                                "anyOf": [
                                    {
                                        "default": "${command:pickProcess}",
                                        "description": "使用进程选择器选择要附加的进程,或输入整数形式的进程 ID。",
                                        "enum": [
                                            "${command:pickProcess}"
                                        ]
                                    },
                                    {
                                        "description": "要附加到的本地进程的 ID。",
                                        "type": "integer"
                                    }
                                ]
                            }
                        }
                    }
                },
                {
                    "if": {
                        "properties": {
                            "request": {
                                "enum": ["launch"]
                            }
                        }
                    },
                    "then": {
                        "properties": {
                            "args": {
                                "default": [],
                                "description": "传递给程序的命令行参数。对于字符串类型参数,它将按原样通过 Shell 传递,因此所有 Shell 变量扩展都将适用。但对于数组类型,值将被 Shell 转义。",
                                "items": {
                                    "type": "string"
                                },
                                "anyOf": [
                                    {
                                        "default": "${command:pickArgs}",
                                        "enum": [
                                            "${command:pickArgs}"
                                        ]
                                    },
                                    {
                                        "type": [
                                            "array",
                                            "string"
                                        ]
                                    }
                                ]
                            },
                            "console": {
                                "default": "integratedTerminal",
                                "description": "在何处启动调试目标:内部控制台、集成终端或外部终端。",
                                "enum": [
                                    "externalTerminal",
                                    "integratedTerminal",
                                    "internalConsole"
                                ]
                            },
                            "cwd": {
                                "default": "${ZED_WORKTREE_ROOT}",
                                "description": "正在调试的程序的工作目录的绝对路径。默认为文件的根目录(留空)。",
                                "type": "string"
                            },
                            "autoStartBrowser": {
                                "default": false,
                                "description": "打开外部浏览器以启动应用程序",
                                "type": "boolean"
                            },
                            "env": {
                                "additionalProperties": {
                                    "type": "string"
                                },
                                "default": {},
                                "description": "定义为键值对的环境变量。属性名将成为环境变量名,属性值将成为环境变量的值。",
                                "type": "object"
                            },
                            "envFile": {
                                "default": "${ZED_WORKTREE_ROOT}/.env",
                                "description": "包含环境变量定义的文件的绝对路径。",
                                "type": "string"
                            },
                            "gevent": {
                                "default": false,
                                "description": "启用 gevent monkey-patched 代码的调试。",
                                "type": "boolean"
                            },
                            "module": {
                                "default": "",
                                "description": "要调试的模块的名称。",
                                "type": "string"
                            },
                            "program": {
                                "default": "${ZED_FILE}",
                                "description": "程序的绝对路径。",
                                "type": "string"
                            },
                            "purpose": {
                                "default": [],
                                "description": "告知扩展使用此配置进行测试调试,或在使用“在终端中调试”命令时使用。",
                                "items": {
                                    "enum": [
                                        "debug-test",
                                        "debug-in-terminal"
                                    ],
                                    "enumDescriptions": [
                                        "在使用测试视图或测试调试命令调试测试时使用此配置。",
                                        "在编辑器中使用“在终端中调试”按钮调试文件时使用此配置。"
                                    ]
                                },
                                "type": "array"
                            },
                            "pyramid": {
                                "default": false,
                                "description": "是否调试 Pyramid 应用程序。",
                                "type": "boolean"
                            },
                            "python": {
                                "default": "${command:python.interpreterPath}",
                                "description": "Python 解释器可执行文件的绝对路径;如果设置,则覆盖工作区配置。",
                                "type": "string"
                            },
                            "pythonArgs": {
                                "default": [],
                                "description": "传递给 Python 解释器的命令行参数。若要向调试目标传递参数,请使用 \"args\"。",
                                "items": {
                                    "type": "string"
                                },
                                "type": "array"
                            },
                            "stopOnEntry": {
                                "default": false,
                                "description": "启动后自动停止。",
                                "type": "boolean"
                            },
                            "sudo": {
                                "default": false,
                                "description": "以提升的权限运行调试程序(在 Unix 上)。",
                                "type": "boolean"
                            },
                            "guiEventLoop": {
                                "default": "matplotlib",
                                "description": "将要运行的 GUI 事件循环。可能的值:\"matplotlib\"、\"wx\"、\"qt\"、\"none\",或将被导入并运行的自定义函数。",
                                "type": "string"
                            }
                        }
                    }
                }
            ]
        })
    }

    async fn get_binary(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        config: &DebugTaskDefinition,
        user_installed_path: Option<PathBuf>,
        user_args: Option<Vec<String>>,
        user_env: Option<HashMap<String, String>>,
        cx: &mut AsyncApp,
    ) -> Result<DebugAdapterBinary> {
        if let Some(local_path) = &user_installed_path {
            log::debug!(
                "使用用户安装的 debugpy 适配器,路径:{}",
                local_path.display()
            );
            return self
                .get_installed_binary(
                    delegate,
                    config,
                    Some(local_path.clone()),
                    user_args,
                    user_env,
                    None,
                )
                .await;
        }

        let base_paths = ["cwd", "program", "module"]
            .into_iter()
            .filter_map(|key| {
                config.config.get(key).and_then(|cwd| {
                    RelPath::new(
                        cwd.as_str()
                            .map(Path::new)?
                            .strip_prefix(delegate.worktree_root_path())
                            .ok()?,
                        PathStyle::local(),
                    )
                    .ok()
                })
            })
            .chain(
                // While Debugpy's wiki saids absolute paths are required, but it actually supports relative paths when cwd is passed in.
                // (Which should always be the case because Zed defaults to the cwd worktree root)
                // So we want to check that these relative paths find toolchains as well. Otherwise, they won't be checked
                // because the strip prefix in the iteration above will return an error
                config
                    .config
                    .get("cwd")
                    .map(|_| {
                        ["program", "module"].into_iter().filter_map(|key| {
                            config.config.get(key).and_then(|value| {
                                let path = Path::new(value.as_str()?);
                                RelPath::new(path, PathStyle::local()).ok()
                            })
                        })
                    })
                    .into_iter()
                    .flatten(),
            )
            .chain([RelPath::empty().into()]);

        let mut toolchain = None;

        for base_path in base_paths {
            if let Some(found_toolchain) = delegate
                .toolchain_store()
                .active_toolchain(
                    delegate.worktree_id(),
                    base_path.into_arc(),
                    language::LanguageName::new_static(Self::LANGUAGE_NAME),
                    cx,
                )
                .await
            {
                toolchain = Some(found_toolchain);
                break;
            }
        }

        self.fetch_debugpy_whl(toolchain.clone(), delegate)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(toolchain) = &toolchain {
            return self
                .get_installed_binary(
                    delegate,
                    config,
                    None,
                    user_args,
                    user_env,
                    Some(toolchain.path.to_string()),
                )
                .await;
        }

        self.get_installed_binary(delegate, config, None, user_args, user_env, None)
            .await
    }

    fn label_for_child_session(&self, args: &StartDebuggingRequestArguments) -> Option<String> {
        let label = args
            .configuration
            .get("name")?
            .as_str()
            .filter(|label| !label.is_empty())?;
        Some(label.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use util::path;

    use super::*;
    use task::TcpArgumentsTemplate;

    #[gpui::test]
    async fn test_tcp_connection_conflict_with_connect_args() {
        let adapter = PythonDebugAdapter {
            base_venv_path: OnceCell::new(),
            debugpy_whl_base_path: OnceCell::new(),
        };

        let config_with_port_conflict = json!({
            "request": "attach",
            "connect": {
                "port": 5679
            }
        });

        let tcp_connection = TcpArgumentsTemplate {
            host: None,
            port: Some(5678),
            timeout: None,
        };

        let task_def = DebugTaskDefinition {
            label: "test".into(),
            adapter: PythonDebugAdapter::ADAPTER_NAME.into(),
            config: config_with_port_conflict,
            tcp_connection: Some(tcp_connection.clone()),
        };

        let result = adapter
            .get_installed_binary(
                &test_mocks::MockDelegate::new(),
                &task_def,
                None,
                None,
                None,
                Some("python3".to_string()),
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot have two different ports")
        );

        let host = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let config_with_host_conflict = json!({
            "request": "attach",
            "connect": {
                "host": "192.168.1.1",
                "port": 5678
            }
        });

        let tcp_connection_with_host = TcpArgumentsTemplate {
            host: Some(host),
            port: None,
            timeout: None,
        };

        let task_def_host = DebugTaskDefinition {
            label: "test".into(),
            adapter: PythonDebugAdapter::ADAPTER_NAME.into(),
            config: config_with_host_conflict,
            tcp_connection: Some(tcp_connection_with_host),
        };

        let result_host = adapter
            .get_installed_binary(
                &test_mocks::MockDelegate::new(),
                &task_def_host,
                None,
                None,
                None,
                Some("python3".to_string()),
            )
            .await;

        assert!(result_host.is_err());
        assert!(
            result_host
                .unwrap_err()
                .to_string()
                .contains("Cannot have two different hosts")
        );
    }

    #[gpui::test]
    async fn test_attach_with_connect_mode_generates_correct_arguments() {
        let host = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let port = 5678;

        let args_without_host = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::AttachWithConnect { host: None },
            None,
            None,
        )
        .await
        .unwrap();

        let expected_suffix = path!("debug_adapters/Debugpy/debugpy/adapter");
        assert!(args_without_host[0].ends_with(expected_suffix));
        assert_eq!(args_without_host[1], "connect");
        assert_eq!(args_without_host[2], "5678");

        let args_with_host = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::AttachWithConnect {
                host: Some("192.168.1.100"),
            },
            None,
            None,
        )
        .await
        .unwrap();

        assert!(args_with_host[0].ends_with(expected_suffix));
        assert_eq!(args_with_host[1], "connect");
        assert_eq!(args_with_host[2], "192.168.1.100:");
        assert_eq!(args_with_host[3], "5678");

        let args_normal = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::Normal,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(args_normal[0].ends_with(expected_suffix));
        assert_eq!(args_normal[1], "--host=127.0.0.1");
        assert_eq!(args_normal[2], "--port=5678");
        assert!(!args_normal.contains(&"connect".to_string()));
    }

    #[gpui::test]
    async fn test_debugpy_install_path_cases() {
        let host = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let port = 5678;

        // Case 1: User-defined debugpy path (highest precedence)
        let user_path = PathBuf::from("/custom/path/to/debugpy/src/debugpy/adapter");
        let user_args = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::Normal,
            Some(&user_path),
            None,
        )
        .await
        .unwrap();

        let venv_args = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::Normal,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(user_args[0], "/custom/path/to/debugpy/src/debugpy/adapter");
        assert_eq!(user_args[1], "--host=127.0.0.1");
        assert_eq!(user_args[2], "--port=5678");

        let expected_suffix = path!("debug_adapters/Debugpy/debugpy/adapter");
        assert!(venv_args[0].ends_with(expected_suffix));
        assert_eq!(venv_args[1], "--host=127.0.0.1");
        assert_eq!(venv_args[2], "--port=5678");

        // The same cases, with arguments overridden by the user
        let user_args = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::Normal,
            Some(&user_path),
            Some(vec!["foo".into()]),
        )
        .await
        .unwrap();
        let venv_args = PythonDebugAdapter::generate_debugpy_arguments(
            &host,
            port,
            DebugpyLaunchMode::Normal,
            None,
            Some(vec!["foo".into()]),
        )
        .await
        .unwrap();

        assert!(user_args[0].ends_with("src/debugpy/adapter"));
        assert_eq!(user_args[1], "foo");

        assert!(venv_args[0].ends_with(expected_suffix));
        assert_eq!(venv_args[1], "foo");

        // Note: Case 3 (GitHub-downloaded debugpy) is not tested since this requires mocking the Github API.
    }
}
