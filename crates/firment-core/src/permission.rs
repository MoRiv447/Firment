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

/// Permission wrapper for the red team attacker: serial/MQTT targets are
/// locked to the interfaces the approved suite declared. An attacker agent
/// that wanders onto another port (or "auto") is attacking something nobody
/// approved — deny before the inner checker ever sees it. Probe-side tools
/// (`debug`, `la`) have no port argument: the probe IS the target, and their
/// own approval prompts still reach the user through `inner`.
pub struct TargetLockPermission {
    inner: Arc<dyn PermissionChecker>,
    /// Allowed `port` values for monitor, and allowed `node` values for
    /// device_cmd. Exact match only — "auto" is a bypass and is denied.
    pub ports: Vec<String>,
}

impl TargetLockPermission {
    pub fn new(inner: Arc<dyn PermissionChecker>, ports: Vec<String>) -> Self {
        Self { inner, ports }
    }
}

#[async_trait]
impl PermissionChecker for TargetLockPermission {
    async fn confirm(&self, tool: &str, args: &Value, reason: &str) -> Result<(), PermissionError> {
        // The attacker must not freeze the target on a breakpoint and then
        // "discover" a hang it manufactured: memory writes are off the
        // table, observation is not.
        if tool == "debug" && args.get("action").and_then(|v| v.as_str()) == Some("write") {
            return Err(PermissionError::denied(
                "red team target lock: debug action=write is not allowed for the campaign — \
                 the attacker observes the firmware's behaviour, it does not poke the target \
                 into a fake hang",
            ));
        }
        let (key, what) = match tool {
            "monitor" => ("port", "serial port"),
            "device_cmd" => ("node", "device node"),
            _ => return self.inner.confirm(tool, args, reason).await,
        };
        match args.get(key).and_then(|v| v.as_str()) {
            // Case-insensitive: Windows COM names are ("COM3" == "com3"),
            // and a needless exact-match denial just burns a campaign turn.
            Some(value) if self.ports.iter().any(|p| p.eq_ignore_ascii_case(value)) => {}
            Some(value) => {
                return Err(PermissionError::denied(format!(
                    "red team target lock: {what} '{value}' is not one of the suite's declared \
                     interfaces ({:?}) — the attacker may only touch what the approved suite \
                     named",
                    self.ports
                )));
            }
            None => {
                return Err(PermissionError::denied(format!(
                    "red team target lock: {tool} without an explicit {key} (e.g. auto-detect) \
                     could reach an unapproved target — pass {key} explicitly, one of {:?}",
                    self.ports
                )));
            }
        }
        self.inner.confirm(tool, args, reason).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Recorder {
        seen: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PermissionChecker for Recorder {
        async fn confirm(
            &self,
            tool: &str,
            _args: &Value,
            _reason: &str,
        ) -> Result<(), PermissionError> {
            self.seen.lock().unwrap().push(tool.to_string());
            Ok(())
        }
    }

    fn lock(ports: &[&str]) -> (TargetLockPermission, Arc<Recorder>) {
        let rec = Arc::new(Recorder {
            seen: std::sync::Mutex::new(Vec::new()),
        });
        (
            TargetLockPermission::new(rec.clone(), ports.iter().map(|s| s.to_string()).collect()),
            rec,
        )
    }

    #[tokio::test]
    async fn locked_port_passes_through_to_inner() {
        let (p, rec) = lock(&["COM3"]);
        p.confirm("monitor", &json!({"port": "COM3"}), "r")
            .await
            .unwrap();
        assert_eq!(*rec.seen.lock().unwrap(), vec!["monitor"]);
    }

    #[tokio::test]
    async fn unlocked_port_is_denied_before_inner_sees_it() {
        let (p, rec) = lock(&["COM3"]);
        let err = p
            .confirm("monitor", &json!({"port": "COM9"}), "r")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("target lock"), "got {err}");
        assert!(
            rec.seen.lock().unwrap().is_empty(),
            "inner must never see it"
        );
    }

    #[tokio::test]
    async fn auto_detect_is_a_bypass_and_denied() {
        let (p, _) = lock(&["COM3"]);
        assert!(
            p.confirm("monitor", &json!({"port": "auto"}), "r")
                .await
                .is_err()
        );
        assert!(p.confirm("monitor", &json!({}), "r").await.is_err());
    }

    #[tokio::test]
    async fn device_cmd_locks_on_node() {
        let (p, _) = lock(&["s3-node-1"]);
        p.confirm(
            "device_cmd",
            &json!({"node": "s3-node-1", "command": "led on"}),
            "r",
        )
        .await
        .unwrap();
        assert!(
            p.confirm("device_cmd", &json!({"node": "other", "command": "x"}), "r")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn non_target_tools_pass_through() {
        let (p, rec) = lock(&["COM3"]);
        p.confirm("debug", &json!({"action": "halt"}), "r")
            .await
            .unwrap();
        p.confirm("read_file", &json!({"path": "x"}), "r")
            .await
            .unwrap();
        assert_eq!(rec.seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn port_match_is_case_insensitive() {
        // Windows COM names are case-insensitive; exact-match denial would
        // just waste campaign turns.
        let (p, _) = lock(&["COM3"]);
        p.confirm("monitor", &json!({"port": "com3"}), "r")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn debug_memory_write_is_denied_to_the_attacker() {
        // The attacker observes; it must not poke the target into a hang it
        // then "discovers".
        let (p, rec) = lock(&["COM3"]);
        let err = p
            .confirm(
                "debug",
                &json!({"action": "write", "addr": "0x20000000"}),
                "r",
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("target lock"), "got {err}");
        assert!(rec.seen.lock().unwrap().is_empty());
        // Observation actions still pass.
        p.confirm("debug", &json!({"action": "analyze"}), "r")
            .await
            .unwrap();
    }
}
