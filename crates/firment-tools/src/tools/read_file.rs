use super::util::{read_text, resolve};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read a text file. Optionally slice by line offset and limit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path, absolute or relative to the workspace"},
                "offset": {"type": "integer", "minimum": 0, "description": "0-based line offset to start reading from"},
                "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of lines to read"}
            },
            "required": ["path"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("missing 'path'"))?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let resolved = resolve(&ctx.cwd, path);
        let content = read_text(&resolved).map_err(|e| {
            if resolved.exists() {
                ToolError::new(format!("[Io] {e}"))
            } else {
                ToolError::new(format!("[NotFound] {e}"))
            }
        })?;

        let text = if offset > 0 || limit > 0 {
            let lines: Vec<&str> = content.split('\n').collect();
            let start = offset.min(lines.len());
            let end = if limit > 0 {
                (start + limit).min(lines.len())
            } else {
                lines.len()
            };
            format!(
                "--- {} (lines {}..{}) ---\n{}",
                resolved.display(),
                start,
                end,
                lines[start..end].join("\n")
            )
        } else {
            content
        };
        Ok(ToolOutput { text })
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
        }
    }

    #[tokio::test]
    async fn missing_file_returns_not_found_tag() {
        let dir = tempdir().unwrap();
        let err = ReadFile
            .run(json!({"path": "nope.txt"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("[NotFound]"), "got: {}", err.message);
    }
}
