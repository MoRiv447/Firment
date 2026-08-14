use super::util::{read_text, resolve_within};
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::fs;

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read a text file. Output lines carry a line-number prefix (\"  123 | content\") so edit_file can target exact ranges; without offset/limit at most the first 1000 lines are returned and a [truncated] hint tells you how to read on."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path, absolute or relative to the workspace; output appends [file-sha256: ...] as metadata"},
                "offset": {"type": "integer", "minimum": 0, "description": "0-based line offset to start reading from"},
                "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of lines to read"},
                "hashlines": {"type": "boolean", "default": false, "description": "prefix every line with its [8-hex content hash] anchor for hashline edits (instead of line numbers)"}
            },
            "required": ["path"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        const DEFAULT_LIMIT: usize = 1000;
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| ToolError::new("missing 'path'"))?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit_arg = args.get("limit").and_then(|v| v.as_u64());
        let hashlines = args
            .get("hashlines")
            .and_then(|h| h.as_bool())
            .unwrap_or(false);
        // No explicit range -> default to the first chunk; an explicit
        // offset without a limit reads to the end (paging forward).
        // hashline mode keeps reading the whole file so large-file hash
        // anchors stay valid across the entire content.
        let limit = limit_arg
            .map(|v| v as usize)
            .unwrap_or(if offset == 0 && !hashlines {
                DEFAULT_LIMIT
            } else {
                0
            });
        let resolved =
            resolve_within(&ctx.cwd, path, &ctx.allowed_roots).map_err(ToolError::new)?;
        let content = read_text(&resolved).map_err(|e| {
            if resolved.exists() {
                ToolError::new(format!("[Io] {e}"))
            } else {
                ToolError::new(format!("[NotFound] {e}"))
            }
        })?;

        let lines: Vec<&str> = content.split('\n').collect();
        let effective_total = if content.ends_with('\n') {
            lines.len().saturating_sub(1)
        } else {
            lines.len()
        };
        let start = offset.min(effective_total);
        let end = if limit > 0 {
            start.saturating_add(limit).min(effective_total)
        } else {
            effective_total
        };
        let truncated = end < effective_total;
        let slice = &lines[start..end];
        let body = if hashlines {
            slice
                .iter()
                .map(|line| format!("[{}] {}", crate::tools::util::line_hash_prefix(line), line))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            // Line-number prefix so edit_file can target exact ranges (and
            // the model can report locations as path:line).
            slice
                .iter()
                .enumerate()
                .map(|(i, line)| format!("{:>6} | {}", start + i + 1, line))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut text = format!(
            "--- {} (lines {}..{}) ---\n{}",
            resolved.display(),
            start + 1, // 1-based, matching the line-number prefixes in the body
            end,
            body
        );
        if truncated {
            text.push_str(&format!(
                "\n[truncated: file has {effective_total} lines; pass offset={end} to read the next chunk]"
            ));
        }
        let digest = firment_core::hash::sha256_hex(
            &fs::read(&resolved).map_err(|e| ToolError::new(format!("[Io] {e}")))?,
        );
        Ok(ToolOutput {
            text: format!("{text}\n[file-sha256: {digest}]"),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::write_file::WriteFile;
    use firment_core::{AutoApprove, EditJournal};
    use serde_json::json;
    use std::fs;
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
    async fn hashlines_prefixes_every_line() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        let out = ReadFile
            .run(
                json!({"path": "a.txt", "hashlines": true}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        let hello_hash = crate::tools::util::line_hash_prefix("hello");
        assert!(
            out.text.contains(&format!("[{hello_hash}] hello")),
            "got: {}",
            out.text
        );
        assert!(
            !out.text.contains("[file-sha256: ["),
            "hashlines must not tag the footer"
        );
    }

    #[tokio::test]
    async fn output_includes_file_sha256() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello world").unwrap();
        let out = ReadFile
            .run(json!({"path": "a.txt"}), &ctx(dir.path()))
            .await
            .unwrap();
        let expected = firment_core::hash::sha256_hex(b"hello world");
        assert!(
            out.text.contains(&format!("[file-sha256: {expected}]")),
            "got: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn default_read_adds_line_numbers() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let out = ReadFile
            .run(json!({"path": "a.txt"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("1 | one"), "got: {}", out.text);
        assert!(out.text.contains("3 | three"), "got: {}", out.text);
        assert!(!out.text.contains("[truncated]"), "got: {}", out.text);
        // Regression: a trailing newline must not produce a phantom empty
        // line-numbered row (the `4 | ` line).
        assert!(
            !out.text.contains("4 | "),
            "trailing empty line should not be emitted: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn hashline_mode_reads_the_whole_file() {
        let dir = tempdir().unwrap();
        let body = (0..2500)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.path().join("big.txt"), &body).unwrap();
        let out = ReadFile
            .run(
                json!({"path": "big.txt", "hashlines": true}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(
            out.text.contains("line2499"),
            "hashline mode must cover the whole file, got tail: {}",
            out.text
        );
        assert!(
            !out.text.contains("[truncated]"),
            "hashline mode must not truncate: {}",
            out.text
        );
    }

    #[tokio::test]
    async fn large_file_is_capped_with_offset_hint() {
        let dir = tempdir().unwrap();
        let body = (0..2500)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.path().join("big.txt"), &body).unwrap();
        let out = ReadFile
            .run(json!({"path": "big.txt"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("1 | line0"), "got: {}", out.text);
        assert!(
            out.text
                .contains("[truncated: file has 2500 lines; pass offset=1000"),
            "got tail: {}",
            out.text
        );
        let out = ReadFile
            .run(json!({"path": "big.txt", "offset": 1000}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("1001 | line1000"), "got: {}", out.text);
        assert!(
            out.text.contains("2500 | line2499"),
            "got tail: {}",
            out.text
        );
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
            err.message.contains("outside the workspace"),
            "got: {}",
            err.message
        );

        let err = ReadFile
            .run(json!({"path": outside.to_string_lossy()}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("outside the workspace"),
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
