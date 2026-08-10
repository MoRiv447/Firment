use crate::Asker;
use crate::agent::{Agent, AgentEvent, EventSink};
use crate::config::Config;
use crate::permission::AutoApprove;
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
    async fn run_subagent(
        &self,
        prompt: &str,
        cwd: PathBuf,
        model: Option<&str>,
        depth: usize,
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
}

impl SubagentRunner {
    /// Build the runner for the session's provider profile.
    pub fn new(
        config: Arc<Config>,
        registry: Arc<ToolRegistry>,
        provider_name: impl Into<String>,
        model: impl Into<String>,
        asker: Option<Arc<dyn Asker>>,
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
        })
    }
}

#[async_trait]
impl SubagentFactory for SubagentRunner {
    async fn run_subagent(
        &self,
        prompt: &str,
        cwd: PathBuf,
        model: Option<&str>,
        depth: usize,
    ) -> Result<String, String> {
        let model = model.unwrap_or(&self.model).to_string();
        let provider = self
            .config
            .build_provider(Some(&self.provider_name), Some(&model))
            .map_err(|e| format!("[Provider] failed to start subagent: {e}"))?;
        let session = Session::new(cwd, self.provider_name.clone(), model.clone());
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
            Arc::new(AutoApprove::everything()),
            Arc::new(NullSink),
            self.max_iterations,
        );
        nested.set_subagent_factory(Some(self.child()));
        nested.set_subagent_depth(depth);
        nested.set_asker(self.asker.clone());
        nested.set_web_search(
            self.web_search_provider.clone(),
            self.web_search_api_key.clone(),
        );
        nested.set_session_dir(Some(store.dir.join("work")));
        nested.set_elf_glob(self.config.tools.elf.clone());
        nested.run_turn(prompt).await.map_err(|e| e.to_string())
    }
}

/// Event sink that drops everything; used for nested agents whose output is
/// returned as the task tool's result text.
pub struct NullSink;

#[async_trait]
impl EventSink for NullSink {
    async fn event(&self, _event: AgentEvent) {}
}
