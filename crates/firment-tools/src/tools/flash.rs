use super::util::{resolve_within, shell_quote, token_arg};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct Flash;

/// Build the probe-rs command line for flashing (exposed for tests). Arguments
/// are shell-quoted so hostile values cannot break out of the command line.
///
/// Note: newer probe-rs removed the `--format` flag (format is inferred from
/// the file extension, e.g. `.elf`/`.bin`), so it must not be passed.
pub fn flash_command(chip: &str, file: &str, probe: Option<&str>) -> String {
    let mut cmd = format!("probe-rs download --chip {}", shell_quote(chip));
    if let Some(probe) = probe {
        cmd.push_str(&format!(" --probe {}", shell_quote(probe)));
    }
    cmd.push_str(&format!(" {}", shell_quote(file)));
    cmd
}

/// Build the probe-rs reset command line (exposed for tests). Arguments are
/// shell-quoted so hostile values cannot break out of the command line.
pub fn reset_command(chip: &str, probe: Option<&str>) -> String {
    let mut cmd = format!("probe-rs reset --chip {}", shell_quote(chip));
    if let Some(probe) = probe {
        cmd.push_str(&format!(" --probe {}", shell_quote(probe)));
    }
    cmd
}

/// Executes a `probe-rs` subcommand directly with an explicit argument array
/// (no shell).
///
/// Running through `cmd /C <string>` is NOT used here: cmd.exe keeps the
/// quotes inside quoted arguments when the whole command is passed as one
/// string (e.g. `--chip "STM32G431RB"` arrives at probe-rs as `"STM32G431RB"`
/// including the quotes), which makes chip lookup fail with "chip not found".
/// Spawning with explicit args avoids quoting entirely and is equally safe.
async fn run_probe_rs(
    args: Vec<String>,
    cwd: &Path,
    timeout_ms: u64,
) -> Result<(String, Option<i32>), String> {
    let mut cmd = Command::new("probe-rs");
    cmd.args(&args)
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
    let read_streams = async {
        let _ = AsyncReadExt::read_to_end(&mut stdout, &mut out_buf).await;
        let _ = AsyncReadExt::read_to_end(&mut stderr, &mut err_buf).await;
    };
    let mut read_streams = Box::pin(read_streams);

    let status = tokio::select! {
        status = child.wait() => status,
        _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
            let _ = child.kill().await;
            return Err(format!(
                "[Timeout] probe-rs timed out after {timeout_ms} ms and was killed"
            ));
        }
    };
    let _ = (&mut read_streams).await;
    let code = status.map_err(|e| format!("wait failed: {e}"))?.code();

    let stdout = String::from_utf8_lossy(&out_buf).to_string();
    let stderr = String::from_utf8_lossy(&err_buf).to_string();
    Ok((format!("{stdout}{stderr}"), code))
}

