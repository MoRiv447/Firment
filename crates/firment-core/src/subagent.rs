use crate::Asker;
use crate::agent::{Agent, AgentEvent, EventSink};
use crate::cancel::Cancellable;
use crate::config::Config;
use crate::permission::PermissionChecker;
use crate::session::{Session, SessionStore};
use crate::tool::ToolRegistry;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

/// Spawns nested research agents. The `task` tool calls this; recursion depth
/// is bounded by `max_subagent_depth` enforced in the tool itself.
#[async_trait]
pub trait SubagentFactory: Send + Sync {
    /// Run a nested read-only agent with the given prompt and return its final
    /// text. `depth` is the nesting level of the new agent (1 for the first).
    /// `provider` optionally overrides the provider name from config (e.g. an
    /// Ollama endpoint added via `add-provider`) so cheap subagents can run on
    /// a different backend than the main loop; `model` likewise overrides the
    /// model within that provider. `cancel` is the parent's turn-level
    /// cancellation signal; when it fires the nested agent stops at its next
    /// checkpoint.
    async fn run_subagent(
        &self,
        prompt: &str,
        cwd: PathBuf,
        provider: Option<&str>,
        model: Option<&str>,
        depth: usize,
        cancel: Cancellable,
    ) -> Result<String, String>;
}

/// Concrete subagent runner used by the TUI and CLI. Rebuilds the provider from
/// the same `Config` (fresh client per nesting level), gives the nested agent a
/// read-only research registry, and keeps its session in a temp directory so it
/// never shows up in the user's session list.
pub struct SubagentRunner {
    pub config: Arc<Config>,
    pub registry: Arc<ToolRegistry>,
    pub provider_name: String,
    pub model: String,
    pub max_iterations: usize,
    pub asker: Option<Arc<dyn Asker>>,
    pub web_search_provider: Option<String>,
    pub web_search_api_key: Option<String>,
    pub permission: Arc<dyn PermissionChecker>,
}

impl SubagentRunner {
    /// Build the runner for the session's provider profile. `permission` is
    /// the same checker the parent agent uses, so nested agents inherit the
    /// user's approval policy instead of bypassing it.
    pub fn new(
        config: Arc<Config>,
        registry: Arc<ToolRegistry>,
        provider_name: impl Into<String>,
        model: impl Into<String>,
        asker: Option<Arc<dyn Asker>>,
        permission: Arc<dyn PermissionChecker>,
    ) -> Self {
        Self {
            max_iterations: 8,
            web_search_provider: config.tools.web_search.clone(),
            web_search_api_key: config.tools.resolved_web_search_api_key(),
            config,
            registry,
            provider_name: provider_name.into(),
            model: model.into(),
            asker,
            permission,
        }
    }

    fn child(&self) -> Arc<dyn SubagentFactory> {
        Arc::new(Self {
            config: self.config.clone(),
            registry: self.registry.clone(),
            provider_name: self.provider_name.clone(),
            model: self.model.clone(),
            max_iterations: self.max_iterations,
            asker: self.asker.clone(),
            web_search_provider: self.web_search_provider.clone(),
            web_search_api_key: self.web_search_api_key.clone(),
            permission: self.permission.clone(),
        })
    }
}

#[async_trait]
impl SubagentFactory for SubagentRunner {
    async fn run_subagent(
        &self,
        prompt: &str,
        cwd: PathBuf,
        provider: Option<&str>,
        model: Option<&str>,
        depth: usize,
        cancel: Cancellable,
    ) -> Result<String, String> {
        // Provider override first (a configured name, e.g. an Ollama endpoint
        // on the SBC), then the model override; both fall back to the
        // session's own values.
        let provider_name = provider.unwrap_or(&self.provider_name);
        let model = model.unwrap_or(&self.model).to_string();
        let provider = self
            .config
            .build_provider(Some(provider_name), Some(&model))
            .map_err(|e| format!("[Provider] failed to start subagent: {e}"))?;
        // Record the EFFECTIVE provider: the nested session's metadata must
        // reflect what actually ran (an override would otherwise be invisible
        // in the transcript).
        let session = Session::new(cwd, provider_name.to_string(), model.clone());
        let store = SessionStore::new(
            std::env::temp_dir()
                .join("firment-subagents")
                .join(&session.id),
        );
        let mut nested = Agent::new(
            Some(provider),
            self.registry.clone(),
            session,
            store.clone(),
            self.permission.clone(),
            Arc::new(NullSink),
            self.max_iterations,
        );
        nested.set_subagent_factory(Some(self.child()));
        nested.set_subagent_depth(depth);
        // Subagents cannot ask the user: the ask_user tool is for questions
        // only the human can answer, and a nested research agent must not
        // pop a question modal on the parent's screen.
        nested.set_asker(None);
        nested.set_web_search(
            self.web_search_provider.clone(),
            self.web_search_api_key.clone(),
        );
        nested.set_session_dir(Some(store.dir.join("work")));
        nested.set_elf_config(self.config.tools.elf.clone());
        // Propagate the parent turn's cancellation into the nested agent so
        // interrupting the parent also stops the subagent (and the processes
        // it spawned, via its own tool layer).
        let propagate = cancel.clone();
        let nested_cancel = nested.cancel_signal();
        tokio::spawn(async move {
            propagate.cancelled().await;
            nested_cancel.cancel();
        });
        let result = nested.run_turn(prompt).await;
        // The subagent session is transient bookkeeping: drop its whole
        // directory when done so long sessions do not accumulate temp junk.
        let _ = std::fs::remove_dir_all(&store.dir);
        result.map_err(|e| e.to_string())
    }
}

/// Event sink that drops everything; used for nested agents whose output is
/// returned as the task tool's result text.
pub struct NullSink;

#[async_trait]
impl EventSink for NullSink {
    async fn event(&self, _event: AgentEvent) {}
}
