use super::util::{read_text, rel_str, resolve};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use globset::Glob;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde_json::{Value, json};

pub struct Grep;

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> &'static str {
        "Search file contents with a regular expression. Optionally filter by glob and line count."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "default": "."},
                "glob": {"type": "string", "description": "Optional file glob filter, e.g. **/*.c"},
                "case_sensitive": {"type": "boolean", "default": false},
                "max_results": {"type": "integer", "minimum": 1, "default": 100},
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
        let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|c| c.as_bool())
            .unwrap_or(false);
        let max_results = args
            .get("max_results")
            .and_then(|m| m.as_u64())
            .unwrap_or(100) as usize;
        let include_hidden = args
            .get("include_hidden")
            .and_then(|h| h.as_bool())
            .unwrap_or(false);
        let glob_filter = args
            .get("glob")
            .and_then(|g| g.as_str())
            .map(Glob::new)
            .transpose()
            .map_err(|e| ToolError::new(format!("bad glob pattern: {e}")))?
            .map(|g| g.compile_matcher());
        let regex = RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| ToolError::new(format!("bad regex: {e}")))?;
        let resolved = resolve(&ctx.cwd, path);
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
            let rel = rel_str(&resolved, entry.path());
            if let Some(matcher) = &glob_filter
                && !matcher.is_match(&rel)
            {
                continue;
            }
            let Ok(content) = read_text(entry.path()) else {
                continue;
            };
            for (idx, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    out.push(format!(
                        "{}:{}:{}",
                        rel,
                        idx + 1,
                        super::util::truncate(line, 500)
                    ));
                    if out.len() >= max_results {
                        break;
                    }
                }
            }
            if out.len() >= max_results {
                break;
            }
        }
        if out.len() >= max_results {
            out.push(format!("... stopped at {max_results} results"));
        }
        Ok(ToolOutput {
            text: out.join("\n"),
        })
    }
}
