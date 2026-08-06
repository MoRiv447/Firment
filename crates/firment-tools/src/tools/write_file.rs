use super::util::resolve;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Create or overwrite a file with the given text content. Parent directories are created if needed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        args.get("path")
            .and_then(|p| p.as_str())
            .map(|p| format!("write file {p}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("missing 'path'"))?;
        let content = args
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| ToolError::new("missing 'content'"))?;
        let resolved = resolve(&ctx.cwd, path);
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolError::new(format!("create dirs: {e}")))?;
        }
        fs::write(&resolved, content).map_err(|e| ToolError::new(format!("write failed: {e}")))?;
        Ok(ToolOutput {
            text: format!("Wrote {} bytes to {}", content.len(), resolved.display()),
        })
    }
}
