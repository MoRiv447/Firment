//! Programmatic flash/run entry points for embedders. The Tauri GUI calls
//! these instead of shelling out to the `firm` binary, so the GUI no longer
//! depends on an installed CLI and always runs the same probe-rs pipeline as
//! the agent tools (workspace sandbox, install hints, ST-Link diagnostics,
//! error mapping).

use std::path::Path;
use std::sync::Arc;

use firment_core::{AutoApprove, Tool, ToolContext};
use serde_json::{Value, json};

use crate::tools::flash::Flash;
use crate::tools::run::Run;

/// A minimal context for direct (non-agent) tool runs: the GUI's flash panel
/// is an explicit user action, so there is nobody left to ask for approval
/// and the call is auto-approved.
fn direct_ctx(cwd: &Path) -> ToolContext {
    let mut ctx = ToolContext::with_cwd(cwd.to_path_buf());
    ctx.permission = Arc::new(AutoApprove::everything());
    ctx.allowed_roots = vec![cwd.to_path_buf()];
    ctx
}

fn with_optional(args: &mut Value, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        args[key] = json!(value);
    }
}

/// Flash a firmware image to the target via probe-rs (reset included).
/// `file` resolves against `cwd` like the `flash` tool does.
pub async fn flash_elf(
    cwd: &Path,
    file: &str,
    chip: Option<&str>,
    probe: Option<&str>,
    timeout_ms: u64,
) -> Result<String, String> {
    let ctx = direct_ctx(cwd);
    let mut args = json!({ "file": file, "timeout_ms": timeout_ms });
    with_optional(&mut args, "chip", chip);
    with_optional(&mut args, "probe", probe);
    Flash
        .run(args, &ctx)
        .await
        .map(|out| out.text)
        .map_err(|e| e.message)
}

/// Flash and run the firmware, capturing the RTT log. `timeout_ms = 0` waits
/// forever, matching the `run` tool.
pub async fn run_elf(
    cwd: &Path,
    file: &str,
    chip: Option<&str>,
    probe: Option<&str>,
    timeout_ms: u64,
) -> Result<String, String> {
    let ctx = direct_ctx(cwd);
    let mut args = json!({ "file": file, "timeout_ms": timeout_ms });
    with_optional(&mut args, "chip", chip);
    with_optional(&mut args, "probe", probe);
    Run
        .run(args, &ctx)
        .await
        .map(|out| out.text)
        .map_err(|e| e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_chip_reports_config_hint() {
        let dir = std::env::temp_dir().join("firment-hardware-test");
        let _ = std::fs::create_dir_all(&dir);
        let err = flash_elf(&dir, "fw.elf", None, None, 1_000)
            .await
            .unwrap_err();
        assert!(err.contains("chip"), "got: {err}");
    }

    #[tokio::test]
    async fn file_outside_cwd_is_rejected() {
        let dir = std::env::temp_dir().join("firment-hardware-test");
        let _ = std::fs::create_dir_all(&dir);
        let outside = std::env::temp_dir().join("firment-hardware-evil.elf");
        std::fs::write(&outside, b"x").unwrap();
        let err = flash_elf(&dir, outside.to_str().unwrap(), Some("stm32f407vetx"), None, 1_000)
            .await
            .unwrap_err();
        assert!(err.contains("outside the workspace"), "got: {err}");
    }
}
