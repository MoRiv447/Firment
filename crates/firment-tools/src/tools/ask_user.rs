use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};

pub struct AskUser;

#[async_trait]
impl Tool for AskUser {
    fn name(&self) -> &'static str {
        "ask_user"
    }

    fn description(&self) -> &'static str {
        "Ask the human user (the person running this session) a question and return their answer. Use only for decisions or information only the user has (which board/chip variant, hardware wiring, whether to install a toolchain, preference between approaches). Prefer 2-5 short options; the user can also type a free-form answer. Never ask something you can find out yourself with read_file / web_search / web_fetch, and never use this to ask another AI for information — spawn a `task` subagent for that."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "The question, phrased as a single clear request"},
                "options": {"type": "array", "items": {"type": "string"}, "maxItems": 9, "description": "Optional short answer options the user can pick from"}
            },
            "required": ["question"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let question = args
            .get("question")
            .and_then(|q| q.as_str())
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing or empty question"))?;
        let mut options: Vec<String> = args
            .get("options")
            .and_then(|o| o.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        options.truncate(9);
        let asker = ctx.asker.as_ref().ok_or_else(|| {
            ToolError::new(
                "[NoUser] no interactive user available in this context (one-shot mode or \
                 tests)",
            )
        })?;
        let answer = asker
            .ask(question, &options)
            .await
            .map_err(|e| ToolError::new(format!("[NoUser] {e}")))?;
        Ok(ToolOutput {
            text: format!("User's answer: {answer}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
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
            subagent: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            device_log_dir: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: None,
            ledger_path: None,
            providers: Vec::new(),
            la: None,
            attacker: None,
            cancel: firment_core::Cancellable::new(),
            allowed_roots: Vec::new(),
        }
    }

    #[tokio::test]
    async fn without_asker_is_an_error() {
        let dir = tempdir().unwrap();
        let err = AskUser
            .run(json!({"question": "chip?"}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("[NoUser]"), "got: {err}");
    }

    #[tokio::test]
    async fn declined_question_returns_a_message() {
        let dir = tempdir().unwrap();
        let mut tool_ctx = ctx(dir.path());
        tool_ctx.asker = Some(Arc::new(DecliningAsker));
        let err = AskUser
            .run(json!({"question": "chip?"}), &tool_ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("declined"), "got: {err}");
    }

    struct DecliningAsker;
    #[async_trait]
    impl firment_core::Asker for DecliningAsker {
        async fn ask(&self, _question: &str, _options: &[String]) -> Result<String, String> {
            Err("user declined".to_string())
        }
    }
}
