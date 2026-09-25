use super::util::{read_text, resolve_write_scope, simple_diff};
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
                "content": {"type": "string"},
                "expected_sha256": {"type": "string", "description": "Optional SHA-256 of the existing file (from read_file footer); mismatches abort with [ConcurrentChange]"}
            },
            "required": ["path", "content"]
        })
    }

    fn approval(&self, args: &Value) -> Option<String> {
        args.get("path")
            .and_then(|p| p.as_str())
            .map(|p| format!("write file {p}"))
    }

    fn preview(&self, args: &Value, ctx: &ToolContext) -> Option<String> {
        let path = args.get("path")?.as_str()?;
        let content = args.get("content")?.as_str()?;
        let resolved = resolve_write_scope(ctx, path).ok()?;
        let old = read_text(&resolved).unwrap_or_default();
        Some(simple_diff(&resolved, &old, content, 4000))
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
        let resolved = resolve_write_scope(ctx, path).map_err(ToolError::new)?;
        let existed = resolved.exists();
        let original_bytes = if existed {
            Some(fs::read(&resolved).map_err(|e| {
                ToolError::new(format!("[Io] cannot read {}: {e}", resolved.display()))
            })?)
        } else {
            None
        };
        ctx.journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(&resolved)
            .map_err(ToolError::new)?;
        if let Some(expected) = args.get("expected_sha256").and_then(|e| e.as_str())
            && let Some(original) = &original_bytes
        {
            let current = firment_core::hash::sha256_hex(original);
            if current != expected {
                return Err(ToolError::new(format!(
                    "[ConcurrentChange] file hash mismatch (expected {expected}, current \
                     {current}): re-read the file with read_file and retry"
                )));
            }
        }
        // CAS: refuse to overwrite a file that changed since we read it.
        if let Some(original) = &original_bytes {
            let current = fs::read(&resolved)
                .map_err(|e| ToolError::new(format!("[Io] re-read failed: {e}")))?;
            if &current != original {
                return Err(ToolError::new(format!(
                    "[ConcurrentChange] file changed during write (concurrent modification), aborted: {}",
                    resolved.display()
                )));
            }
        } else if resolved.exists() {
            return Err(ToolError::new(format!(
                "[ConcurrentChange] file appeared during write (concurrent modification), aborted: {}",
                resolved.display()
            )));
        }
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| ToolError::new(format!("[Io] create dirs: {e}")))?;
        }
        firment_core::session::write_atomic(&resolved, content)
            .map_err(|e| ToolError::new(format!("[Io] write failed: {e}")))?;
        // Echo the change for an OVERWRITE, like `edit_file` does: the model
        // sees what it replaced and the UI has a diff to render. A brand-new
        // file gets no diff on purpose — every line would be a `+`, which is
        // pure noise in the 1 MiB-sized transcripts this tool is used for.
        let text = match original_bytes.as_deref().map(String::from_utf8_lossy) {
            Some(old) => format!(
                "Wrote {} bytes to {}\n{}",
                content.len(),
                resolved.display(),
                simple_diff(&resolved, &old, content, 4000)
            ),
            None => format!("Wrote {} bytes to {}", content.len(), resolved.display()),
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
    async fn a_declared_scope_refuses_a_write_outside_it() {
        // The gate, at the tool a child would actually use: a refusal that names the scope,
        // and a file that was never created.
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("crates/foo")).unwrap();
        std::fs::create_dir_all(dir.path().join("crates/bar")).unwrap();
        let mut context = ctx(dir.path());
        context.write_scope = Some(vec![dir.path().join("crates/foo")]);

        WriteFile
            .run(
                json!({"path": "crates/foo/a.rs", "content": "fn a() {}\n"}),
                &context,
            )
            .await
            .expect("a write inside the scope is allowed");

        let err = WriteFile
            .run(
                json!({"path": "crates/bar/b.rs", "content": "fn b() {}\n"}),
                &context,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("declared scope"), "{}", err.message);
        assert!(
            !dir.path().join("crates/bar/b.rs").exists(),
            "a refused write must not have happened"
        );
    }

    #[tokio::test]
    async fn preview_shows_unified_diff() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        let preview = WriteFile
            .preview(
                &json!({"path": "a.txt", "content": "hello base\nworld\n"}),
                &ctx(dir.path()),
            )
            .unwrap();
        assert!(preview.contains("---"));
        assert!(preview.contains("+++"));
        assert!(preview.contains("-hello"), "got: {preview}");
        assert!(preview.contains("+hello base"), "got: {preview}");
    }

    #[tokio::test]
    async fn run_echoes_a_diff_on_overwrite_but_not_on_create() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();

        let overwritten = WriteFile
            .run(
                json!({"path": "a.txt", "content": "hello base\nworld\n"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(
            overwritten.text.contains("Wrote"),
            "got: {}",
            overwritten.text
        );
        assert!(
            overwritten.text.contains("-hello"),
            "an overwrite must show what it replaced: {}",
            overwritten.text
        );
        assert!(
            overwritten.text.contains("+hello base"),
            "got: {}",
            overwritten.text
        );

        let created = WriteFile
            .run(
                json!({"path": "b.txt", "content": "brand new\n"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(created.text.contains("Wrote"), "got: {}", created.text);
        assert!(
            !created.text.contains("@@") && !created.text.contains("+brand new"),
            "a brand-new file must not echo every line as an addition: {}",
            created.text
        );
    }
}
