use std::{env::consts, path::PathBuf, sync::OnceLock};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use collections::HashMap;
use dap::adapters::{DebugTaskDefinition, latest_github_release};
use futures::StreamExt;
use gpui::AsyncApp;
use serde_json::Value;
use task::{DebugRequest, DebugScenario, ZedDebugConfig};
use util::fs::remove_matching;

use crate::*;

#[derive(Default)]
pub(crate) struct CodeLldbDebugAdapter {
    path_to_codelldb: OnceLock<String>,
}

impl CodeLldbDebugAdapter {
    const ADAPTER_NAME: &'static str = "CodeLLDB";

    async fn request_args(
        &self,
        delegate: &Arc<dyn DapDelegate>,
        mut configuration: Value,
        label: &str,
    ) -> Result<dap::StartDebuggingRequestArguments> {
        let obj = configuration
            .as_object_mut()
            .context("CodeLLDB is not a valid json object")?;

        // CodeLLDB uses `name` for a terminal label.
        obj.entry("name")
            .or_insert(Value::String(String::from(label)));

        obj.entry("cwd")
            .or_insert(delegate.worktree_root_path().to_string_lossy().into());

        let request = self.request_kind(&configuration).await?;

        Ok(dap::StartDebuggingRequestArguments {
            request,
            configuration,
        })
    }

    async fn fetch_latest_adapter_version(
        &self,
        delegate: &Arc<dyn DapDelegate>,
    ) -> Result<AdapterVersion> {
        let release =
            latest_github_release("vadimcn/codelldb", true, false, delegate.http_client()).await?;

        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "x64",
            unsupported => {
                anyhow::bail!("unsupported architecture {unsupported}");
            }
        };
        let platform = match std::env::consts::OS {
            "macos" => "darwin",
            "linux" => "linux",
            "windows" => "win32",
            unsupported => {
                anyhow::bail!("unsupported operating system {unsupported}");
            }
        };
        let asset_name = format!("codelldb-{platform}-{arch}.vsix");
        let ret = AdapterVersion {
            tag_name: release.tag_name,
            url: release
                .assets
                .iter()
                .find(|asset| asset.name == asset_name)
                .with_context(|| format!("no asset found matching {asset_name:?}"))?
                .browser_download_url
                .clone(),
        };

        Ok(ret)
    }
}

#[async_trait(?Send)]
impl DebugAdapter for CodeLldbDebugAdapter {
    fn name(&self) -> DebugAdapterName {
        DebugAdapterName(Self::ADAPTER_NAME.into())
    }

