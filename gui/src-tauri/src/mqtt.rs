//! MQTT data-plane link to the SBC broker (docs/sbc-agent.md §3,
//! gui-workbench.md §5). Subscribes `firment/#` and forwards device frames
//! and guard status to the frontend as global FrontendEvents.
//!
//! Deviation from the design note: this is a dedicated task emitting
//! FrontendEvents directly rather than a CollabBackend impl — the trait's
//! std-mpsc shape fits the multi-user review flow, not a live telemetry
//! feed, and the GUI needs zero coupling to it.

use std::sync::Arc;
use std::time::Duration;

use rumqttc::AsyncClient;

use crate::events::FrontendEvent;
use crate::state::Shared;

/// Spawn the MQTT link when `[mqtt] broker` is configured; no-op otherwise
/// (single-player without an SBC must not see errors).
pub fn spawn_if_configured(shared: Arc<Shared>) {
    let broker = {
        let cfg = shared.config.lock().unwrap();
        cfg.mqtt.broker.trim().to_string()
    };
    if broker.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move { run(shared, broker).await });
}

/// Split "host:port" (port optional, default 1883). IPv6 literals are not
/// supported on purpose — the SBC sits on a LAN with a v4 address.
fn parse_broker(broker: &str) -> (String, u16) {
    match broker.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
        None => (broker.to_string(), 1883),
    }
}

async fn run(shared: Arc<Shared>, broker: String) {
    let (host, port) = parse_broker(&broker);
    loop {
        let (client, mut eventloop) = {
            let opts = rumqttc::MqttOptions::new("firment-gui", &host, port);
            let (c, el) = AsyncClient::new(opts, 64);
            (c, el)
        };
        if let Err(e) = client
            .subscribe("firment/#", rumqttc::QoS::AtMostOnce)
            .await
        {
            emit(
                &shared,
                FrontendEvent::Info {
                    session_id: None,
                    message: format!("[mqtt] subscribe failed: {e}"),
                },
            );
        } else {
            emit(
                &shared,
                FrontendEvent::GuardStatus {
                    frame: format!("{{\"connected\":true,\"broker\":\"{broker}\"}}"),
                },
            );
        }

        loop {
            match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                    forward(&shared, &p.topic, &p.payload);
                }
                Ok(_) => {}
                Err(e) => {
                    emit(
                        &shared,
                        FrontendEvent::GuardStatus {
                            frame: format!("{{\"connected\":false,\"error\":\"{e}\"}}"),
                        },
                    );
                    // Backoff then rebuild the whole client: rumqttc event
                    // loops do not recover cleanly from transport errors.
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    break;
                }
            }
        }
    }
}

/// Route one publish to its frontend event by topic shape:
///   firment/device/<node>/<kind>  -> DeviceFrame
///   firment/guard/status          -> GuardStatus
fn forward(shared: &Arc<Shared>, topic: &str, payload: &[u8]) {
    let frame = String::from_utf8_lossy(payload).into_owned();
    let mut parts = topic.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("device"), Some(node), Some(kind), None) => {
            emit(
                shared,
                FrontendEvent::DeviceFrame {
                    node: node.to_string(),
                    kind: kind.to_string(),
                    frame,
                },
            );
        }
        (Some("guard"), Some("status"), None, None) => {
            emit(shared, FrontendEvent::GuardStatus { frame });
        }
        _ => {} // collab/presence etc. land in M4
    }
}

fn emit(shared: &Arc<Shared>, ev: FrontendEvent) {
    use tauri::Emitter as _;
    let _ = shared.app.emit("agent-event", ev);
}
