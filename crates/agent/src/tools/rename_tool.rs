use std::fmt::Write;
use std::sync::Arc;

use agent_client_protocol::schema as acp;
use collections::HashSet;
use gpui::{App, Entity, SharedString, Task};
use project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::symbol_locator::SymbolLocator;
use crate::{AgentTool, ToolCallEventStream, ToolInput};

/// Renames a symbol across the project using the language server.
///
/// This performs a semantic rename, updating all references to the symbol across all files in the project. The language server determines which occurrences to rename based on the symbol's type and scope.
///
/// Before using this tool, use read_file or grep to find the exact symbol name and line number.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RenameToolInput {
    /// The symbol to rename.
    pub symbol: SymbolLocator,

    /// The new name for the symbol.
    pub new_name: String,
}

pub struct RenameTool {
    project: Entity<Project>,
}

impl RenameTool {
    pub fn new(project: Entity<Project>) -> Self {
        Self { project }
    }
}

impl AgentTool for RenameTool {
    type Input = RenameToolInput;
    type Output = String;

    const NAME: &'static str = "rename_symbol";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        if let Ok(input) = input {
            format!(
                "将 `{}` 重命名为 `{}`",
                input.symbol.symbol_name, input.new_name
            )
            .into()
        } else {
            "重命名符号".into()
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<String, String>> {
        let project = self.project.clone();
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|e| format!("接收工具输入失败: {e}"))?;

            let resolved = input.symbol.resolve(&project, cx).await?;

            let rename_task = project.update(cx, |project, cx| {
                project.perform_rename(
                    resolved.buffer.clone(),
                    resolved.position,
                    input.new_name.clone(),
                    cx,
                )
            });

            let transaction = rename_task
                .await
                .map_err(|e| format!("重命名失败: {e}"))?;

            if transaction.0.is_empty() {
                return Ok(format!(
                    "未做任何更改。语言服务器无法重命名 '{}'。",
                    input.symbol.symbol_name
                ));
            }

            let buffers = transaction.0.keys().cloned().collect::<HashSet<_>>();
            project
                .update(cx, |project, cx| project.save_buffers(buffers, cx))
                .await
                .map_err(|e| format!("重命名成功,但无法保存重命名后的文件:{e}"))?;

            let mut output = format!(
                "已在 {} 个文件中将 `{}` 重命名为 `{}`:\n",
                input.symbol.symbol_name,
                input.new_name,
                transaction.0.len()
            );

            for (buffer, _) in &transaction.0 {
                buffer.read_with(cx, |buffer, cx| {
                    let path = buffer
                        .file()
                        .map(|f| f.full_path(cx).display().to_string())
                        .unwrap_or_else(|| "<无标题>".to_string());
                    writeln!(output, "- {path}").ok();
                });
            }

            Ok(output)
        })
    }
}
