use super::shell::dangerous_reason;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct Build;

#[async_trait]
impl Tool for Build {
    fn name(&self) -> &'static str {
        "build"
    }

    fn description(&self) -> &'static str {
        "Run the project's configured build command (config [tools] build_command, e.g. \"cmake --build build\" or Keil/IAR CLI). A non-zero exit means the build failed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 600000}
            }
        })
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("run configured build command".to_string())
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = ctx.build_command.clone().ok_or_else(|| {
            ToolError::new(
                "[InvalidInput] build 工具未配置：请在 config.toml 的 [tools] 中设置 build_command（例如 build_command = \"cmake --build build\"）",
            )
        })?;
        if let Some(reason) = dangerous_reason(&command)
            && !ctx.allow_dangerous
        {
            return Err(ToolError::new(format!(
                "[Permission] build 命令命中危险命令安全闸（{reason}），已拒绝执行: {command}"
            )));
        }
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(600_000);
        let (text, code) = super::util::run_command(&command, &ctx.cwd, timeout_ms, None)
            .await
            .map_err(ToolError::new)?;
        match code {
            Some(0) => Ok(ToolOutput {
                text: format!("build passed (exit 0)\n{text}"),
            }),
            Some(code) => Err(ToolError::new(format!(
                "[CompileError] build failed (exit {code})\n{text}"
            ))),
            None => Err(ToolError::new(format!("[Timeout] build timed out\n{text}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx(dir: &Path, build_command: Option<&str>) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: build_command.map(|s| s.to_string()),
            default_chip: None,
            allowed_roots: Vec::new(),
        }
    }

    #[tokio::test]
    async fn unconfigured_build_is_an_error() {
        let dir = tempdir().unwrap();
        let err = Build
            .run(json!({}), &ctx(dir.path(), None))
            .await
            .unwrap_err();
        assert!(err.message.contains("build_command"));
    }

    #[tokio::test]
    async fn passing_build_returns_success() {
        let cmd = if cfg!(windows) {
            "cmd /c echo ok"
        } else {
            "echo ok"
        };
        let dir = tempdir().unwrap();
        let out = Build
            .run(json!({}), &ctx(dir.path(), Some(cmd)))
            .await
            .unwrap();
        assert!(out.text.contains("build passed (exit 0)"));
    }

    #[tokio::test]
    async fn failing_build_returns_compile_error() {
        let cmd = if cfg!(windows) {
            "cmd /c exit 2"
        } else {
            "exit 2"
        };
        let dir = tempdir().unwrap();
        let err = Build
            .run(json!({}), &ctx(dir.path(), Some(cmd)))
            .await
            .unwrap_err();
        assert!(err.message.contains("[CompileError]"));
    }
}
