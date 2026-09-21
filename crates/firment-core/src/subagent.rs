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
/// One subagent call: what the factory needs that only the caller knows.
///
/// A struct rather than a parameter list. The list reached the point where positional
/// arguments were a hazard — `provider` and `model` are both `Option<&str>`, so swapping them
/// compiles — and the fields keep arriving one per lesson: `cancel` came from the turn,
/// `journal` from the same place so a child's edits belong to the transaction that spawned it.
pub struct SubagentCall<'a> {
    pub prompt: &'a str,
    pub cwd: PathBuf,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    /// Nesting level of the new agent (1 for the first).
    pub depth: usize,
    /// The parent's turn-level cancellation signal; when it fires the nested agent stops at
    /// its next checkpoint.
    pub cancel: Cancellable,
    /// The paths the child may write to, when the caller declares a scope (review §6 step 3).
    /// Resolved by the caller against the workspace, so a scope cannot point out of it.
    pub scope: Option<Vec<PathBuf>>,
    /// The **caller's** edit journal — the parent turn's transaction. A child's edits belong
    /// to the turn that spawned it: sharing the journal is what makes `/undo` after a batch
    /// roll back everything, and what stops a child's writes from landing in a journal nobody
    /// will ever read. See the concurrency design review (§5).
    pub journal: Arc<std::sync::Mutex<crate::journal::EditJournal>>,
}

#[async_trait]
pub trait SubagentFactory: Send + Sync {
    /// Run a nested read-only agent and return its final text.
    async fn run_subagent(&self, call: SubagentCall<'_>) -> Result<String, String>;
}

/// Concrete subagent runner used by the TUI and CLI. Rebuilds the provider from
/// the same `Config` (fresh client per nesting level), gives the nested agent a
/// read-only research registry, and keeps its session in a temp directory so it
/// never shows up in the user's session list.
///
/// `Clone` + public fields on purpose: the red team campaign clones the
/// research runner and swaps registry/permission/max_iterations for the
/// attacker profile, without re-plumbing provider construction.
#[derive(Clone)]
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
    /// Where the nested agent's events go. Research subagents keep the
    /// default [`NullSink`] (their output is the task tool's result text);
    /// the red team campaign passes the parent's sink so attack tool cards
    /// stream into the live UI.
    pub sink: Arc<dyn EventSink>,
    /// The slot pool shared by the whole subagent tree (plan §5, item 2).
    ///
    /// Passed down rather than created per level: `MAX_CONCURRENT_SUBAGENTS` is a bound on
    /// concurrent *provider streams*, and a bound that multiplies by depth is not a bound.
    pub subagent_slots: Arc<tokio::sync::Semaphore>,
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
            // A fresh pool by default; the assembly overrides it with the parent agent's so
            // that a whole tree shares one bound.
            subagent_slots: Arc::new(tokio::sync::Semaphore::new(
                crate::tool::MAX_CONCURRENT_SUBAGENTS,
            )),
            web_search_provider: config.tools.web_search.clone(),
            web_search_api_key: config.tools.resolved_web_search_api_key(),
            config,
            registry,
            provider_name: provider_name.into(),
            model: model.into(),
            asker,
            permission,
            sink: Arc::new(NullSink),
        }
    }

    /// A runner for a nested level, sharing this level's slot pool.
    ///
    /// Returns the concrete type so a test can check the sharing; the one call site coerces
    /// it to the trait object.
    fn child(&self) -> Arc<SubagentRunner> {
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
            sink: self.sink.clone(),
            subagent_slots: self.subagent_slots.clone(),
        })
    }
}

#[async_trait]
impl SubagentFactory for SubagentRunner {
    async fn run_subagent(&self, call: SubagentCall<'_>) -> Result<String, String> {
        let SubagentCall {
            prompt,
            cwd,
            provider,
            model,
            depth,
            cancel,
            scope,
            journal,
        } = call;
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
        // Captured before `Agent::new` takes ownership: the nested session's id
        // is what the SubagentStart/End pair names it with.
        let subagent_id = session.id.clone();
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
            self.sink.clone(),
            self.max_iterations,
        );
        nested.set_subagent_slots(self.subagent_slots.clone());
        // The parent turn's transaction, not a fresh one: see `SubagentFactory::run_subagent`.
        nested.set_edit_journal(journal);
        nested.set_write_scope(scope);
        nested.set_subagent_factory(Some(self.child() as Arc<dyn SubagentFactory>));
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
        nested.set_la_config(self.config.tools.la.clone());
        // Propagate the parent turn's cancellation into the nested agent so
        // interrupting the parent also stops the subagent (and the processes
        // it spawned, via its own tool layer). The handle is kept and aborted
        // on drop: an uncancelled parent would otherwise leave the parked
        // propagation task alive (one leaked tokio task per task-tool call).
        let propagate = cancel.clone();
        let nested_cancel = nested.cancel_signal();
        let propagator = tokio::spawn(async move {
            propagate.cancelled().await;
            nested_cancel.cancel();
        });
        struct AbortOnDrop(tokio::task::JoinHandle<()>);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _propagator_guard = AbortOnDrop(propagator);