    async fn config_from_zed_format(&self, zed_scenario: ZedDebugConfig) -> Result<DebugScenario> {
        let mut configuration = json!({
            "request": match zed_scenario.request {
                DebugRequest::Launch(_) => "launch",
                DebugRequest::Attach(_) => "attach",
            },
        });
        let map = configuration.as_object_mut().unwrap();
        // CodeLLDB uses `name` for a terminal label.
        map.insert(
            "name".into(),
            Value::String(String::from(zed_scenario.label.as_ref())),
        );
        match &zed_scenario.request {
            DebugRequest::Attach(attach) => {
                map.insert("pid".into(), attach.process_id.into());
            }
            DebugRequest::Launch(launch) => {
                map.insert("program".into(), launch.program.clone().into());

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
        }

        Ok(DebugScenario {
            adapter: zed_scenario.adapter,
            label: zed_scenario.label,
            config: configuration,
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
                "program": {
                    "type": "string",
                    "description": "要调试或附加的程序路径"
                },
                "args": {
                    "type": ["array", "string"],
                    "description": "程序参数"
                },
                "cwd": {
                    "type": "string",
                    "description": "程序工作目录"
                },
                "env": {
                    "type": "object",
                    "description": "额外的环境变量",
                    "patternProperties": {
                        ".*": {
                            "type": "string"
                        }
                    }
                },
                "envFile": {
                    "type": "string",
                    "description": "读取环境变量的文件"
                },
                "stdio": {
                    "type": ["null", "string", "array", "object"],
                    "description": "标准输入输出流的目标:null = 发送到调试器控制台或终端,\"<path>\" = 附加到文件/tty/fifo"
                },
                "terminal": {
                    "type": "string",
                    "enum": ["integrated", "console"],
                    "description": "要使用的终端类型",
                    "default": "integrated"
                },
                "console": {
                    "type": "string",
                    "enum": ["integratedTerminal", "internalConsole"],
                    "description": "要使用的终端类型 ('terminal' 的兼容性别名)"
                },
                "stopOnEntry": {
                    "type": "boolean",
                    "description": "启动后自动停止被调试程序",
                    "default": false
                },
                "initCommands": {
                    "type": "array",
                    "description": "调试器启动时执行的初始化命令",
                    "items": {
                        "type": "string"
                    }
                },
                "targetCreateCommands": {
                    "type": "array",
                    "description": "创建调试目标的命令",
                    "items": {
                        "type": "string"
                    }
                },
                "preRunCommands": {
                    "type": "array",
                    "description": "程序启动前执行的命令",
                    "items": {
                        "type": "string"
                    }
                },
                "processCreateCommands": {
                    "type": "array",
                    "description": "创建被调试进程的命令",
                    "items": {
                        "type": "string"
                    }
                },
                "postRunCommands": {
                    "type": "array",
                    "description": "程序启动后执行的命令",
                    "items": {
                        "type": "string"
                    }
                },
                "preTerminateCommands": {
                    "type": "array",
                    "description": "被调试程序终止或断开连接前执行的命令",
                    "items": {
                        "type": "string"
                    }
                },
                "exitCommands": {
                    "type": "array",
                    "description": "调试会话结束时执行的命令",
                    "items": {
                        "type": "string"
                    }
                },
                "expressions": {
                    "type": "string",
                    "enum": ["simple", "python", "native"],
                    "description": "表达式使用的默认求值器类型"
                },
                "sourceMap": {
                    "type": "object",
                    "description": "构建机器与本地机器之间的源路径重映射",
                    "patternProperties": {
                        ".*": {
                            "type": ["string", "null"]
                        }
                    }
                },
                "relativePathBase": {
                    "type": "string",
                    "description": "用于解析相对源路径的基础目录。默认为工作区文件夹"
                },
                "sourceLanguages": {
                    "type": "array",
                    "description": "启用特定语言功能的源语言列表",
                    "items": {
                        "type": "string"
                    }
                },
                "reverseDebugging": {
                    "type": "boolean",
                    "description": "启用反向调试",
                    "default": false
                },
                "breakpointMode": {
                    "type": "string",
                    "enum": ["path", "file"],
                    "description": "指定应如何设置源断点"
                },
                "pid": {
                    "type": ["integer", "string"],
                    "description": "要附加的进程 ID"
                },
                "waitFor": {
                    "type": "boolean",
                    "description": "等待进程启动 (仅限 MacOS)",
                    "default": false
                }
            },
            "required": ["request"],
            "allOf": [
                {
                    "if": {
                        "properties": {
                            "request": {
                                "enum": ["launch"]
                            }
                        }
                    },
                    "then": {
                        "oneOf": [
                            {
                                "required": ["program"]
                            },
                            {
                                "required": ["targetCreateCommands"]
                            }
                        ]
                    }
                },
                {
                    "if": {
                        "properties": {
                            "request": {
                                "enum": ["attach"]
                            }
                        }
                    },
                    "then": {
                        "oneOf": [
                            {
                                "required": ["pid"]
                            },
                            {
                                "required": ["program"]
                            }
                        ]
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
        _: &mut AsyncApp,
    ) -> Result<DebugAdapterBinary> {
        let mut command = user_installed_path
            .map(|p| p.to_string_lossy().into_owned())
            .or(self.path_to_codelldb.get().cloned());

        if command.is_none() {
            delegate.output_to_console(format!("正在检查 {} 的最新版本...", self.name()));
            let adapter_path = paths::debug_adapters_dir().join(&Self::ADAPTER_NAME);
            let version_path = match self.fetch_latest_adapter_version(delegate).await {
                Ok(version) => {
                    adapters::download_adapter_from_github(
                        self.name(),
                        version.clone(),
                        adapters::DownloadedFileType::Vsix,
                        delegate.as_ref(),
                    )
                    .await?;
                    let version_path =
                        adapter_path.join(format!("{}_{}", Self::ADAPTER_NAME, version.tag_name));
                    remove_matching(&adapter_path, |entry| entry != version_path).await;
                    version_path
                }
                Err(e) => {
                    delegate.output_to_console("无法获取最新版本".to_string());
                    log::error!("Error fetching latest version of {}: {}", self.name(), e);
                    delegate.output_to_console(format!(
                        "正在以下位置搜索适配器: {}",
                        adapter_path.display()
                    ));
                    let mut paths = delegate
                        .fs()
                        .read_dir(&adapter_path)
                        .await
                        .context("No cached adapter directory")?;
                    paths
                        .next()
                        .await
                        .context("No cached adapter found")?
                        .context("No cached adapter found")?
                }
            };
            let adapter_dir = version_path.join("extension").join("adapter");
            let path = adapter_dir
                .join(format!("codelldb{}", consts::EXE_SUFFIX))
                .to_string_lossy()
                .into_owned();
            self.path_to_codelldb.set(path.clone()).ok();
            command = Some(path);
        };
        let mut json_config = config.config.clone();

        // Auto-detect Rust projects and add sourceLanguages if not present.
        // This enables panic breakpoints to work correctly with CodeLLDB.
        if let Some(config_obj) = json_config.as_object_mut() {
            if !config_obj.contains_key("sourceLanguages") {
                // Check if this looks like a Rust binary (Cargo build output)
                if let Some(program) = config_obj.get("program").and_then(|p| p.as_str()) {
                    let path_str = program.replace('\\', "/");
                    if path_str.contains("/target/debug/") || path_str.contains("/target/release/")
                    {
                        config_obj.insert("sourceLanguages".to_owned(), json!(["rust"]));
                    }
                }
            }
        }

        Ok(DebugAdapterBinary {
            command: Some(command.unwrap()),
            cwd: Some(delegate.worktree_root_path().to_path_buf()),
            arguments: user_args.unwrap_or_else(|| {
                if let Some(config) = json_config.as_object_mut()
                    && let Some(source_languages) = config.get("sourceLanguages").filter(|value| {
                        value
                            .as_array()
                            .is_some_and(|array| array.iter().all(Value::is_string))
                    })
                {
                    let ret = vec![
                        "--settings".into(),
                        json!({"sourceLanguages": source_languages}).to_string(),
                    ];
                    config.remove("sourceLanguages");
                    ret
                } else {
                    vec![]
                }
            }),
            request_args: self
                .request_args(delegate, json_config, &config.label)
                .await?,
            envs: user_env.unwrap_or_default(),
            connection: None,
        })
    }
}
