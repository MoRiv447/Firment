use crate::journal::EditJournal;
use crate::{PermissionChecker, PermissionError, ToolSpec};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub permission: Arc<dyn PermissionChecker>,
    /// Allow destructive shell commands (rm/del/git clean 等) without the
    /// hard guard. Interactive TUI enables this so the normal permission
    /// popup stays the decision point; one-shot `-y` keeps it disabled unless
    /// the user passes `--allow-dangerous`.
    pub allow_dangerous: bool,
    /// Per-turn edit journal: backups + rollback for write/edit tools.
    pub journal: Arc<Mutex<EditJournal>>,
    /// Configured verification command from `[tools] verify_command`.
    pub verify_command: Option<String>,
    /// Extra roots (besides cwd) that file tools may access, e.g. the
    /// session's spill directory. Paths outside cwd + these roots are rejected.
    pub allowed_roots: Vec<PathBuf>,
    /// Symbol index backend override: `auto` / `ctags` / `regex`.
    pub symbols_backend: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ToolError {
    pub message: String,
}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
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
    tools: HashMap<&'static str, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self
            .tools
            .values()
            .map(|t| ToolSpec {
                name: t.name().to_string(),
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
        crate::schema::validate_args(tool.name(), &tool.input_schema(), &args)
            .map_err(ToolError::new)?;
        let mut reason = tool.approval(&args);
        if let Some(preview) = tool.preview(&args, ctx)
            && let Some(reason) = reason.as_mut()
        {
            reason.push_str(&format!("\n{preview}"));
        }
        if let Some(reason) = reason {
            match ctx.permission.confirm(tool.name(), &args, &reason).await {
                Ok(()) => {}
                Err(PermissionError::Denied(message)) => {
                    return Ok(ToolOutput {
                        text: format!("Permission denied: {message}"),
                    });
                }
                Err(PermissionError::Io(e)) => {
                    return Ok(ToolOutput {
                        text: format!("Permission check failed: {e}"),
                    });
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
