use super::util::{resolve_within, run_command};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct Run;

/// Build the probe-rs run command line (exposed for tests).
pub fn run_command_line(chip: &str, file: &str, probe: Option<&str>) -> String {
    let mut cmd = format!("probe-rs run --chip {chip}");
    if let Some(probe) = probe {
        cmd.push_str(&format!(" --probe {probe}"));
    }
    cmd.push_str(&format!(" \"{file}\""));
    cmd
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
            .map(|s| s.to_string())
            .or_else(|| ctx.default_chip.clone())
            .ok_or_else(|| {
                ToolError::new(
                    "[InvalidInput] missing chip: pass a chip parameter or set default_chip in \
                     [tools] of config.toml",
                )
            })?;
        let probe = args.get("probe").and_then(|p| p.as_str());
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

        let command = run_command_line(&chip, &resolved.to_string_lossy(), probe);
        match run_command(&command, &ctx.cwd, timeout_ms, None).await {
            Ok((text, Some(0))) => Ok(ToolOutput {
                text: format!("run finished (exit 0)\n{text}"),
            }),
            Ok((text, Some(code))) => Err(ToolError::new(format!(
                "[Io] run failed (exit {code})\n{text}"
            ))),
            Ok((text, None)) => Ok(ToolOutput {
                text: format!("run timed out after {timeout_ms} ms; captured output:\n{text}"),
            }),
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
        }
    }

    #[test]
    fn run_command_line_builds_probe_rs_invocation() {
        let cmd = run_command_line("stm32f407vetx", "target/out.elf", None);
        assert!(cmd.contains("probe-rs run --chip stm32f407vetx"));
        assert!(cmd.contains("target/out.elf"));
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
