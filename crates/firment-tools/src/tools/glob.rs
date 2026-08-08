use super::util::resolve_within;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use globset::Glob as GlobPattern;
use ignore::WalkBuilder;
use serde_json::{Value, json};

pub struct Glob;

#[async_trait]
impl Tool for Glob {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn description(&self) -> &'static str {
        "Find files matching a glob pattern (e.g. **/*.rs) inside the workspace, respecting .gitignore."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "root": {"type": "string", "default": "."},
                "limit": {"type": "integer", "minimum": 1, "default": 200},
                "include_hidden": {"type": "boolean", "default": false}
            },
            "required": ["pattern"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("missing 'pattern'"))?;
        let root = args.get("root").and_then(|r| r.as_str()).unwrap_or(".");
        let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(200) as usize;
        let include_hidden = args
            .get("include_hidden")
            .and_then(|h| h.as_bool())
            .unwrap_or(false);
        let glob = GlobPattern::new(pattern)
            .map_err(|e| ToolError::new(format!("bad glob pattern: {e}")))?
            .compile_matcher();
        let resolved =
            resolve_within(&ctx.cwd, root, &ctx.allowed_roots).map_err(ToolError::new)?;
        if !resolved.is_dir() {
            return Err(ToolError::new(format!(
                "{} is not a directory",
                resolved.display()
            )));
        }

        let mut out = Vec::new();
        for entry in WalkBuilder::new(&resolved).hidden(!include_hidden).build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = super::util::rel_str(&resolved, entry.path());
            if glob.is_match(&rel) {
                out.push(rel);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(ToolOutput {
            text: out.join("\n"),
        })
    }
}
