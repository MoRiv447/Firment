use super::util::resolve;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct Shell;

#[async_trait]
impl Tool for Shell {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
        "Run a shell command in the workspace. Supports pipes and conditionals via the platform shell."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "cwd": {"type": "string", "description": "Working directory, defaults to workspace root"},
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 120000},
                "env": {"type": "object", "additionalProperties": {"type": "string"}}
            },
            "required": ["command"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        args.get("command")
            .and_then(|c| c.as_str())
            .map(|c| format!("run shell command: {c}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|c| c.as_str())
            .ok_or_else(|| ToolError::new("missing 'command'"))?;
        let cwd = args
            .get("cwd")
            .and_then(|c| c.as_str())
            .map(|c| resolve(&ctx.cwd, c))
            .unwrap_or_else(|| ctx.cwd.clone());
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(120_000);
        let env = args.get("env").and_then(|e| e.as_object());

        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        };
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(env) = env {
            for (k, v) in env {
                if let Some(v) = v.as_str() {
                    cmd.env(k, v);
                }
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::new(format!("spawn failed: {e}")))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::new("stdout handle unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolError::new("stderr handle unavailable"))?;
        let mut out_buf: Vec<u8> = Vec::new();
        let mut err_buf: Vec<u8> = Vec::new();
        let read_stdout = async { AsyncReadExt::read_to_end(&mut stdout, &mut out_buf).await };
        let read_stderr = async { AsyncReadExt::read_to_end(&mut stderr, &mut err_buf).await };
        let mut read_stdout = Box::pin(read_stdout);
        let mut read_stderr = Box::pin(read_stderr);

        let status = tokio::select! {
            status = child.wait() => status,
            _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = (&mut read_stdout).await;
                let _ = (&mut read_stderr).await;
                return Ok(ToolOutput {
                    text: format!(
                        "command timed out after {timeout_ms} ms and was killed (child processes may survive):\n{command}"
                    ),
                });
            }
        };
        let _ = (&mut read_stdout).await;
        let _ = (&mut read_stderr).await;

        let stdout = super::util::truncate(&String::from_utf8_lossy(&out_buf), 32_000);
        let stderr = super::util::truncate(&String::from_utf8_lossy(&err_buf), 32_000);
        let status = match status {
            Ok(s) => s,
            Err(e) => return Err(ToolError::new(format!("wait failed: {e}"))),
        };
        let status = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        Ok(ToolOutput {
            text: format!(
                "exit code: {status}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
            ),
        })
    }
}
