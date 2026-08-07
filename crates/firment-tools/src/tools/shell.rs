use super::util::resolve;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct Shell;

/// Returns a short reason when the shell command looks destructive. Used to
/// warn in interactive approval popups and to hard-block in one-shot mode
/// unless `--allow-dangerous` was passed.
pub fn dangerous_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    let normalized = lower.replace('\\', "/").replace(['"', '\''], " ");
    let tokens: Vec<&str> = normalized
        .split([' ', '\t', '|', '&', ';', '>', '<', '\n', '\r'])
        .filter(|t| !t.is_empty())
        .collect();

    // Scan every token, not just the first: `cmd /c del ...`,
    // `cd . && del ...`, `powershell -Command Remove-Item ...`, and other
    // wrapper shapes must not slip past.
    for token in &tokens {
        let base = token
            .trim_start_matches(['.', '/', '\\'])
            .rsplit('/')
            .next()
            .unwrap_or(token);
        match base {
            "del" | "erase" | "rm" | "rmdir" | "rd" | "deltree" | "remove-item" | "ri"
            | "shred" | "unlink" => {
                return Some("delete/erase files or directories");
            }
            "mv" | "move" | "move-item" | "mi" | "ren" | "rename" | "rename-item" | "rni" => {
                return Some("move/rename files (changes workspace layout)");
            }
            "format" | "diskpart" | "shutdown" | "taskkill" | "wipe" => {
                return Some("destructive system operation");
            }
            "reg" if tokens.contains(&"delete") => {
                return Some("delete registry keys");
            }
            "mkfs" | "mkfs.ext2" | "mkfs.ext3" | "mkfs.ext4" | "mkfs.xfs" | "mkfs.btrfs" => {
                return Some("format a filesystem");
            }
            _ => {}
        }
    }

    // Scripting-API deletions: `python -c "os.remove(...)"` / `Path.unlink()`
    // / `shutil.rmtree(...)` etc. Quoting was normalized above, so tokens
    // still carry the call shape.
    for token in &tokens {
        if token.contains("os.remove")
            || token.contains("os.unlink")
            || token.contains("shutil.rmtree")
            || token.contains("shutil.move")
            || token.contains("os.rename")
            || token.contains("os.replace")
            || token.contains("unlink(")
            || token.contains("rmtree(")
            || token.contains("remove(")
            || token.contains("rename(")
            || token.contains("move(")
        {
            return Some("delete/move via scripting API");
        }
    }

    let joined = tokens.join(" ");
    if joined.contains("git clean") {
        Some("git clean (discard untracked files)")
    } else if joined.contains("git reset") && joined.contains("--hard") {
        Some("git reset --hard (discard local changes)")
    } else if joined.contains("git push")
        && (joined.contains("--force") || joined.contains(" -f ") || joined.ends_with(" -f"))
    {
        Some("git force push (overwrite remote history)")
    } else {
        None
    }
}

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
        let command = args.get("command").and_then(|c| c.as_str())?;
        Some(match dangerous_reason(command) {
            Some(reason) => format!("⚠ 危险命令（{reason}）: {command}"),
            None => format!("run shell command: {command}"),
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|c| c.as_str())
            .ok_or_else(|| ToolError::new("missing 'command'"))?;
        if let Some(reason) = dangerous_reason(command)
            && !ctx.allow_dangerous
        {
            return Err(ToolError::new(format!(
                "危险命令已被安全闸拦截（{reason}）: {command}\n\
                 当前是一次性/自动批准模式，默认不允许破坏性或改变布局的操作。\n\
                 请改用更安全的操作；如确需执行，请用户加 --allow-dangerous 重新运行，\
                 或先在交互式 TUI 中确认。\n\
                 汇报前请先用 git status / list_dir 核实工作区实际状态，不要声称操作被完全拦截。"
            )));
        }
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
                        "command: {command}\ntimed out after {timeout_ms} ms and was killed (child processes may survive)"
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
                "command: {command}\nexit code: {status}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::dangerous_reason;
    use crate::tools::shell::Shell;
    use firment_core::{AutoApprove, Tool, ToolContext};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn detects_destructive_commands() {
        for cmd in [
            "del todo.py test_todo.py",
            "del /q /f *.py",
            "rm -rf src",
            "rm todo.py",
            "Remove-Item -Recurse -Force .",
            "rd /s /q build",
            "git clean -fdx",
            "git reset --hard HEAD",
            "git push --force origin main",
            "format C:",
            "taskkill /f /im app.exe",
            "cmd /c del todo.py test_todo.py",
            "cd . && del todo.py",
            "powershell -Command Remove-Item -Recurse -Force .",
            "pwsh -c 'rm -rf src'",
            "echo y | del /q *.py",
            "python -c \"import os; os.remove('todo.py')\"",
            "python -c \"import shutil; shutil.rmtree('src')\"",
            "git rm todo.py",
            "shred -u todo.py",
            "mkdir _backup && move todo.py _backup\\todo.py",
            "move *.py _backup\\",
            "mv todo.py test_todo.py _backup/",
            "Move-Item -Path *.py -Destination _backup",
            "ren todo.py old.py",
            "git mv todo.py src/todo.py",
            "python -c \"import shutil; shutil.move('todo.py', '_backup/')\"",
        ] {
            assert!(
                dangerous_reason(cmd).is_some(),
                "should detect dangerous: {cmd}"
            );
        }
    }

    #[test]
    fn allows_safe_commands() {
        for cmd in [
            "python -m pytest test_todo.py -q",
            "git status",
            "git diff",
            "cargo test",
            "pip install pytest",
            "cat todo.py",
            "cargo remove serde",
            "python -c \"print('hello')\"",
            "Get-ChildItem *.py",
            "copy todo.py todo.bak",
            "mkdir build",
        ] {
            assert!(dangerous_reason(cmd).is_none(), "should allow safe: {cmd}");
        }
    }

    #[tokio::test]
    async fn hard_guard_blocks_destructive_command_in_one_shot() {
        let tool = Shell;
        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
        };
        let result = tool
            .run(json!({"command": "del todo.py test_todo.py"}), &ctx)
            .await;
        let err = result.unwrap_err();
        assert!(err.message.contains("危险命令已被安全闸拦截"));
    }

    #[tokio::test]
    async fn approval_labels_dangerous_commands() {
        let tool = Shell;
        let reason = tool
            .approval(&json!({"command": "git clean -fdx"}))
            .unwrap();
        assert!(reason.contains("⚠ 危险命令"));
        let safe = tool.approval(&json!({"command": "git status"})).unwrap();
        assert!(!safe.contains("⚠"));
    }
}
