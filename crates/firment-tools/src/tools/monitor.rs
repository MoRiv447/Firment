use super::util::{resolve_within, truncate};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Monitor;

/// Blocking serial read loop: collect lines until the deadline, decoding hex
/// code addresses against an ELF when provided.
fn read_serial(port: &str, baud: u32, timeout_ms: u64, elf: Option<&Path>) -> Result<String, String> {
    use std::io::Read;
    let mut reader = serialport::new(port, baud)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| format!("failed to open serial port {port}: {e}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut buf = [0u8; 4096];
    let mut line_buf = String::new();
    let mut lines: Vec<String> = Vec::new();
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                for ch in String::from_utf8_lossy(&buf[..n]).chars() {
                    if ch == '\n' {
                        lines.push(crate::decode::decode_line(&line_buf, elf));
                        line_buf.clear();
                    } else if ch != '\r' {
                        line_buf.push(ch);
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue
            }
            Err(e) => return Err(format!("serial read failed: {e}")),
        }
    }
    if !line_buf.is_empty() {
        lines.push(crate::decode::decode_line(&line_buf, elf));
    }
    let text = lines.join("\n");
    if text.is_empty() {
        Ok(format!("no data received on {port} within {timeout_ms} ms"))
    } else {
        Ok(truncate(&text, 32_000))
    }
}

#[async_trait]
impl Tool for Monitor {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn description(&self) -> &'static str {
        "Open a serial port (UART) for a bounded time and return the captured log lines. Optionally decode hex code addresses (e.g. panic backtraces) using an ELF. Use after flash/run when the target logs over a physical UART."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "port": {"type": "string", "description": "Serial port, e.g. COM3 or /dev/ttyUSB0 (defaults to [tools] monitor_port)"},
                "baud": {"type": "integer", "minimum": 1, "description": "Baud rate (defaults to [tools] monitor_baud)"},
                "elf": {"type": "string", "description": "Optional path to the firmware ELF (inside the workspace) for decoding hex code addresses in log lines"},
                "timeout_ms": {"type": "integer", "minimum": 1, "default": 10000, "description": "How long to listen before returning captured output"}
            }
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        let port = args
            .get("port")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "configured port".to_string());
        Some(format!("⚠ open serial port for monitoring: {port}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let port = args
            .get("port")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .or_else(|| ctx.monitor_port.clone())
            .ok_or_else(|| {
                ToolError::new(
                    "[InvalidInput] missing port: pass a port parameter or set monitor_port in \
                     [tools] of config.toml (e.g. COM3)",
                )
            })?;
        let baud = args
            .get("baud")
            .and_then(|b| b.as_u64())
            .map(|b| b as u32)
            .unwrap_or(ctx.monitor_baud);
        let elf: Option<std::path::PathBuf> = args
            .get("elf")
            .and_then(|e| e.as_str())
            .map(|e| resolve_within(&ctx.cwd, e, &ctx.allowed_roots))
            .transpose()
            .map_err(ToolError::new)?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(10_000);

        let port_clone = port.clone();
        let captured = tokio::task::spawn_blocking(move || {
            read_serial(&port_clone, baud, timeout_ms, elf.as_deref())
        })
        .await
        .map_err(|e| ToolError::new(format!("[Io] monitor task failed: {e}")))?
        .map_err(ToolError::new)?;
        Ok(ToolOutput {
            text: format!("monitor {port} ({baud} baud, {timeout_ms} ms)\n{captured}"),
        })
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

    fn ctx(dir: &Path, monitor_port: Option<&str>) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: Some("stm32f407vetx".to_string()),
            monitor_port: monitor_port.map(|s| s.to_string()),
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
        }
    }

    #[tokio::test]
    async fn missing_port_is_an_error() {
        let dir = tempdir().unwrap();
        let err = Monitor
            .run(json!({}), &ctx(dir.path(), None))
            .await
            .unwrap_err();
        assert!(err.message.contains("monitor_port"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn elf_outside_workspace_is_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("evil.elf");
        std::fs::write(&outside, b"x").unwrap();
        let err = Monitor
            .run(
                json!({"port": "COM_FAKE", "elf": outside.to_string_lossy()}),
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
    async fn unopenable_port_returns_error() {
        let dir = tempdir().unwrap();
        let err = Monitor
            .run(json!({"port": "COM_DOES_NOT_EXIST_12345"}), &ctx(dir.path(), None))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("failed to open serial port"),
            "got: {}",
            err.message
        );
    }
}
