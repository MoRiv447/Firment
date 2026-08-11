use super::shell::dangerous_reason;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct Verify;

#[async_trait]
impl Tool for Verify {
    fn name(&self) -> &'static str {
        "verify"
    }

    fn description(&self) -> &'static str {
        "Run the project's configured verification command (config [tools] verify_command, e.g. \"cargo check\"). Use it after code changes before declaring the task done; a non-zero exit means verification failed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 120000}
            }
        })
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("run configured verify command".to_string())
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = ctx.verify_command.clone().ok_or_else(|| {
            ToolError::new(
                "[InvalidInput] verify tool is not configured: set verify_command in [tools] of \
                 config.toml (e.g. verify_command = \"cargo check\")",
            )
        })?;
        if let Some(reason) = dangerous_reason(&command)
            && !ctx.allow_dangerous
        {
            return Err(ToolError::new(format!(
                "[Permission] verify command was blocked by the dangerous-command guard \
                 ({reason}); refusing to run: {command}"
            )));
        }
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(120_000);
        let (text, code) =
            super::util::run_command(&command, &ctx.cwd, timeout_ms, None, Some(&ctx.cancel))
                .await
                .map_err(ToolError::new)?;
        match code {
            Some(0) => Ok(ToolOutput {
                text: format!("verify passed (exit 0)\n{text}"),
            }),
            Some(code) => Err(ToolError::new(format!(
                "[CompileError] verify failed (exit {code})\n{text}"
            ))),
            None => Err(ToolError::new(format!(
                "[Timeout] verify timed out\n{text}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn ctx_with(command: Option<&str>) -> (ToolContext, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let ctx = ToolContext {
            cwd: dir.path().to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.path().join("undo")))),
            verify_command: command.map(|s| s.to_string()),
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        };
        (ctx, dir)
    }

    #[tokio::test]
    async fn unconfigured_verify_is_an_error() {
        let (ctx, _dir) = ctx_with(None);
        let err = Verify.run(json!({}), &ctx).await.unwrap_err();
        assert!(err.message.contains("verify_command"));
    }

    #[tokio::test]
    async fn passing_verify_returns_success() {
        let cmd = if cfg!(windows) {
            "cmd /c echo ok"
        } else {
            "echo ok"
        };
        let (ctx, _dir) = ctx_with(Some(cmd));
        let out = Verify.run(json!({}), &ctx).await.unwrap();
        assert!(out.text.contains("verify passed (exit 0)"));
    }

    #[tokio::test]
    async fn failing_verify_returns_error() {
        let cmd = if cfg!(windows) {
            "cmd /c exit 3"
        } else {
            "exit 3"
        };
        let (ctx, _dir) = ctx_with(Some(cmd));
        let err = Verify.run(json!({}), &ctx).await.unwrap_err();
        assert!(err.message.contains("verify failed (exit 3)"));
    }

    #[tokio::test]
    async fn dangerous_verify_command_is_blocked_without_allow_dangerous() {
        let cmd = if cfg!(windows) {
            "del dummy.txt"
        } else {
            "rm -rf dummy"
        };
        let (ctx, _dir) = ctx_with(Some(cmd));
        let err = Verify.run(json!({}), &ctx).await.unwrap_err();
        assert!(err.message.contains("dangerous-command guard"));
    }
}