#[async_trait]
impl Tool for Flash {
    fn name(&self) -> &'static str {
        "flash"
    }

    fn description(&self) -> &'static str {
        "Flash a firmware ELF to the target via probe-rs (ST-Link / J-Link / CMSIS-DAP / DFU). Requires `probe-rs` on PATH and a chip id (e.g. stm32f407vetx). Resets the target (restarts the firmware) after flashing by default."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {"type": "string", "description": "Path to the firmware ELF (must be inside the workspace)"},
                "chip": {"type": "string", "description": "probe-rs chip id, e.g. stm32f407vetx"},
                "probe": {"type": "string", "description": "Optional probe serial/id when multiple probes are attached"},
                "reset": {"type": "boolean", "default": true, "description": "Reset the target so the flashed firmware starts running (default true)"},
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 180000}
            },
            "required": ["file"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let file = args.get("file").and_then(|f| f.as_str()).unwrap_or("?");
        Some(format!("⚠ flash firmware to device: {file}"))
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
                    "[InvalidInput] missing chip: pass a chip parameter (e.g. stm32g431rb) or \
                     set default_chip in [tools] of config.toml. To find the right chip id: run \
                     `probe-rs chip list` and match the MCU family (e.g. an STM32G431RB is \
                     \"stm32g431rb\"); verify the board is connected with `probe-rs list` first.",
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
            .unwrap_or(180_000);
        let reset = args
            .get("reset")
            .and_then(|r| r.as_bool())
            .unwrap_or(true);

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

        let command = flash_command(&chip, &resolved.to_string_lossy(), probe.as_deref());

        let mut dl_args = vec!["download".to_string(), "--chip".to_string(), chip.clone()];
        if let Some(probe) = probe.as_ref() {
            dl_args.push("--probe".to_string());
            dl_args.push(probe.clone());
        }
        dl_args.push(resolved.to_string_lossy().to_string());

        let result = run_probe_rs(dl_args, &ctx.cwd, timeout_ms).await;
        match result {
            Ok((text, Some(0))) if !reset => Ok(ToolOutput {
                text: format!("flash passed (exit 0)\n{text}"),
            }),
            Ok((text, Some(0))) => {
                let reset_cmd = reset_command(&chip, probe.as_deref());
                let mut reset_args = vec!["reset".to_string(), "--chip".to_string(), chip];
                if let Some(probe) = probe {
                    reset_args.push("--probe".to_string());
                    reset_args.push(probe);
                }
                match run_probe_rs(reset_args, &ctx.cwd, timeout_ms).await {
                    Ok((rtext, Some(0))) => Ok(ToolOutput {
                        text: format!(
                            "flash passed and target reset (exit 0)\n{text}\nreset: {rtext}"
                        ),
                    }),
                    Ok((rtext, Some(code))) => Err(ToolError::new(format!(
                        "[Io] reset after flash failed (exit {code})\ncommand: {reset_cmd}\n{rtext}"
                    ))),
                    Ok((rtext, None)) => Err(ToolError::new(format!(
                        "[Timeout] reset after flash timed out\n{rtext}"
                    ))),
                    Err(e) if e.contains("spawn failed") => Err(ToolError::new(format!(
                        "[NotFound] probe-rs is not installed or not on PATH: install it with \
                         `cargo install probe-rs-tools` or download from the probe-rs GitHub \
                         Releases ({e})"
                    ))),
                    Err(e) => Err(ToolError::new(format!("[Io] {e}"))),
                }
            }
            Ok((text, Some(code))) => Err(ToolError::new(format!(
                "[Io] flash failed (exit {code})\ncommand: {command}\n{text}"
            ))),
            Ok((text, None)) => Err(ToolError::new(format!("[Timeout] flash timed out\n{text}"))),
            Err(e) if e.contains("spawn failed") => Err(ToolError::new(format!(
                "[NotFound] probe-rs is not installed or not on PATH: install it with \
                 `cargo install probe-rs-tools` or download from the probe-rs GitHub Releases ({e})"
            ))),
            Err(e) => Err(ToolError::new(format!("[Io] {e}"))),
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

    fn ctx(dir: &Path, default_chip: Option<&str>) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: default_chip.map(|s| s.to_string()),
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[test]
    fn flash_command_builds_probe_rs_line() {
        let cmd = flash_command("stm32f407vetx", "target/out.elf", Some("PROBE123"));
        assert!(cmd.contains("probe-rs download --chip"));
        assert!(cmd.contains("stm32f407vetx"));
        assert!(cmd.contains("PROBE123"));
        assert!(cmd.contains("target/out.elf"));
        assert!(cmd.contains(&shell_quote("stm32f407vetx")));
        assert!(cmd.contains(&shell_quote("PROBE123")));
    }

    #[test]
    fn flash_command_quotes_hostile_values() {
        let cmd = flash_command("x & whoami", "a'$(rm -rf /).elf", Some("p`p"));
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

    #[test]
    fn reset_command_builds_probe_rs_line() {
        let cmd = reset_command("stm32f407vetx", Some("PROBE123"));
        assert!(cmd.contains("probe-rs reset --chip"));
        assert!(cmd.contains("stm32f407vetx"));
        assert!(cmd.contains("PROBE123"));
        assert!(cmd.contains(&shell_quote("stm32f407vetx")));
        assert!(cmd.contains(&shell_quote("PROBE123")));
        let plain = reset_command("stm32f407vetx", None);
        assert!(plain.contains("probe-rs reset --chip"));
        assert!(!plain.contains("--probe"));
    }

    #[tokio::test]
    async fn missing_chip_is_an_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("out.elf"), b"x").unwrap();
        let err = Flash
            .run(json!({"file": "out.elf"}), &ctx(dir.path(), None))
            .await
            .unwrap_err();
        assert!(err.message.contains("default_chip"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn file_outside_workspace_is_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("evil.elf");
        std::fs::write(&outside, b"x").unwrap();
        let err = Flash
            .run(
                json!({"file": outside.to_string_lossy(), "chip": "stm32f407vetx"}),
                &ctx(dir.path(), None),
            )
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
            return; // probe-rs installed locally; skip this environment-specific check
        }
        let err = Flash
            .run(
                json!({"file": "out.elf", "chip": "stm32f407vetx"}),
                &ctx(dir.path(), None),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("probe-rs is not installed"),
            "got: {}",
            err.message
        );
    }
}
