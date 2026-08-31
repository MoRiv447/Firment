use super::util::resolve_within;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct Task;

#[async_trait]
impl Tool for Task {
    fn name(&self) -> &'static str {
        "task"
    }

    fn description(&self) -> &'static str {
        "Run a read-only research subagent that investigates on its own and returns a report. Use for long, self-contained investigations (code archaeology, datasheet research, writing a design summary) so you can keep working. The subagent can read files, search the web, fetch pages, and keep todos, but cannot modify the workspace or ask the user. Its report is returned as text; recursion depth is bounded."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "What the subagent should investigate and what to report back. Be specific about the expected output format."},
                "model": {"type": "string", "description": "Optional model override for the subagent (defaults to the session model)"},
                "provider": {"type": "string", "description": "Optional provider name override (a provider configured in config.toml, e.g. an Ollama endpoint added via add-provider). Defaults to the session provider. Combine with model to run cheap subagents on a local/small backend. Discover available models first with the models tool."},
                "cwd": {"type": "string", "description": "Optional subdirectory of the workspace to focus the subagent on"}
            },
            "required": ["prompt"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(|p| p.as_str())
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing or empty prompt"))?;
        let max_depth = ctx.max_subagent_depth.max(1);
        if ctx.subagent_depth >= max_depth {
            return Err(ToolError::new(format!(
                "[TooDeep] subagent recursion limit reached ({max_depth}); do not spawn more \
                 task tools, do the work directly"
            )));
        }
        let factory = ctx.subagent.as_ref().ok_or_else(|| {
            ToolError::new(
                "[NoSubagent] the task tool has no subagent runner in this context (direct \
                 tool run or tests)",
            )
        })?;
        let cwd = match args.get("cwd").and_then(|c| c.as_str()) {
            Some(dir) => {
                resolve_within(&ctx.cwd, dir, &ctx.allowed_roots).map_err(ToolError::new)?
            }
            None => ctx.cwd.clone(),
        };
        let model = args.get("model").and_then(|m| m.as_str());
        let provider = args.get("provider").and_then(|p| p.as_str());
        let report = factory
            .run_subagent(
                prompt,
                cwd,
                provider,
                model,
                ctx.subagent_depth + 1,
                ctx.cancel.clone(),
            )
            .await
            .map_err(ToolError::new)?;
        Ok(ToolOutput {
            text: format!("[subagent report]\n{report}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use firment_core::{AutoApprove, EditJournal, SubagentFactory};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    type Capture = (String, PathBuf, Option<String>, Option<String>, usize);

    #[derive(Clone)]
    struct StubFactory {
        answer: String,
        captures: Arc<Mutex<Vec<Capture>>>,
    }

    #[async_trait]
    impl SubagentFactory for StubFactory {
        async fn run_subagent(
            &self,
            prompt: &str,
            cwd: PathBuf,
            provider: Option<&str>,
            model: Option<&str>,
            depth: usize,
            _cancel: firment_core::Cancellable,
        ) -> Result<String, String> {
            self.captures.lock().unwrap().push((
                prompt.to_string(),
                cwd,
                provider.map(|p| p.to_string()),
                model.map(|m| m.to_string()),
                depth,
            ));
            Ok(self.answer.clone())
        }
    }

    fn ctx(
        dir: &Path,
        depth: usize,
        max_depth: usize,
        factory: Option<StubFactory>,
    ) -> ToolContext {
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
            subagent: factory.map(|f| Arc::new(f) as _),
            subagent_depth: depth,
            max_subagent_depth: max_depth,
            asker: None,
            device_log_dir: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: None,
            ledger_path: None,
            providers: Vec::new(),
            la: None,
            cancel: firment_core::Cancellable::new(),
            allowed_roots: Vec::new(),
        }
    }

    #[tokio::test]
    async fn missing_prompt_is_an_error() {
        let dir = tempdir().unwrap();
        let err = Task
            .run(json!({}), &ctx(dir.path(), 0, 2, None))
            .await
            .unwrap_err();
        assert!(err.message.contains("[InvalidInput]"), "got: {err}");
    }

    #[tokio::test]
    async fn without_runner_is_an_error() {
        let dir = tempdir().unwrap();
        let err = Task
            .run(
                json!({"prompt": "research x"}),
                &ctx(dir.path(), 0, 2, None),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[NoSubagent]"), "got: {err}");
    }

    #[tokio::test]
    async fn recursion_limit_is_enforced() {
        let dir = tempdir().unwrap();
        let factory = StubFactory {
            answer: "n/a".to_string(),
            captures: Arc::new(Mutex::new(Vec::new())),
        };
        let err = Task
            .run(
                json!({"prompt": "research x"}),
                &ctx(dir.path(), 2, 2, Some(factory)),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[TooDeep]"), "got: {err}");
    }

    #[tokio::test]
    async fn runs_the_subagent_and_returns_the_report() {
        let dir = tempdir().unwrap();
        let captures = Arc::new(Mutex::new(Vec::new()));
        let factory = StubFactory {
            answer: "the report".to_string(),
            captures: captures.clone(),
        };
        let out = Task
            .run(
                json!({"prompt": "research x", "model": "deepseek-v4-flash"}),
                &ctx(dir.path(), 0, 2, Some(factory)),
            )
            .await
            .unwrap();
        assert!(out.text.contains("[subagent report]"), "got: {}", out.text);
        assert!(out.text.contains("the report"), "got: {}", out.text);
        let captured = captures.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "research x");
        assert_eq!(captured[0].1, dir.path());
        // No provider override: None (session provider is inherited).
        assert_eq!(captured[0].2, None);
        assert_eq!(captured[0].3.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(captured[0].4, 1);
    }

    #[tokio::test]
    async fn provider_and_model_overrides_are_passed_through() {
        let dir = tempdir().unwrap();
        let captures = Arc::new(Mutex::new(Vec::new()));
        let factory = StubFactory {
            answer: "n/a".to_string(),
            captures: captures.clone(),
        };
        Task.run(
            json!({"prompt": "triage logs", "provider": "sbc-ollama", "model": "qwen3.5:0.8b"}),
            &ctx(dir.path(), 0, 2, Some(factory)),
        )
        .await
        .unwrap();
        let captured = captures.lock().unwrap();
        assert_eq!(captured[0].2.as_deref(), Some("sbc-ollama"));
        assert_eq!(captured[0].3.as_deref(), Some("qwen3.5:0.8b"));
    }

    #[tokio::test]
    async fn cwd_must_stay_inside_the_workspace() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("evil");
        let factory = StubFactory {
            answer: "n/a".to_string(),
            captures: Arc::new(Mutex::new(Vec::new())),
        };
        let err = Task
            .run(
                json!({"prompt": "research x", "cwd": outside.to_string_lossy()}),
                &ctx(dir.path(), 0, 2, Some(factory)),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("outside the workspace"), "got: {err}");
    }
}
