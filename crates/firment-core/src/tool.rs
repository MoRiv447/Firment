use crate::ask::Asker;
use crate::cancel::Cancellable;
use crate::journal::EditJournal;
use crate::subagent::SubagentFactory;
use crate::{PermissionChecker, PermissionError, ToolSpec};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// How many subagents may run at once (plan §5, item 2).
///
/// A chosen default rather than a derived one: four is enough that a research task feels
/// parallel, and few enough to stay inside the rate limits of the providers people
/// configure. The number that would be *right* depends on the provider, which is why this
/// is one named constant instead of a rule scattered through the code — and why raising it
/// is a one-line change with an obvious reason.
pub const MAX_CONCURRENT_SUBAGENTS: usize = 4;

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub permission: Arc<dyn PermissionChecker>,
    /// Allow destructive shell commands (rm/del/git clean, etc.) without the
    /// hard guard. Interactive TUI enables this so the normal permission
    /// popup stays the decision point; one-shot `-y` keeps it disabled unless
    /// the user passes `--allow-dangerous`.
    pub allow_dangerous: bool,
    /// Per-turn edit journal: backups + rollback for write/edit tools.
    pub journal: Arc<Mutex<EditJournal>>,
    /// Slots for concurrent subagents (plan §5, item 2).
    ///
    /// The agent runs a turn's tool calls as one concurrent wave
    /// (`futures::future::join_all`), so several `task` calls in one turn already run in
    /// parallel — that is the read-only research parallelism the concurrency review
    /// recommends, and it needed no new machinery. What it *did* need is a bound: a wave of
    /// twenty `task` calls is twenty concurrent provider streams, which is a rate limit, a
    /// memory cost and a bill. A call waits for a slot rather than being refused, because a
    /// slow answer beats an error the model will retry.
    pub subagent_slots: Arc<tokio::sync::Semaphore>,
    /// Configured verification command from `[tools] verify_command`.
    pub verify_command: Option<String>,
    /// Extra roots (besides cwd) that file tools may access, e.g. the
    /// session's spill directory. Paths outside cwd + these roots are rejected.
    pub allowed_roots: Vec<PathBuf>,
    /// Symbol index backend override: `auto` / `ctags` / `regex`.
    pub symbols_backend: Option<String>,
    /// Configured build command from `[tools] build_command`.
    pub build_command: Option<String>,
    /// Default target chip for the flash tool from `[tools] default_chip`.
    pub default_chip: Option<String>,
    /// The paths this agent may **write** to, when a parent declared a scope for it
    /// (concurrency review §6, step 3). `None` means "the workspace, as usual".
    ///
    /// A write path must be inside both the workspace and this scope. Reads are not
    /// constrained: the hazard being designed against is two writers, and a research child
    /// that cannot read across the workspace cannot do its job.
    ///
    /// **The boundary of the guarantee, stated rather than implied**: it covers the file-edit
    /// tools (`write_file`, `edit_file`, `rename_symbol`). It does *not* cover `shell` — which
    /// is why the write-capable subagent registry excludes that tool — nor `build`/`verify`,
    /// which run the project's own commands, nor the hardware tools. A scope that silently
    /// did not cover a shell would be worse than no scope at all.
    pub write_scope: Option<Vec<PathBuf>>,
    /// Where a long tool reports phases (plan §6-10, item 7). `None` means nobody is listening —
    /// which is every direct tool run and every test, so a tool that reports has to work without
    /// a reporter as well as with one.
    pub progress: Option<crate::progress::ProgressReporter>,
    /// The active board profile's name, from `[board] active` (e.g. `nucleo-g431rb`).
    ///
    /// A tool that needs the board's *identity* rather than a path reads it here and resolves
    /// the profile with `board::find`. `periph_init` is the first: its `part` and its pinmap
    /// filter both fall back to the profile, which is what the profile's own `part` field says
    /// it is for.
    pub active_board: Option<String>,
    /// Serial port for the monitor tool from `[tools] monitor_port`.
    pub monitor_port: Option<String>,
    /// Baud rate for the monitor tool from `[tools] monitor_baud`.
    pub monitor_baud: u32,
    /// Nested-agent runner for the `task` tool; `None` in direct tool runs.
    pub subagent: Option<Arc<dyn SubagentFactory>>,
    /// Attacker-profile runner for the `redteam` campaign: same plumbing as
    /// `subagent` but with the hardware-capable registry, so the campaign
    /// can clone it and swap in the target-locked permission. `None` in
    /// direct tool runs and plan mode.
    pub attacker: Option<Arc<crate::subagent::SubagentRunner>>,
    /// Current subagent nesting depth (0 = main agent).
    pub subagent_depth: usize,
    /// Recursion limit for nested agents from `[tools] max_subagent_depth`.
    pub max_subagent_depth: usize,
    /// Interactive user front-end for the `ask_user` tool.
    pub asker: Option<Arc<dyn Asker>>,
    /// Web search provider name from `[tools] web_search`.
    pub web_search_provider: Option<String>,
    /// Resolved web search API key (inline config or env var).
    pub web_search_api_key: Option<String>,
    /// Per-session directory for tool bookkeeping (e.g. the todo list).
    pub session_dir: Option<PathBuf>,
    /// This session's change ledger, when the embedder runs the tool inside
    /// a session. Fault forensics correlates the captured scene against
    /// recent changes through it; None for session-less direct invocations.
    pub ledger_path: Option<PathBuf>,
    /// Turn-level cooperative cancellation signal. Long-running tools poll
    /// `cancelled()` and terminate child processes when it fires.
    pub cancel: Cancellable,
    /// Directory holding the desktop MQTT link's device-log files
    /// (device-log-<date>.jsonl). `None` falls back to the global config
    /// dir; tests inject a temp dir.
    pub device_log_dir: Option<PathBuf>,
    /// OpenAI-compatible endpoints from config.toml [providers], with keys
    /// already resolved through the one product-wide order (inline → auth.json →
    /// env, blank treated as unset). Backs the `models` discovery tool so the
    /// agent can see what each backend serves.
    pub providers: Vec<ProviderEndpoint>,
    /// Logic-analyzer defaults from config.toml [tools.la] (sigrok driver,
    /// samplerate, channel spec, sample cap). `None` = not configured; the
    /// `la` tool then requires every parameter explicitly.
    pub la: Option<crate::config::LaConfig>,
}

