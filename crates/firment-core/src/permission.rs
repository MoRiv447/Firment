use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("{0}")]
    Denied(String),
    #[error("io error while asking for permission: {0}")]
    Io(#[from] std::io::Error),
}

impl PermissionError {
    pub fn denied(message: impl Into<String>) -> Self {
        Self::Denied(message.into())
    }
}

#[async_trait]
pub trait PermissionChecker: Send + Sync {
    async fn confirm(&self, tool: &str, args: &Value, reason: &str) -> Result<(), PermissionError>;
}

/// Permission checker that approves a fixed set of tools (or everything).
pub struct AutoApprove {
    pub allow_all: bool,
    pub allow_names: HashSet<String>,
}

impl AutoApprove {
    pub fn new(allow_all: bool, allow_names: impl IntoIterator<Item = String>) -> Self {
        Self {
            allow_all,
            allow_names: allow_names.into_iter().collect(),
        }
    }

    pub fn everything() -> Self {
        Self {
            allow_all: true,
            allow_names: HashSet::new(),
        }
    }

    pub fn nothing() -> Self {
        Self {
            allow_all: false,
            allow_names: HashSet::new(),
        }
    }
}

#[async_trait]
impl PermissionChecker for AutoApprove {
    async fn confirm(
        &self,
        tool: &str,
        _args: &Value,
        _reason: &str,
    ) -> Result<(), PermissionError> {
        if self.allow_all || self.allow_names.contains(tool) {
            Ok(())
        } else {
            Err(PermissionError::denied(format!(
                "tool '{tool}' requires approval and no auto-approve rule matches"
            )))
        }
    }
}

/// Permission wrapper that hard-rejects mutating tools in PLAN mode even if
/// they somehow reach the registry (the read-only registry is the first line
/// of defence; this is the second).
pub struct PlanModePermission {
    inner: Arc<dyn PermissionChecker>,
}

impl PlanModePermission {
    pub fn new(inner: Arc<dyn PermissionChecker>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl PermissionChecker for PlanModePermission {
    async fn confirm(&self, tool: &str, args: &Value, reason: &str) -> Result<(), PermissionError> {
        if matches!(tool, "write_file" | "edit_file" | "shell") {
            return Err(PermissionError::denied(
                "plan mode: read-only mode, write_file/edit_file/shell are disabled",
            ));
        }
        self.inner.confirm(tool, args, reason).await
    }
}
