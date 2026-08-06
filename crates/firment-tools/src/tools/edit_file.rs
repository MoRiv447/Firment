use super::util::{read_text, resolve};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Edit a file by exact old_text anchor (must match exactly once) or by 1-based line range."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_text": {"type": "string", "description": "Exact text to replace; must occur exactly once"},
                "start_line": {"type": "integer", "minimum": 1},
                "end_line": {"type": "integer", "minimum": 1},
                "new_text": {"type": "string", "description": "Replacement text"}
            },
            "required": ["path", "new_text"],
            "oneOf": [
                {"required": ["old_text"]},
                {"required": ["start_line"]}
            ]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        args.get("path")
            .and_then(|p| p.as_str())
            .map(|p| format!("edit file {p}"))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("missing 'path'"))?;
        let new_text = args
            .get("new_text")
            .and_then(|n| n.as_str())
            .ok_or_else(|| ToolError::new("missing 'new_text'"))?;
        let old_text = args.get("old_text").and_then(|o| o.as_str());
        let start_line = args.get("start_line").and_then(|s| s.as_u64());
        let end_line = args.get("end_line").and_then(|e| e.as_u64());
        let resolved = resolve(&ctx.cwd, path);
        let original = read_text(&resolved).map_err(ToolError::new)?;

        let new_content = if let Some(old) = old_text {
            let occurrences = original.match_indices(old).count();
            if occurrences != 1 {
                return Err(ToolError::new(format!(
                    "old_text matched {occurrences} times in {}; expected exactly 1",
                    resolved.display()
                )));
            }
            original.replacen(old, new_text, 1)
        } else {
            let start = start_line
                .ok_or_else(|| ToolError::new("provide either 'old_text' or 'start_line'"))?
                as usize;
            let end = end_line.unwrap_or(start as u64) as usize;
            if start == 0 || end < start {
                return Err(ToolError::new("invalid line range"));
            }
            let mut lines: Vec<&str> = original.split('\n').collect();
            let trailing_newline = original.ends_with('\n');
            if trailing_newline {
                lines.pop();
            }
            if start > lines.len() || end > lines.len() {
                return Err(ToolError::new(format!(
                    "line range {start}..{end} out of bounds (file has {} lines)",
                    lines.len()
                )));
            }
            let mut out: Vec<&str> = Vec::new();
            out.extend_from_slice(&lines[..start - 1]);
            out.extend(new_text.split('\n'));
            out.extend_from_slice(&lines[end..]);
            let mut joined = out.join("\n");
            if trailing_newline {
                joined.push('\n');
            }
            joined
        };

        fs::write(&resolved, &new_content)
            .map_err(|e| ToolError::new(format!("write failed: {e}")))?;
        let old_lines = original.lines().count();
        let new_lines = new_content.lines().count();
        Ok(ToolOutput {
            text: format!(
                "Edited {} ({} lines -> {} lines)",
                resolved.display(),
                old_lines,
                new_lines
            ),
        })
    }
}