        // Bracket the nested run on the PARENT's sink. The nested agent emits
        // through that same sink, so its tool calls would otherwise be
        // indistinguishable from the parent's -- and on the GUI, where each sink
        // stamps its own session id, they arrived as the parent's outright.
        // A UI that does not care can ignore both; one that does keeps a stack.
        self.sink
            .event(AgentEvent::SubagentStart {
                id: subagent_id.clone(),
                label: subagent_label(prompt),
                depth,
            })
            .await;
        let result = nested.run_turn(prompt).await;
        // Emitted on every path, including the error one: a UI stack that is
        // pushed but never popped would attribute the rest of the session to a
        // subagent that has already finished.
        self.sink
            .event(AgentEvent::SubagentEnd {
                id: subagent_id,
                depth,
            })
            .await;

        // The subagent session is transient bookkeeping: drop its whole
        // directory when done so long sessions do not accumulate temp junk.
        let _ = std::fs::remove_dir_all(&store.dir);
        result.map_err(|e| e.to_string())
    }
}

/// A short form of the prompt, for a UI that has one line to name the subagent
/// with.
///
/// The prompt *is* the question the subagent was asked, so it is the only
/// truthful label: an id names nothing a person recognises, and the tool name
/// would be the same word for every delegation. First non-empty line, trimmed,
/// capped at 60 chars on char boundaries so a CJK prompt is not cut mid-glyph.
fn subagent_label(prompt: &str) -> String {
    let first = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let trimmed = first.trim();
    let mut out: String = trimmed.chars().take(60).collect();
    if trimmed.chars().count() > 60 {
        out.push('…');
    }
    out
}

/// Event sink that drops everything; used for nested agents whose output is
/// returned as the task tool's result text.
pub struct NullSink;

#[async_trait]
impl EventSink for NullSink {
    async fn event(&self, _event: AgentEvent) {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_rollback_covers_every_file_the_batch_touched() {
        // Concurrency review §4.1 and §6 step 5 — the last link of the chain, and this test is
        // careful about which link it is.
        //
        // That a child *is handed* the parent's journal is proven elsewhere, one seam per test:
        // `task.rs::the_child_gets_the_parent_turns_journal` (the tool passes it to the runner)
        // and `agent.rs::a_nested_agent_writes_into_the_journal_it_was_handed` (the turn uses
        // the one it was given). This is the third seam: **given** one journal shared across a
        // parent and a child, a single rollback covers the child's files too — including a file
        // the child created, which the rollback has to remove rather than leave behind. Naming
        // this test after the sharing would be claiming a proof it does not contain.
        use crate::journal::EditJournal;
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        let parent_file = dir.path().join("parent.rs");
        let child_file = dir.path().join("child.rs");
        let created = dir.path().join("created-by-child.rs");
        std::fs::write(&parent_file, "parent: original\n").unwrap();
        std::fs::write(&child_file, "child: original\n").unwrap();

        let journal = Arc::new(Mutex::new(EditJournal::new(dir.path().join("undo"))));

        // The turn mutates its own file…
        journal.lock().unwrap().begin(&parent_file).unwrap();
        std::fs::write(&parent_file, "parent: changed\n").unwrap();

        // …and the child it spawns mutates its own, through the SAME journal — which is what
        // `SubagentCall::journal` carries into the nested agent's `set_edit_journal`.
        journal.lock().unwrap().begin(&child_file).unwrap();
        std::fs::write(&child_file, "child: changed\n").unwrap();
        // A file the child *creates* has nothing to restore, so the rollback has to remove it.
        journal.lock().unwrap().begin(&created).unwrap();
        std::fs::write(&created, "new\n").unwrap();

        // The batch fails: one rollback, every file the batch touched.
        let restored = journal.lock().unwrap().rollback().unwrap();
        assert_eq!(
            restored.len(),
            3,
            "all three belong to the batch: {restored:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&parent_file).unwrap(),
            "parent: original\n"
        );
        assert_eq!(
            std::fs::read_to_string(&child_file).unwrap(),
            "child: original\n",
            "the child's edit must roll back with the turn that spawned it"
        );
        assert!(
            !created.exists(),
            "a file the child created must be removed, not left behind half-undone"
        );
    }

    use super::*;
    use crate::config::Config;
    use crate::permission::AutoApprove;
    use crate::tool::ToolRegistry;

    #[test]
    fn a_child_runner_shares_the_slot_pool_instead_of_creating_its_own() {
        // A bound that multiplies by depth is not a bound. `child()` is what nested levels are
        // built from, so this is the assertion that keeps the pool shared: without it, a
        // depth-2 tree could run twenty children while the constant says four.
        let slots = Arc::new(tokio::sync::Semaphore::new(4));
        let runner = SubagentRunner {
            subagent_slots: slots.clone(),
            ..SubagentRunner::new(
                Arc::new(Config::default_config()),
                Arc::new(ToolRegistry::new()),
                "provider",
                "model",
                None,
                Arc::new(AutoApprove::everything()),
            )
        };

        let child = runner.child();
        assert!(
            Arc::ptr_eq(&child.subagent_slots, &slots),
            "the child must share the parent's pool, not create one of its own"
        );
        assert_eq!(slots.available_permits(), 4);

        // The default a directly-built runner gets is the shared constant, so a runner
        // constructed outside the assembly is bounded too.
        let standalone = SubagentRunner::new(
            Arc::new(Config::default_config()),
            Arc::new(ToolRegistry::new()),
            "provider",
            "model",
            None,
            Arc::new(AutoApprove::everything()),
        );
        assert_eq!(
            standalone.subagent_slots.available_permits(),
            crate::tool::MAX_CONCURRENT_SUBAGENTS
        );
    }
}