/// One callable model endpoint for the `models` discovery tool.
#[derive(Clone, Debug)]
pub struct ProviderEndpoint {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ToolContext {
    /// Convenience constructor with safe defaults; tests and direct tool runs
    /// override fields afterwards. Permission defaults to deny-all (fail
    /// closed) so a caller that forgets to set a permission checker cannot
    /// accidentally auto-approve mutating tools.
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self {
            cwd,
            permission: Arc::new(crate::AutoApprove::nothing()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(
                std::env::temp_dir().join("firment-journal"),
            ))),
            verify_command: None,
            allowed_roots: Vec::new(),
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            active_board: None,
            write_scope: None,
            progress: None,
            monitor_port: None,
            monitor_baud: 115_200,
            subagent: None,
            attacker: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: None,
            ledger_path: None,
            cancel: Cancellable::new(),
            device_log_dir: None,
            providers: Vec::new(),
            la: None,
            subagent_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SUBAGENTS)),
        }
    }
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::with_cwd(PathBuf::from("."))
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ToolError {
    pub message: String,
    /// Whether the call was rejected before execution (user denied approval
    /// or the permission check failed). A denied call mutates nothing, so the
    /// harness must not count it as a mutation or roll back on it.
    pub denied: bool,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolError {}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denied: false,
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denied: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    /// The name this tool is registered under, as an **owned** string.
    ///
    /// Defaults to `self.name()`, which is why all 54 built-in implementations are untouched by
    /// the registry's change: they inherit this, and a tool whose name is only known at run time
    /// — a plugin's — overrides this one method (plugin review §5 step 2).
    fn owned_name(&self) -> Arc<str> {
        Arc::from(self.name())
    }

    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;

    /// Return a human-readable reason if this invocation needs explicit approval.
    fn approval(&self, _args: &Value) -> Option<String> {
        None
    }

    /// Optional unified-diff preview appended to the approval prompt (used by
    /// write/edit tools so the user sees exactly what will change).
    fn preview(&self, _args: &Value, _ctx: &ToolContext) -> Option<String> {
        None
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}

pub struct ToolRegistry {
    /// Keyed by an **owned** name, not `&'static str` (plugin review §5 step 1).
    ///
    /// `Tool::name()` still returns `&'static str`, so every built-in is unchanged and the
    /// registry pays one small allocation per tool to copy the name in. What that buys is the
    /// thing a plugin mechanism needs: the map itself no longer demands that a tool's name was
    /// known at compile time, which was the constraint the review found first.
    tools: HashMap<Arc<str>, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a **built-in** tool. A duplicate name replaces.
    ///
    /// That is right while the built-in set is assembled in one place, and wrong for anything
    /// whose name comes from a manifest — see [`ToolRegistry::register_plugin`].
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.owned_name(), tool);
    }

    /// Add every tool of `other`, as built-ins.
    ///
    /// A shallow copy of `Arc`s, which is what makes composing registries cheap enough to do
    /// rather than to cache. The name rule is `register`'s (last wins), because that is what
    /// assembling the built-in set is.
    pub fn extend_from(&mut self, other: &ToolRegistry) {
        for (name, tool) in &other.tools {
            self.tools.insert(name.clone(), tool.clone());
        }
    }

    /// Register a **plugin's** tool, refusing a name that is already taken.
    ///
    /// The difference from [`ToolRegistry::register`] is the point of having two methods. While
    /// every tool is compiled in, a duplicate name is a mistake inside one file. The moment a
    /// name can come from a manifest, a duplicate is a plugin claiming to be `write_file`, and
    /// "the last one wins" would mean a plugin silently replacing the tool the user believes
    /// they are calling.
    pub fn register_plugin(&mut self, tool: Arc<dyn Tool>) -> Result<(), String> {
        let name = tool.owned_name();
        if self.tools.contains_key(name.as_ref()) {
            return Err(format!(
                "plugin tool {name:?} would shadow a tool that already exists — plugin names \
                 must not collide with built-ins or with each other"
            ));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Every registered name, **owned**.
    ///
    /// It used to return `&'static str`, which was free and also the reason a runtime-named
    /// tool could not be described. The caller that wants borrowed strings maps them.
    pub fn names(&self) -> Vec<Arc<str>> {
        self.tools.keys().cloned().collect()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self
            .tools
            .values()
            .map(|t| ToolSpec {
                name: t.owned_name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub async fn run(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::new(format!("unknown tool: {name}")))?;
        crate::schema::validate_args(&tool.owned_name(), &tool.input_schema(), &args)
            .map_err(ToolError::new)?;
        let mut reason = tool.approval(&args);
        if let Some(preview) = tool.preview(&args, ctx)
            && let Some(reason) = reason.as_mut()
        {
            reason.push_str(&format!("\n{preview}"));
        }
        if let Some(reason) = reason {
            match ctx
                .permission
                .confirm(&tool.owned_name(), &args, &reason)
                .await
            {
                Ok(()) => {}
                Err(PermissionError::Denied(message)) => {
                    return Err(ToolError::denied(format!("Permission denied: {message}")));
                }
                Err(PermissionError::Io(e)) => {
                    return Err(ToolError::denied(format!("Permission check failed: {e}")));
                }
            }
        }
        tool.run(args, ctx).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// A tool whose name is fixed at compile time — which is all a `Tool` can be today
    /// (`name()` returns `&'static str`). The registry no longer needs that, and the next step
    /// is letting a tool say otherwise; see the plugin review's §5.
    struct Stub(&'static str);

    #[async_trait]
    impl Tool for Stub {
        fn name(&self) -> &'static str {
            self.0
        }

        fn description(&self) -> &'static str {
            "a stub"
        }

        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                text: String::new(),
            })
        }
    }

    /// A tool whose name is built at run time — which is exactly what the registry could not
    /// hold before the key became owned, and what a plugin's tool will look like.
    struct Named(String);

    #[async_trait]
    impl Tool for Named {
        fn name(&self) -> &'static str {
            // Deliberately not the run-time name: a plugin-provided tool has no `'static` one
            // to give, and `owned_name` is the door it uses instead.
            "plugin-tool"
        }

        fn owned_name(&self) -> Arc<str> {
            Arc::from(self.0.as_str())
        }

        fn description(&self) -> &'static str {
            "a runtime-named stub"
        }

        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                text: String::new(),
            })
        }
    }

    /// Like [`Named`], but every call needs approval — so the permission layer's key can be
    /// observed rather than assumed.
    struct GuardedNamed(String);

    #[async_trait]
    impl Tool for GuardedNamed {
        fn name(&self) -> &'static str {
            "plugin-tool"
        }

        fn owned_name(&self) -> Arc<str> {
            Arc::from(self.0.as_str())
        }

        fn description(&self) -> &'static str {
            "a guarded runtime-named stub"
        }

        fn input_schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        fn approval(&self, _args: &Value) -> Option<String> {
            Some("writes outside the session".to_string())
        }

        async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                text: String::new(),
            })
        }
    }

    #[test]
    fn owned_name_defaults_to_the_static_one_and_lets_a_runtime_name_through() {
        // Two halves of one promise: built-ins keep working without an edit (the default), and
        // a name that did not exist at compile time can be registered (the override).
        let dir = tempfile::tempdir().unwrap();
        let _ = dir; // the stub never runs

        assert_eq!(&*Stub("alpha").owned_name(), "alpha");

        let mut registry = ToolRegistry::new();
        let runtime_name = format!("sensor-{}", 7);
        registry.register(Arc::new(Named(runtime_name.clone())));
        assert!(
            registry.get(&runtime_name).is_some(),
            "a run-time name must be registrable"
        );
        assert!(registry.get("plugin-tool").is_none());
    }

    #[test]
    fn a_plugin_tool_may_not_shadow_an_existing_one() {
        // The premise that makes the plugin step safe: with names coming from a manifest, "last one
        // wins" would let a plugin replace `write_file` and the user would never see it.
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(Stub("write_file")));

        let err = registry
            .register_plugin(Arc::new(Named("write_file".to_string())))
            .unwrap_err();
        assert!(err.contains("write_file"), "{err}");
        assert!(err.contains("shadow"), "{err}");
        // And the original is still the one that answers.
        assert_eq!(registry.get("write_file").unwrap().description(), "a stub");

        // A fresh name is fine.
        registry
            .register_plugin(Arc::new(Named("sensor".to_string())))
            .expect("a new name is allowed");
        assert!(registry.get("sensor").is_some());
    }

    #[test]
    fn the_names_it_advertises_are_the_names_it_resolves() {
        // The invariant the owned key has to keep as the registry stops being compile-time
        // closed: what the model is shown (`specs`) and what a call resolves through (`get`)
        // are the same set. A plugin mechanism starts inserting names that were never
        // `'static`, and a registration that only one of the two could see is exactly the bug
        // this would catch.
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(Stub("alpha")));
        registry.register(Arc::new(Stub("beta")));

        let names = registry.names();
        assert_eq!(names.len(), 2);
        for name in &names {
            assert!(
                registry.get(name).is_some(),
                "{name} is advertised but not resolvable"
            );
        }
        let specs = registry.specs();
        assert_eq!(specs.len(), names.len());
        for spec in &specs {
            assert!(
                names.iter().any(|n| n.as_ref() == spec.name),
                "{} is in specs() but not names()",
                spec.name
            );
        }

        // Re-registering a name replaces the tool: last one wins. That is the behaviour the
        // plugin step has to make a decision about (a plugin that shadows `write_file` would be
        // a security hole, so a built-in collision must be refused) — this test pins today's
        // behaviour so changing it is a deliberate act.
        registry.register(Arc::new(Stub("alpha")));
        assert_eq!(registry.names().len(), 2);
    }

    #[test]
    fn a_runtime_named_tool_is_advertised_under_the_name_that_resolves() {
        // The plugin case, and the reason `specs()` cannot use `name()`: a plugin tool has no
        // compile-time name, so it returns a placeholder there and the real one through
        // `owned_name()`. Advertising the placeholder showed the model a tool that then failed
        // `get()` as "unknown tool" — no plugin was ever callable — and two plugins produced two
        // identical entries. The stub above cannot express this, because its two names are equal.
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(Stub("build")));
        registry
            .register_plugin(Arc::new(Named("sensor".to_string())))
            .unwrap();
        registry
            .register_plugin(Arc::new(Named("camera".to_string())))
            .unwrap();

        let specs = registry.specs();
        let advertised: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(advertised, ["build", "camera", "sensor"]);
        assert!(
            !advertised.contains(&"plugin-tool"),
            "the placeholder reached the model: {advertised:?}"
        );
        for name in ["sensor", "camera"] {
            assert!(
                registry.get(name).is_some(),
                "{name} advertised but not resolvable"
            );
        }
    }

    #[tokio::test]
    async fn approval_is_decided_per_plugin_rather_than_for_all_of_them() {
        // The same key decides `auto_approve` matching and the `[a] always allow` answer. Read
        // off the placeholder, one approval on any plugin approved every plugin for the session —
        // including one declared `fs.write` — and `auto_approve = ["sensor"]` never matched.
        let mut registry = ToolRegistry::new();
        registry
            .register_plugin(Arc::new(GuardedNamed("sensor".to_string())))
            .unwrap();
        registry
            .register_plugin(Arc::new(GuardedNamed("camera".to_string())))
            .unwrap();

        let ctx = ToolContext {
            permission: Arc::new(crate::AutoApprove::new(false, ["sensor".to_string()])),
            ..ToolContext::default()
        };

        registry
            .run("sensor", serde_json::json!({}), &ctx)
            .await
            .expect("the named plugin is allowed by name");
        let err = registry
            .run("camera", serde_json::json!({}), &ctx)
            .await
            .expect_err("a different plugin must not inherit that allowance");
        assert!(
            err.message.contains("camera"),
            "the refusal must name what it refused, got: {}",
            err.message
        );
    }
}
