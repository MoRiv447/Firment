use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

use firment_core::WorkbenchConfig;

pub struct Decision;

fn now_date() -> String {
    // YYYY-MM-DD from the system clock (no chrono dependency).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[async_trait]
impl Tool for Decision {
    fn name(&self) -> &'static str {
        "decision"
    }

    fn description(&self) -> &'static str {
        "Project decision log (ADR-lite, .firment/workbench.toml [[decision]]). \
         Record every choice a future session would need to know: chip/peripheral \
         selection, protocol formats, pin assignments rationale, library picks. \
         Actions: list (all decisions), add (title+body; new branches whose title \
         matches automatically inherit it), remove (by 1-based list index). \
         Keep bodies short and factual — they are injected verbatim."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "add", "remove"], "description": "What to do"},
                "title": {"type": "string", "description": "add: short decision headline, e.g. \"I2C bus uses 400k\""},
                "body": {"type": "string", "description": "add: rationale/constraints, e.g. \"sensor max clock 400k; PA9/PA10 reserved\""},
                "index": {"type": "integer", "description": "remove: 1-based index from list"}
            },
            "required": ["action"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let action = args
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("list")
            .to_lowercase();
        let root = ctx.cwd.clone();
        match action.as_str() {
            "list" => {
                let cfg = WorkbenchConfig::load(&root).map_err(tool_err)?;
                if cfg.decision.is_empty() {
                    return Ok(ToolOutput {
                        text: "no decisions recorded yet for this project.".into(),
                    });
                }
                let mut text = String::new();
                for (i, d) in cfg.decision.iter().enumerate() {
                    text.push_str(&format!("{}. [{}] {}\n", i + 1, d.date, d.title));
                    if !d.body.is_empty() {
                        text.push_str(&format!("   {}\n", d.body.replace('\n', "\n   ")));
                    }
                }
                Ok(ToolOutput { text })
            }
            "add" => {
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ToolError::new("[InvalidInput] missing 'title'"))?
                    .to_string();
                if title.chars().count() > 120 {
                    return Err(ToolError::new(
                        "[InvalidInput] 'title' too long (max 120 chars) — keep the headline short",
                    ));
                }
                let body = args
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let mut cfg = WorkbenchConfig::load(&root).map_err(tool_err)?;
                cfg.decision.push(firment_core::DecisionEntry {
                    title,
                    body,
                    date: now_date(),
                });
                let total = cfg.decision.len();
                cfg.save(&root).map_err(tool_err)?;
                Ok(ToolOutput {
                    text: format!(
                        "recorded (#{total}). Branches whose title matches will inherit it automatically."
                    ),
                })
            }
            "remove" => {
                let index = args
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| ToolError::new("[InvalidInput] missing 'index' (1-based)"))?
                    as usize;
                let mut cfg = WorkbenchConfig::load(&root).map_err(tool_err)?;
                if index == 0 || index > cfg.decision.len() {
                    return Err(ToolError::new(format!(
                        "[InvalidInput] index {index} out of range (1..={})",
                        cfg.decision.len()
                    )));
                }
                let removed = cfg.decision.remove(index - 1);
                cfg.save(&root).map_err(tool_err)?;
                Ok(ToolOutput {
                    text: format!("removed #{}: {}", index, removed.title),
                })
            }
            other => Err(ToolError::new(format!(
                "[InvalidInput] unknown action '{other}' (list/add/remove)"
            ))),
        }
    }
}

/// `WorkbenchConfig` errors are plain Strings; wrap without losing text.
fn tool_err(e: String) -> ToolError {
    ToolError::new(format!("[Decision] {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
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
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn add_list_remove_roundtrip() {
        let dir = tempdir().unwrap();
        let out = Decision
            .run(json!({"action": "list"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("no decisions"), "got: {}", out.text);

        Decision
            .run(
                json!({"action": "add", "title": "I2C bus at 400k", "body": "sensor limit"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        let out = Decision
            .run(json!({"action": "list"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("I2C bus at 400k"), "got: {}", out.text);
        assert!(out.text.contains("1. ["), "numbered entry expected");

        let cfg = WorkbenchConfig::load(dir.path()).unwrap();
        assert_eq!(cfg.decision[0].body, "sensor limit");
        assert!(!cfg.decision[0].date.is_empty(), "date must be stamped");

        let out = Decision
            .run(json!({"action": "remove", "index": 1}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("removed #1"), "got: {}", out.text);
        assert!(
            WorkbenchConfig::load(dir.path())
                .unwrap()
                .decision
                .is_empty()
        );
    }

    #[tokio::test]
    async fn validation_errors() {
        let dir = tempdir().unwrap();
        let err = Decision
            .run(json!({"action": "add", "body": "x"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("missing 'title'"),
            "got: {}",
            err.message
        );

        let err = Decision
            .run(json!({"action": "remove", "index": 5}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("out of range"), "got: {}", err.message);
    }
}
