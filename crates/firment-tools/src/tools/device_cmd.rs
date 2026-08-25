use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use firment_core::WorkbenchConfig;

pub struct DeviceCmd;

fn load_broker() -> Result<String, ToolError> {
    let path = firment_core::config::config_path();
    let cfg = firment_core::Config::load_or_create(&path)
        .map_err(|e| ToolError::new(format!("[DeviceCmd] config: {e}")))?;
    let broker = cfg.mqtt.broker.trim().to_string();
    if broker.is_empty() {
        return Err(ToolError::new(
            "[NoBroker] no [mqtt] broker configured in config.toml — the device data plane is off",
        ));
    }
    Ok(broker)
}

#[async_trait]
impl Tool for DeviceCmd {
    fn name(&self) -> &'static str {
        "device_cmd"
    }

    fn description(&self) -> &'static str {
        "Send a downlink command to a registered device node over MQTT \
         (firment/device/<node>/cmd). ONLY nodes declared in this project's \
         workbench.toml [devices] table are addressable; if the node has an \
         `allow` prefix list, the command must start with one of them. \
         The device's reply/state arrives as a retained frame on \
         firment/device/<node>/state — read it back with device_log. \
         Example: node=s3-node-1 command=\"rgb:#ff8800\"."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "node": {"type": "string", "description": "Target node name, e.g. s3-node-1"},
                "command": {"type": "string", "description": "Raw command payload, e.g. rgb:#ff0000 or rgb:off"}
            },
            "required": ["node", "command"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let node = args
            .get("node")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'node'"))?
            .to_string();
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing 'command'"))?
            .to_string();

        // Strict binding gate: only nodes the project declared in [devices]
        // are commandable. This keeps the agent off boards that belong to
        // other projects (or nobody).
        let cfg = WorkbenchConfig::load(&ctx.cwd)
            .map_err(|e| ToolError::new(format!("[DeviceCmd] {e}")))?;
        let Some(entry) = cfg.devices.get(&node) else {
            let known: Vec<String> = cfg.devices.keys().cloned().collect();
            return Err(ToolError::new(format!(
                "[NotRegistered] node '{node}' is not in this project's [devices] table \
                 (registered: {}). Register it first (workbench.toml) and confirm with the user.",
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            )));
        };
        if !entry.allow.is_empty()
            && !entry
                .allow
                .iter()
                .any(|prefix| command.starts_with(prefix.as_str()))
        {
            return Err(ToolError::new(format!(
                "[NotAllowed] command does not match any allowed prefix for '{node}' ({:?})",
                entry.allow
            )));
        }

        let broker = load_broker()?;
        let (host, port) = match broker.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
            None => (broker.clone(), 1883),
        };

        // Sync fire-and-forget publish: connect, queue, wait briefly for the
        // broker ack (QoS1), then drop the connection. rumqttc's sync client
        // must not run inside a tokio worker — hop to the blocking pool.
        let node2 = node.clone();
        let command2 = command.clone();
        let (sent_note, publish_result) =
            tokio::task::spawn_blocking(move || -> (String, Result<(), ToolError>) {
                // Unique per process AND per call: two rapid commands from
                // the same session must not kick each other off the broker.
                let client_id = format!(
                    "firment-cmd-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos())
                        .unwrap_or(0)
                );
                let opts = rumqttc::MqttOptions::new(&client_id, &host, port);
                let (client, mut conn) = rumqttc::Client::new(opts, 8);
                let topic = format!("firment/device/{node2}/cmd");
                if let Err(e) = client.publish(
                    &topic,
                    rumqttc::QoS::AtLeastOnce,
                    false,
                    command2.as_bytes(),
                ) {
                    return (
                        String::new(),
                        Err(ToolError::new(format!("[DeviceCmd] queue failed: {e}"))),
                    );
                }

                let deadline = Instant::now() + Duration::from_secs(5);
                let mut acked = false;
                while Instant::now() < deadline {
                    match conn.recv_timeout(Duration::from_millis(250)) {
                        Ok(Ok(rumqttc::Event::Incoming(rumqttc::Packet::PubAck(_)))) => {
                            acked = true;
                            break;
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            return (
                                String::new(),
                                Err(ToolError::new(format!(
                                    "[DeviceCmd] broker connection failed: {e}"
                                ))),
                            );
                        }
                        Err(_) => {} // recv timeout tick — loop until the deadline
                    }
                }
                (
                    format!(
                        " ({})",
                        if acked {
                            "broker acked"
                        } else {
                            "queued, no ack within 5s"
                        }
                    ),
                    Ok(()),
                )
            })
            .await
            .map_err(|e| ToolError::new(format!("[DeviceCmd] join: {e}")))?;
        publish_result?;
        let _ = ctx; // cwd already used for the registry lookup
        Ok(ToolOutput {
            text: format!("sent to {node}: {command:?}{sent_note}"),
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
            allowed_roots: Vec::new(),
            ..ToolContext::default()
        }
    }

    #[tokio::test]
    async fn unregistered_node_is_refused() {
        let dir = tempdir().unwrap();
        let err = DeviceCmd
            .run(
                json!({"node": "ghost-1", "command": "rgb:on"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("[NotRegistered]") && err.message.contains("none"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn allow_prefix_whitelist_is_enforced() {
        let dir = tempdir().unwrap();
        let mut cfg = WorkbenchConfig::load(dir.path()).unwrap();
        cfg.devices.insert(
            "s3-node-1".into(),
            firment_core::DeviceEntry {
                role: "main mcu".into(),
                note: String::new(),
                allow: vec!["rgb:".into()],
            },
        );
        cfg.save(dir.path()).unwrap();

        let err = DeviceCmd
            .run(
                json!({"node": "s3-node-1", "command": "reboot"}),
                &ctx(dir.path()),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[NotAllowed]"), "got: {}", err.message);

        // Allowed prefix passes the gate (fails later only if no broker is
        // configured on the test machine — assert the gate, not the wire).
        let result = DeviceCmd
            .run(
                json!({"node": "s3-node-1", "command": "rgb:on"}),
                &ctx(dir.path()),
            )
            .await;
        match result {
            Ok(_) => { /* broker present on this machine */ }
            Err(e) => assert!(e.message.contains("[NoBroker]"), "got: {}", e.message),
        }
    }
}
