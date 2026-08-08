use super::util::{read_text, resolve_within};
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
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
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
    use crate::tools::write_file::WriteFile;
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
            allowed_roots: Vec::new(),
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

    #[tokio::test]
    async fn rejects_paths_outside_workspace() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("firm-escape.txt");
        std::fs::write(&outside, "secret").unwrap();

        let err = WriteFile
            .run(
                json!({"path": "../firm-escape.txt", "content": "x"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("超出工作区边界"),
            "got: {}",
            err.message
        );

        let err = ReadFile
            .run(json!({"path": outside.to_string_lossy()}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("超出工作区边界"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn allowed_roots_are_readable() {
        let dir = tempdir().unwrap();
        let spill = dir.path().join("spill");
        std::fs::create_dir_all(&spill).unwrap();
        std::fs::write(spill.join("x.txt"), "spilled").unwrap();
        let mut tool_ctx = ctx(dir.path());
        tool_ctx.allowed_roots = vec![spill.clone()];
        let out = ReadFile
            .run(
                json!({"path": spill.join("x.txt").to_string_lossy()}),
                &tool_ctx,
            )
            .await
            .unwrap();
        assert!(out.text.contains("spilled"));
    }
}
