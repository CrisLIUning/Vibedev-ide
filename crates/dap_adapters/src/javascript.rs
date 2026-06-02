use adapters::latest_github_release;
use anyhow::Context as _;
use collections::HashMap;
use dap::{StartDebuggingRequestArguments, adapters::DebugTaskDefinition};
use gpui::AsyncApp;
use serde_json::Value;
use std::{path::PathBuf, sync::OnceLock};
use task::DebugRequest;
use util::{ResultExt, maybe, shell::ShellKind};

use crate::*;

#[derive(Debug, Default)]
pub(crate) struct JsDebugAdapter {
    checked: OnceLock<()>,
}

impl JsDebugAdapter {
    const ADAPTER_NAME: &'static str = "JavaScript";
    const ADAPTER_NPM_NAME: &'static str = "vscode-js-debug";
    const ADAPTER_PATH: &'static str = "js-debug/src/dapDebugServer.js";

    async fn fetch_latest_adapter_version(
        &self,
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<AdapterVersion> {
        let release = latest_github_release(
            &format!("microsoft/{}", Self::ADAPTER_NPM_NAME),
            true,
            false,
            delegate.http_client(),
        )
        .await?;

        let asset_name = format!("js-debug-dap-{}.tar.gz", release.tag_name);

        Ok(AdapterVersion {
            tag_name: release.tag_name,
            url: release
                .assets
                .iter()
                .find(|asset| asset.name == asset_name)
                .with_context(|| format!("no asset found matching {asset_name:?}"))?
                .browser_download_url
                .clone(),
        })
    }

    async fn get_installed_binary(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        task_definition: &DebugTaskDefinition,
        user_installed_path: Option<PathBuf>,
        user_args: Option<Vec<String>>,
        user_env: Option<HashMap<String, String>>,
        _: &mut AsyncApp,
    ) -> Result<DebugAdapterBinary> {
        let tcp_connection = task_definition.tcp_connection.clone().unwrap_or_default();
        let (host, port, timeout) = crate::configure_tcp_connection(tcp_connection).await?;

        let mut envs = user_env.unwrap_or_default();

        let mut configuration = task_definition.config.clone();
        if let Some(configuration) = configuration.as_object_mut() {
            maybe!({
                configuration
                    .get("type")
                    .filter(|value| value == &"node-terminal")?;
                let command = configuration.get("command")?.as_str()?.to_owned();
                let mut args = ShellKind::Posix.split(&command)?.into_iter();
                let program = args.next()?;
                configuration.insert("runtimeExecutable".to_owned(), program.into());
                configuration.insert(
                    "runtimeArgs".to_owned(),
                    args.map(Value::from).collect::<Vec<_>>().into(),
                );
                configuration.insert("console".to_owned(), "externalTerminal".into());
                Some(())
            });

            configuration.entry("type").and_modify(normalize_task_type);

            if let Some(program) = configuration
                .get("program")
                .cloned()
                .and_then(|value| value.as_str().map(str::to_owned))
            {
                match program.as_str() {
                    "npm" | "pnpm" | "yarn" | "bun"
                        if !configuration.contains_key("runtimeExecutable")
                            && !configuration.contains_key("runtimeArgs") =>
                    {
                        configuration.remove("program");
                        configuration.insert("runtimeExecutable".to_owned(), program.into());
                        if let Some(args) = configuration.remove("args") {
                            configuration.insert("runtimeArgs".to_owned(), args);
                        }
                    }
                    _ => {}
                }
            }

            if let Some(env) = configuration.get("env").cloned()
                && let Ok(env) = serde_json::from_value::<HashMap<String, String>>(env)
            {
                envs.extend(env);
            }

            configuration
                .entry("cwd")
                .or_insert(delegate.worktree_root_path().to_string_lossy().into());

            configuration
                .entry("console")
                .or_insert("externalTerminal".into());

            configuration.entry("sourceMaps").or_insert(true.into());
            configuration
                .entry("pauseForSourceMap")
                .or_insert(true.into());
            configuration
                .entry("sourceMapRenames")
                .or_insert(true.into());

            // Set up remote browser debugging
            if delegate.is_headless() {
                configuration
                    .entry("browserLaunchLocation")
                    .or_insert("ui".into());
            }
        }

        let adapter_path = if let Some(user_installed_path) = user_installed_path {
            user_installed_path
        } else {
            let adapter_path = paths::debug_adapters_dir().join(self.name().as_ref());

            let file_name_prefix = format!("{}_", self.name());

            util::fs::find_file_name_in_dir(adapter_path.as_path(), |file_name| {
                file_name.starts_with(&file_name_prefix)
            })
            .await
            .context("Couldn't find JavaScript dap directory")?
            .join(Self::ADAPTER_PATH)
        };

        let arguments = if let Some(mut args) = user_args {
            args.insert(0, adapter_path.to_string_lossy().into_owned());
            args
        } else {
            vec![
                adapter_path.to_string_lossy().into_owned(),
                port.to_string(),
                host.to_string(),
            ]
        };

        Ok(DebugAdapterBinary {
            command: Some(
                delegate
                    .node_runtime()
                    .binary_path()
                    .await?
                    .to_string_lossy()
                    .into_owned(),
            ),
            arguments,
            cwd: Some(delegate.worktree_root_path().to_path_buf()),
            envs,
            connection: Some(adapters::TcpArguments {
                host,
                port,
                timeout,
            }),
            request_args: StartDebuggingRequestArguments {
                configuration,
                request: self.request_kind(&task_definition.config).await?,
            },
        })
    }
}

#[async_trait(?Send)]
impl DebugAdapter for JsDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        DebugAdapterName(Self::ADAPTER_NAME.into())
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut args = json!({
            "type": "pwa-node",
            "request": match zed_scenario.request {
                DebugRequest::Launch(_) => "launch",
                DebugRequest::Attach(_) => "attach",
            },
        });

