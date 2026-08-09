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
                "expected_sha256": {"type": "string", "description": "Optional SHA-256 of the file content as read (from read_file footer); mismatches abort with [ConcurrentChange]"},
                "start_line": {"type": "integer", "minimum": 1},
                "end_line": {"type": "integer", "minimum": 1},
                "new_text": {"type": "string", "description": "Replacement text"},
                "hashline": {"type": "string", "description": "8-hex content hash anchor of the first line to replace (from read_file hashlines=true); the file must still contain exactly one line with this hash"},
                "end_hashline": {"type": "string", "description": "8-hex content hash of the last line of the range (optional, same mode as hashline)"}
            },
            "required": ["path", "new_text"],
            "oneOf": [
                {"required": ["old_text"]},
                {"required": ["start_line"]},
                {"required": ["hashline"]}
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
        if let Some(expected) = args.get("expected_sha256").and_then(|e| e.as_str()) {
            let current = firment_core::hash::sha256_hex(&original_bytes);
            if current != expected {
                return Err(ToolError::new(format!(
                    "[ConcurrentChange] file hash mismatch (expected {expected}, current \
                     {current}): re-read the file with read_file and retry"
                )));
            }
        }
        let new_content = compute_edit(&resolved, &original, &args)?;
        if new_content == original {
            return Err(ToolError::new(
                "[InvalidInput] the edit produced no change (target content equals replacement \
                 content). The problem is likely elsewhere: re-read the file first; do not \
                 widen the anchor or resubmit the same edit.",
            ));
        }

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
    let hashline = args.get("hashline").and_then(|h| h.as_str());
    let end_hashline = args.get("end_hashline").and_then(|h| h.as_str());

    if let Some(hashline) = hashline {
        return edit_by_hashline(original, hashline, end_hashline, new_text);
    }

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

/// omp-style hashline edit: locate lines by 8-hex content-hash anchors.
fn edit_by_hashline(
    original: &str,
    hashline: &str,
    end_hashline: Option<&str>,
    new_text: &str,
) -> Result<String, ToolError> {
    let mut lines: Vec<&str> = original.split('\n').collect();
    let trailing_newline = original.ends_with('\n');
    if trailing_newline {
        lines.pop();
    }
    let full_hash = |line: &str| firment_core::hash::sha256_hex(line.as_bytes());
    let find = |anchor: &str| -> Result<usize, ToolError> {
        let anchor = anchor.to_lowercase();
        let matches: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| full_hash(line).starts_with(&anchor))
            .map(|(i, _)| i)
            .collect();
        match matches.len() {
            1 => Ok(matches[0]),
            0 => Err(ToolError::new(format!(
                "[ConcurrentChange] anchor hash {anchor} not found in the file: it may have \
                 changed; re-read with read_file and retry"
            ))),
            _ => Err(ToolError::new(format!(
                "[InvalidInput] anchor hash {anchor} matches {} line(s), not unique: re-read \
                 with read_file hashlines=true and use a longer hash",
                matches.len()
            ))),
        }
    };
    let start = find(hashline)?;
    let end = match end_hashline {
        Some(end) => {
            let end = find(end)?;
            if end < start {
                return Err(ToolError::new(
                    "[InvalidInput] end_hashline is before hashline; invalid range",
                ));
            }
            end
        }
        None => start,
    };
    let mut out: Vec<&str> = Vec::new();
    out.extend_from_slice(&lines[..start]);
    out.extend(new_text.split('\n'));
    out.extend_from_slice(&lines[end + 1..]);
    let mut joined = out.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    Ok(joined)
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
            monitor_port: None,
            monitor_baud: 115_200,
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
    async fn hashline_edits_by_content_hash() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa\nbbb\nccc\n").unwrap();
        let anchor = crate::tools::util::line_hash_prefix("bbb");
        let ok = EditFile
            .run(
                json!({"path": "a.txt", "hashline": anchor, "new_text": "XXX"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(ok.text.contains("Edited"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "aaa\nXXX\nccc\n"
        );
    }

    #[tokio::test]
    async fn hashline_range_edits_multiple_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa\nbbb\nccc\nddd\n").unwrap();
        let start = crate::tools::util::line_hash_prefix("bbb");
        let end = crate::tools::util::line_hash_prefix("ccc");
        let ok = EditFile
            .run(
                json!({"path": "a.txt", "hashline": start, "end_hashline": end, "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(ok.text.contains("Edited"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "aaa\nX\nddd\n"
        );
    }

    #[tokio::test]
    async fn hashline_missing_anchor_reports_concurrent_change() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa\n").unwrap();
        let err = EditFile
            .run(
                json!({"path": "a.txt", "hashline": "00000000", "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[ConcurrentChange]"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn hashline_ambiguous_anchor_is_rejected() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "bbb\nbbb\n").unwrap();
        let anchor = crate::tools::util::line_hash_prefix("bbb");
        let err = EditFile
            .run(
                json!({"path": "a.txt", "hashline": anchor, "new_text": "X"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("not unique"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn no_change_edit_is_a_hard_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "bbb\n").unwrap();
        let err = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "bbb", "new_text": "bbb"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("no change"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn expected_sha256_guards_against_stale_reads() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let digest = firment_core::hash::sha256_hex(b"hello\n");
        let err = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "hello", "new_text": "hi", "expected_sha256": "0".repeat(64)}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[ConcurrentChange]"),
            "got: {}",
            err.message
        );
        assert!(err.message.contains("current"), "got: {}", err.message);

        let ok = EditFile
            .run(
                json!({"path": "a.txt", "old_text": "hello", "new_text": "hi", "expected_sha256": digest}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(ok.text.contains("Edited"));
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
