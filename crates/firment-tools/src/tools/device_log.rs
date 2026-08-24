use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::path::Path;

pub struct DeviceLog;

fn today_file(dir: Option<&Path>) -> std::path::PathBuf {
    // Same YYYYMMDD naming the GUI link and sbc-guard use for daily sinks.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        / 86_400;
    let z = secs as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    let base = firment_core::config::config_dir();
    let base = dir.unwrap_or(&base);
    base.join(format!("device-log-{y:04}{m:02}{d:02}.jsonl"))
}

#[async_trait]
impl Tool for DeviceLog {
    fn name(&self) -> &'static str {
        "device_log"
    }

    fn description(&self) -> &'static str {
        "Read recent device frames captured by the desktop MQTT link \
         (telemetry/state/alert/echo from all nodes). Optional filters: node \
         name, kind (telemetry|state|alert|echo), tail (last N lines, default \
         50). Use this to answer \"what is the board doing / what did the \
         guard catch\" without leaving the conversation."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "node": {"type": "string", "description": "Only frames whose payload contains this node name"},
                "kind": {"type": "string", "description": "Only frames whose topic kind matches, e.g. alert"},
                "tail": {"type": "integer", "default": 50, "description": "Return at most the last N matching lines (1..500)"}
            },
            "required": []
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let node = args.get("node").and_then(|v| v.as_str()).unwrap_or("");
        let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let tail = args
            .get("tail")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .clamp(1, 500) as usize;

        let path = today_file(ctx.device_log_dir.as_deref());
        if !path.is_file() {
            return Ok(ToolOutput {
                text: "no device traffic recorded today (the desktop MQTT link \
                       appends frames to device-log-<date>.jsonl automatically)."
                    .into(),
            });
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ToolError::new(format!("[DeviceLog] read: {e}")))?;
        let mut matched: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter(|l| node.is_empty() || l.contains(node))
            .filter(|l| kind.is_empty() || l.contains(&format!("\"kind\":\"{kind}\"")))
            .collect();
        let total = matched.len();
        if total > tail {
            matched.drain(..total - tail);
        }

        Ok(ToolOutput {
            text: format!(
                "{total} matching frame(s) today, showing last {}:\n{}",
                matched.len(),
                matched.join("\n")
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            device_log_dir: Some(dir.to_path_buf()),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn reads_filters_and_tails_today_log() {
        // Inject the log dir directly (no env-var races with parallel tests).
        let dir = tempdir().unwrap();
        let path = today_file(Some(dir.path()));
        std::fs::write(
            &path,
            concat!(
                "{\"node\":\"s3-node-1\",\"kind\":\"telemetry\",\"payload\":\"raw=1\"}\n",
                "{\"node\":\"s3-node-1\",\"kind\":\"alert\",\"payload\":\"E (1) x\"}\n",
                "{\"node\":\"other\",\"kind\":\"telemetry\",\"payload\":\"raw=2\"}\n",
            ),
        )
        .unwrap();

        let ctx = ctx(dir.path());
        let out = DeviceLog
            .run(json!({"node": "s3-node-1", "kind": "alert"}), &ctx)
            .await
            .unwrap();
        assert!(out.text.contains("1 matching frame"), "got: {}", out.text);
        assert!(out.text.contains("E (1) x"), "got: {}", out.text);

        let out = DeviceLog.run(json!({"tail": 2}), &ctx).await.unwrap();
        assert!(out.text.contains("3 matching frame"), "got: {}", out.text);
        let lines: usize = out.text.lines().count();
        assert_eq!(lines, 3, "header + 2 tail lines");
    }
}
