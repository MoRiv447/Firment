use super::util::{read_text, resolve_within, simple_diff};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

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

    fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<String> {
        let path = args.get("path")?.as_str()?;
        let resolved = resolve_within(&ctx.cwd, path, &ctx.allowed_roots).ok()?;
        let original = read_text(&resolved).ok()?;
        let new_content = compute_edit(&resolved, &original, args).ok()?;
        Some(simple_diff(&resolved, &original, &new_content, 4000))
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'path'"))?;
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        let original = read_text(&resolved).map_err(|e| {
            if resolved.exists() {
                ToolError::new(format!("[Io] {e}"))
            } else {
                ToolError::new(format!("[NotFound] {e}"))
            }
        })?;
        let original_bytes = fs::read(&resolved)
            .map_err(|e| ToolError::new(format!("[Io] cannot read {}: {e}", resolved.display())))?;
        let new_content = compute_edit(&resolved, &original, &args)?;

        ctx.journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(&resolved)
            .map_err(ToolError::new)?;
        // CAS: refuse to apply the edit if the file changed since we read it.
        let current =
            fs::read(&resolved).map_err(|e| ToolError::new(format!("[Io] re-read failed: {e}")))?;
        if current != original_bytes {
            return Err(ToolError::new(format!(
                "[ConcurrentChange] file changed during edit (concurrent modification), aborted: {}",
                resolved.display()
            )));
        }
        fs::write(&resolved, &new_content)
            .map_err(|e| ToolError::new(format!("[Io] write failed: {e}")))?;
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

fn compute_edit(resolved: &Path, original: &str, args: &Value) -> Result<String, ToolError> {
    let new_text = args
        .get("new_text")
        .and_then(|n| n.as_str())
        .ok_or_else(|| ToolError::new("[InvalidInput] missing 'new_text'"))?;
    let old_text = args.get("old_text").and_then(|o| o.as_str());
    let start_line = args.get("start_line").and_then(|s| s.as_u64());
    let end_line = args.get("end_line").and_then(|e| e.as_u64());

    if let Some(old) = old_text {
        let occurrences = original.match_indices(old).count();
        if occurrences != 1 {
            return Err(ToolError::new(format!(
                "[InvalidInput] old_text matched {occurrences} times in {}; expected exactly 1",
                resolved.display()
            )));
        }
        Ok(original.replacen(old, new_text, 1))
    } else {
        let start = start_line.ok_or_else(|| {
            ToolError::new("[InvalidInput] provide either 'old_text' or 'start_line'")
        })? as usize;
        let end = end_line.unwrap_or(start as u64) as usize;
        if start == 0 || end < start {
            return Err(ToolError::new("[InvalidInput] invalid line range"));
        }
        let mut lines: Vec<&str> = original.split('\n').collect();
        let trailing_newline = original.ends_with('\n');
        if trailing_newline {
            lines.pop();
        }
        if start > lines.len() || end > lines.len() {
            return Err(ToolError::new(format!(
                "[InvalidInput] line range {start}..{end} out of bounds (file has {} lines)",
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
        Ok(joined)
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
            default_chip: None,
            allowed_roots: Vec::new(),
        }
    }

    #[tokio::test]
    async fn preview_shows_edit_diff() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        let preview = EditFile
            .preview(
                &json!({"path": "a.txt", "old_text": "hello", "new_text": "hi"}),
                &ctx(dir.path()),
            )
            .unwrap();
        assert!(preview.contains("-hello"), "got: {preview}");
        assert!(preview.contains("+hi"), "got: {preview}");
    }

    #[tokio::test]
    async fn invalid_anchor_returns_tagged_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let err = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "nope", "new_text": "x"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[InvalidInput]"),
            "got: {}",
            err.message
        );
    }
}
