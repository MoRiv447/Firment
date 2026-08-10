use super::util::{resolve_within, run_command, shell_quote, token_arg};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct Flash;

/// Build the probe-rs command line for flashing (exposed for tests). Arguments
/// are shell-quoted so hostile values cannot break out of the command line.
pub fn flash_command(chip: &str, file: &str, probe: Option<&str>) -> String {
    let mut cmd = format!(
        "probe-rs download --chip {} --format elf",
        shell_quote(chip)
    );
    if let Some(probe) = probe {
        cmd.push_str(&format!(" --probe {}", shell_quote(probe)));
    }
    cmd.push_str(&format!(" {}", shell_quote(file)));
    cmd
}

#[async_trait]
impl Tool for Flash {
    fn name(&self) -> &'static str {
        "flash"
    }

    fn description(&self) -> &'static str {
        "Flash a firmware ELF to the target via probe-rs (ST-Link / J-Link / CMSIS-DAP / DFU). Requires `probe-rs` on PATH and a chip id (e.g. stm32f407vetx)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {"type": "string", "description": "Path to the firmware ELF (must be inside the workspace)"},
                "chip": {"type": "string", "description": "probe-rs chip id, e.g. stm32f407vetx"},
                "probe": {"type": "string", "description": "Optional probe serial/id when multiple probes are attached"},
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
                    "[InvalidInput] missing chip: pass a chip parameter or set default_chip in \
                     [tools] of config.toml (e.g. stm32f407vetx)",
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
        match run_command(&command, &ctx.cwd, timeout_ms, None).await {
            Ok((text, Some(0))) => Ok(ToolOutput {
                text: format!("flash passed (exit 0)\n{text}"),
            }),
            Ok((text, Some(code))) => Err(ToolError::new(format!(
                "[Io] flash failed (exit {code})\n{text}"
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