        let map = args.as_object_mut().unwrap();
        match &zed_scenario.request {
            DebugRequest::Attach(attach) => {
                map.insert("processId".into(), attach.process_id.into());
            }
            DebugRequest::Launch(launch) => {
                if launch.program.starts_with("http://") {
                    map.insert("url".into(), launch.program.clone().into());
                } else {
                    map.insert("program".into(), launch.program.clone().into());
                }

                if !launch.args.is_empty() {
                    map.insert("args".into(), launch.args.clone().into());
                }
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
        };

        Ok(DebugScenario {
            adapter: zed_scenario.adapter,
            label: zed_scenario.label,
            build: None,
            config: args,
            tcp_connection: None,
        })
    }

    fn dap_schema(&self) -> serde_json::Value {
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
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": ["pwa-node", "node", "chrome", "pwa-chrome", "msedge", "pwa-msedge", "node-terminal"],
                                    "description": "调试会话的类型",
                                    "default": "pwa-node"
                                },
                                "program": {
                                    "type": "string",
                                    "description": "要调试的程序或文件的路径"
                                },
                                "cwd": {
                                    "type": "string",
                                    "description": "被调试程序的工作目录的绝对路径"
                                },
                                "args": {
                                    "type": ["array", "string"],
                                    "description": "传递给程序的命令行参数",
                                    "items": {
                                        "type": "string"
                                    },
                                    "default": []
                                },
                                "env": {
                                    "type": "object",
                                    "description": "传递给程序的环境变量",
                                    "default": {}
                                },
                                "envFile": {
                                    "type": ["string", "array"],
                                    "description": "包含环境变量定义的文件路径",
                                    "items": {
                                        "type": "string"
                                    }
                                },
                                "stopOnEntry": {
                                    "type": "boolean",
                                    "description": "启动后自动暂停程序",
                                    "default": false
                                },
                                "attachSimplePort": {
                                    "type": "number",
                                    "description": "如果设置,将通过指定端口附加到进程。这对于 Node.js 程序通常不再需要,并且会失去调试子进程的能力,但在某些特殊场景(如 Deno 和 Docker 启动)中可能有用。如果设置为 0,将自动选择一个随机端口,并在启动参数中添加 --inspect-brk。"
                                },
                                "runtimeExecutable": {
                                    "type": ["string", "null"],
                                    "description": "要使用的运行时,可以是绝对路径或 PATH 中可用的运行时名称",
                                    "default": "node"
                                },
                                "runtimeArgs": {
                                    "type": ["array", "null"],
                                    "description": "传递给运行时可执行文件的参数",
                                    "items": {
                                        "type": "string"
                                    },
                                    "default": []
                                },
                                "outFiles": {
                                    "type": "array",
                                    "description": "用于定位生成的 JavaScript 文件的 Glob 模式",
                                    "items": {
                                        "type": "string"
                                    },
                                    "default": ["${ZED_WORKTREE_ROOT}/**/*.js", "!**/node_modules/**"]
                                },
                                "sourceMaps": {
                                    "type": "boolean",
                                    "description": "如果存在 JavaScript 源映射则使用",
                                    "default": true
                                },
                                "pauseForSourceMap": {
                                    "type": "boolean",
                                    "description": "在设置断点之前等待源映射加载完成。",
                                    "default": true
                                },
                                "sourceMapRenames": {
                                    "type": "boolean",
                                    "description": "是否在源映射中使用“names”映射。",
                                    "default": true
                                },
                                "sourceMapPathOverrides": {
                                    "type": "object",
                                    "description": "将源映射中指定的源文件位置重写为磁盘上的实际位置",
                                    "default": {}
                                },
                                "restart": {
                                    "type": ["boolean", "object"],
                                    "description": "Node.js 终止后重启会话",
                                    "default": false
                                },
                                "trace": {
                                    "type": ["boolean", "object"],
                                    "description": "启用调试适配器的日志记录",
                                    "default": false
                                },
                                "console": {
                                    "type": "string",
                                    "enum": ["internalConsole", "integratedTerminal"],
                                    "description": "在何处启动调试目标",
                                    "default": "internalConsole"
                                },
                                // Browser-specific
                                "url": {
                                    "type": ["string", "null"],
                                    "description": "将导航到此 URL 并附加到它(浏览器调试)"
                                },
                                "webRoot": {
                                    "type": "string",
                                    "description": "Web 服务器根目录在工作区的绝对路径",
                                    "default": "${ZED_WORKTREE_ROOT}"
                                },
                                "userDataDir": {
                                    "type": ["string", "boolean"],
                                    "description": "自定义 Chrome 用户配置文件的路径(浏览器调试)",
                                    "default": true
                                },
                                "skipFiles": {
                                    "type": "array",
                                    "description": "调试时要跳过的文件的 Glob 模式数组",
                                    "items": {
                                        "type": "string"
                                    },
                                    "default": ["<node_internals>/**"]
                                },
                                "timeout": {
                                    "type": "number",
                                    "description": "连接调试适配器的重试毫秒数",
                                    "default": 10000
                                },
                                "resolveSourceMapLocations": {
                                    "type": ["array", "null"],
                                    "description": "用于源映射解析的 minimatch 模式列表",
                                    "items": {
                                        "type": "string"
                                    }
                                }
                            },
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
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": ["pwa-node", "node", "chrome", "pwa-chrome", "edge", "pwa-edge"],
                                    "description": "调试会话的类型",
                                    "default": "pwa-node"
                                },
                                "processId": {
                                    "type": ["string", "number"],
                                    "description": "要附加到的进程 ID(Node.js 调试)"
                                },
                                "port": {
                                    "type": "number",
                                    "description": "要附加到的调试端口",
                                    "default": 9229
                                },
                                "address": {
                                    "type": "string",
                                    "description": "要调试的进程的 TCP/IP 地址",
                                    "default": "localhost"
                                },
                                "restart": {
                                    "type": ["boolean", "object"],
                                    "description": "Node.js 终止后重启会话",
                                    "default": false
                                },
                                "sourceMaps": {
                                    "type": "boolean",
                                    "description": "如果存在 JavaScript 源映射则使用",
                                    "default": true
                                },
                                "sourceMapPathOverrides": {
                                    "type": "object",
                                    "description": "将源映射中指定的源文件位置重写为磁盘上的实际位置",
                                    "default": {}
                                },
                                "outFiles": {
                                    "type": "array",
                                    "description": "用于定位生成的 JavaScript 文件的 Glob 模式",
                                    "items": {
                                        "type": "string"
                                    },
                                    "default": ["${ZED_WORKTREE_ROOT}/**/*.js", "!**/node_modules/**"]
                                },
                                "url": {
                                    "type": "string",
                                    "description": "将搜索具有此 URL 的页面并附加到它(浏览器调试)"
                                },
                                "webRoot": {
                                    "type": "string",
                                    "description": "Web 服务器根目录在工作区的绝对路径",
                                    "default": "${ZED_WORKTREE_ROOT}"
                                },
                                "skipFiles": {
                                    "type": "array",
                                    "description": "调试时要跳过的文件的 Glob 模式数组",
                                    "items": {
                                        "type": "string"
                                    },
                                    "default": ["<node_internals>/**"]
                                },
                                "timeout": {
                                    "type": "number",
                                    "description": "连接调试适配器的重试毫秒数",
                                    "default": 10000
                                },
                                "resolveSourceMapLocations": {
                                    "type": ["array", "null"],
                                    "description": "用于源映射解析的 minimatch 模式列表",
                                    "items": {
                                        "type": "string"
                                    }
                                },
                                "remoteRoot": {
                                    "type": ["string", "null"],
                                    "description": "包含程序的远程目录路径"
                                },
                                "localRoot": {
                                    "type": ["string", "null"],
                                    "description": "包含程序的本地目录路径"
                                }
                            },
                            "oneOf": [
                                { "required": ["processId"] },
                                { "required": ["port"] }
                            ]
                        }
                    ]
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
        if self.checked.set(()).is_ok() {
            delegate.output_to_console(format!("正在检查 {} 的最新版本...", self.name()));
            if let Some(version) = self.fetch_latest_adapter_version(delegate).await.log_err() {
                adapters::download_adapter_from_github(
                    self.name(),
                    version,
                    adapters::DownloadedFileType::GzipTar,
                    delegate.as_ref(),
                )
                .await?;
            } else {
                delegate.output_to_console(format!("{} 调试适配器已是最新版本", self.name()));
            }
        }

        self.get_installed_binary(
            delegate,
            config,
            user_installed_path,
            user_args,
            user_env,
            cx,
        )
        .await
    }

    fn label_for_child_session(&self, args: &StartDebuggingRequestArguments) -> Option<String> {
        let label = args
            .configuration
            .get("name")?
            .as_str()
            .filter(|name| !name.is_empty())?;
        Some(label.to_owned())
    }

    fn compact_child_session(&self) -> bool {
        true
    }

    fn prefer_thread_name(&self) -> bool {
        true
    }
}

fn normalize_task_type(task_type: &mut Value) {
    let Some(task_type_str) = task_type.as_str() else {
        return;
    };

    let new_name = match task_type_str {
        "node" | "pwa-node" | "node-terminal" => "pwa-node",
        "chrome" | "pwa-chrome" => "pwa-chrome",
        "edge" | "msedge" | "pwa-edge" | "pwa-msedge" => "pwa-msedge",
        _ => task_type_str,
    }
    .to_owned();

    *task_type = Value::String(new_name);
}
