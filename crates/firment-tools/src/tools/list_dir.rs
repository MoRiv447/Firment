use super::util::resolve_within;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct ListDir;

#[async_trait]
impl Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List entries in a directory. Set recursive=true to walk subdirectories."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "default": "."},
                "recursive": {"type": "boolean", "default": false},
                "limit": {"type": "integer", "minimum": 1, "default": 200}
            },
            "required": []
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let recursive = args
            .get("recursive")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);
        let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(200) as usize;
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        if !resolved.is_dir() {
            return Err(ToolError::new(format!(
                "{} is not a directory",
                resolved.display()
            )));
        }
        let mut out = Vec::new();
        walk(&resolved, &resolved, recursive, limit, &mut out);
        if out.len() >= limit {
            out.push(format!("... stopped at {limit} entries"));
        }
        Ok(ToolOutput {
            text: out.join("\n"),
        })
    }
}

fn walk(
    root: &std::path::Path,
    dir: &std::path::Path,
    recursive: bool,
    limit: usize,
    out: &mut Vec<String>,
) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        out.push(format!("! cannot read {}", dir.display()));
        return;
    };
    let mut items: Vec<fs::DirEntry> = entries.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        if out.len() >= limit {
            return;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            out.push(format!("{rel}/"));
            if recursive {
                walk(root, &path, true, limit, out);
            }
        } else {
            out.push(format!("{rel}  ({} B)", meta.len()));
        }
    }
}
