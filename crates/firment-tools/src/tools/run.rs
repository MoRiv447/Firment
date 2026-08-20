use super::util::{probe_rs_err, resolve_within, shell_quote, token_arg};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct Run;

/// Build the probe-rs run command line (exposed for tests). Arguments are
/// shell-quoted so hostile values cannot break out of the command line.
pub fn run_command_line(chip: &str, file: &str, probe: Option<&str>) -> String {
    let mut cmd = format!("probe-rs run --chip {}", shell_quote(chip));
    if let Some(probe) = probe {
        cmd.push_str(&format!(" --probe {}", shell_quote(probe)));
    }
    cmd.push_str(&format!(" {}", shell_quote(file)));
    cmd
}

/// Executes `probe-rs run` with an argument array (no shell).
///
/// Like flash, running through `cmd /C <string>` would leak the quotes of
/// `--chip "STM32G431RB"` to probe-rs, making chip lookup fail. `probe-rs run`
/// is a long-lived process that streams RTT logs; we capture output until the
/// timeout elapses and then kill the tree, returning whatever was captured.
async fn run_probe_rs_run(
    chip: &str,
    file: &Path,
    probe: Option<&str>,
    cwd: &Path,
    timeout_ms: u64,
) -> Result<(String, Option<i32>), String> {
    let mut cmd = Command::new("probe-rs");
    cmd.arg("run").arg("--chip").arg(chip);
    if let Some(probe) = probe {
        cmd.arg("--probe").arg(probe);
    }
    cmd.arg(file)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout handle unavailable".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr handle unavailable".to_string())?;
    let mut out_buf: Vec<u8> = Vec::new();
    let mut err_buf: Vec<u8> = Vec::new();
    let mut drain_timed_out = false;
    let read_streams = async {
        drain_timed_out = tokio::time::timeout(Duration::from_secs(15), async {
            let _ = AsyncReadExt::read_to_end(&mut stdout, &mut out_buf).await;
            let _ = AsyncReadExt::read_to_end(&mut stderr, &mut err_buf).await;
        })
        .await
        .is_err();
    };
    let mut read_streams = Box::pin(read_streams);

    let status = if timeout_ms == 0 {
        Some(child.wait().await)
    } else {
        tokio::select! {
            status = child.wait() => Some(status),
            _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                None
            }
        }
    };
    let _ = (&mut read_streams).await;
    drop(read_streams);

    let mut text = String::from_utf8_lossy(&out_buf).to_string();
    text.push_str(&String::from_utf8_lossy(&err_buf));
    if drain_timed_out {
        text.push_str("\n[output truncated: still streaming after 15s]");
    }
    let code = match status {
        Some(s) => s.map_err(|e| format!("wait failed: {e}"))?.code(),
        None => None,
    };
    Ok((text, code))
}

#[async_trait]
impl Tool for Run {
    fn name(&self) -> &'static str {
        "run"
    }

    fn description(&self) -> &'static str {
        "Flash and run the firmware on the target via probe-rs, streaming RTT logs. Use a bounded timeout so you can observe the output."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {"type": "string", "description": "Path to the firmware ELF (must be inside the workspace)"},
                "chip": {"type": "string", "description": "probe-rs chip id, e.g. stm32f407vetx"},
                "probe": {"type": "string", "description": "Optional probe serial/id"},
                "timeout_ms": {"type": "integer", "minimum": 0, "default": 30000, "description": "0 = wait forever"}
            },
            "required": ["file"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let file = args.get("file").and_then(|f| f.as_str()).unwrap_or("?");
        Some(format!("⚠ flash and run target: {file}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let file = args
            .get("file")
            .and_then(|f| f.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'file'"))?;
        let resolved =
            resolve_within(&ctx.cwd, file, &ctx.allowed_roots).map_err(ToolError::new)?;
        let chip = args
            .get("chip")
            .and_then(|c| c.as_str())
            .map(|s| token_arg(s, "chip"))
            .transpose()
            .map_err(ToolError::new)?
            .or_else(|| ctx.default_chip.clone())
            .ok_or_else(|| {
                ToolError::new(
                    "[InvalidInput] missing chip: pass a chip parameter or set default_chip in \
                     [tools] of config.toml",
                )
            })?;
        let probe = args
            .get("probe")
            .and_then(|p| p.as_str())
            .map(|s| token_arg(s, "probe"))
            .transpose()
            .map_err(ToolError::new)?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(30_000);

        let probe_rs_ok = std::process::Command::new("probe-rs")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !probe_rs_ok {
            return Err(ToolError::new(
                "[NotFound] probe-rs is not installed or not on PATH: install it with \
                 `cargo install probe-rs-tools` or download from the probe-rs GitHub Releases",
            ));
        }

        let command = run_command_line(&chip, &resolved.to_string_lossy(), probe.as_deref());
        match run_probe_rs_run(&chip, &resolved, probe.as_deref(), &ctx.cwd, timeout_ms).await {
            Ok((text, Some(0))) => Ok(ToolOutput {
                text: format!("run finished (exit 0)\n{text}"),
            }),
            Ok((text, Some(code))) => Err(ToolError::new(format!(
                "[Io] run failed (exit {code})\ncommand: {command}\n{text}"
            ))),
            Ok((text, None)) => Ok(ToolOutput {
                text: format!("run timed out after {timeout_ms} ms; captured output:\n{text}"),
            }),
            Err(e) => Err(probe_rs_err(e)),
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

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: Some("stm32f407vetx".to_string()),
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[test]
    fn run_command_line_builds_probe_rs_invocation() {
        let cmd = run_command_line("stm32f407vetx", "target/out.elf", None);
        assert!(cmd.contains("probe-rs run --chip"));
        assert!(cmd.contains("stm32f407vetx"));
        assert!(cmd.contains("target/out.elf"));
        assert!(cmd.contains(&shell_quote("stm32f407vetx")));
    }

    #[test]
    fn run_command_line_quotes_hostile_values() {
        let cmd = run_command_line("x & whoami", "a'$(rm -rf /).elf", Some("p`p"));
        assert!(
            cmd.contains(&shell_quote("x & whoami")),
            "chip must stay a single token: {cmd}"
        );
        assert!(
            cmd.contains(&shell_quote("a'$(rm -rf /).elf")),
            "file must be shell-safe: {cmd}"
        );
        assert!(
            cmd.contains(&shell_quote("p`p")),
            "probe must stay a single token: {cmd}"
        );
    }

    #[tokio::test]
    async fn file_outside_workspace_is_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("evil.elf");
        std::fs::write(&outside, b"x").unwrap();
        let err = Run
            .run(json!({"file": outside.to_string_lossy()}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("outside the workspace"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn missing_probe_rs_gives_install_hint() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("out.elf"), b"x").unwrap();
        if std::process::Command::new("probe-rs")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return;
        }
        let err = Run
            .run(json!({"file": "out.elf"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("probe-rs is not installed"),
            "got: {}",
            err.message
        );
    }
}
